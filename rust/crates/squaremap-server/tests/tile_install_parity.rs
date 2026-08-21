use squaremap_render::fixture::{FixtureCorpus, PreparedCase, render_case};
use squaremap_render::{
    MemoryTileStore, PngOptions, RenderSettings, TILE_SIZE, TileStore, decode_rgba_png,
};
use squaremap_server::scheduler::{
    InstallRequest, PrefixedTileStore, RenderTileInstaller, SnapshotBundle, TileInstaller,
    WorldRenderConfig,
};
use squaremap_state::{ChunkCoordinate, WorldId};
use std::path::Path;
use std::sync::Arc;

const CORPUS_ROOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../testdata/bridge/v2/render"
);

fn corpus() -> FixtureCorpus {
    FixtureCorpus::load(Path::new(CORPUS_ROOT)).expect("valid v2 render corpus")
}

fn case_by_id<'a>(corpus: &'a FixtureCorpus, id: &str) -> &'a PreparedCase {
    corpus
        .cases
        .iter()
        .find(|case| case.row.id == id)
        .unwrap_or_else(|| panic!("missing fixture {id}"))
}

fn world_id() -> WorldId {
    WorldId::new("minecraft", "overworld", 1)
}

fn world_config(corpus: &FixtureCorpus, case: &PreparedCase) -> WorldRenderConfig {
    let row = &case.row;
    WorldRenderConfig {
        settings: RenderSettings {
            iterate_up: row.iterate_up,
            map_max_height: row.max_height,
            biomes_enabled: row.biome_enabled,
            biome_blend: row.biome_blend,
            glass_clear: row.glass_clear,
            water_clear: row.water_clear,
            water_checkerboard: row.water_checkerboard,
            lava_checkerboard: row.lava_checkerboard,
        },
        invisible_ids: [row.invisible_id]
            .into_iter()
            .filter(|id| *id != 0)
            .collect::<Vec<_>>()
            .into(),
        iterate_up_base_ids: [row.iterate_up_base_id]
            .into_iter()
            .filter(|id| *id != 0)
            .collect::<Vec<_>>()
            .into(),
        biome_zoom_seed: corpus.manifest.biome_zoom_seed,
        max_zoom: 1,
        png_options: PngOptions::default(),
        tile_prefix: "world".into(),
    }
}

fn snapshot_bundle(case: &PreparedCase) -> SnapshotBundle {
    SnapshotBundle {
        north: case.north.clone(),
        center: case.center.clone(),
        south: case.south.clone(),
        biome_sources: case
            .biome_source
            .as_ref()
            .map(|source| source.snapshots())
            .unwrap_or_default(),
        grass_resolutions: case
            .row
            .grass_samples
            .iter()
            .map(|sample| {
                (
                    sample.block_x,
                    sample.block_y,
                    sample.block_z,
                    sample.biome_id,
                    sample.resolved_grass_argb as u32,
                )
            })
            .collect(),
    }
}

fn argb_rgba(argb: u32) -> [u8; 4] {
    [
        (argb >> 16) as u8,
        (argb >> 8) as u8,
        argb as u8,
        (argb >> 24) as u8,
    ]
}

fn pixel(rgba: &[u8], x: usize, z: usize) -> [u8; 4] {
    let offset = (z * TILE_SIZE + x) * 4;
    rgba[offset..offset + 4]
        .try_into()
        .expect("four-byte pixel")
}

async fn decoded(store: &MemoryTileStore, relative: &str) -> Vec<u8> {
    let bytes = store
        .read(Path::new(relative))
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("missing tile {relative}"));
    decode_rgba_png(&bytes).unwrap()
}

async fn install_chunk(
    installer: &RenderTileInstaller,
    world: &WorldId,
    coordinate: ChunkCoordinate,
    case: &PreparedCase,
) {
    let staged = installer
        .stage(InstallRequest {
            world: world.clone(),
            coordinate,
            revision: 1,
            snapshots: snapshot_bundle(case),
        })
        .await
        .unwrap_or_else(|error| panic!("stage {}: {error}", case.row.id));
    staged
        .publish()
        .await
        .unwrap_or_else(|error| panic!("publish {}: {error}", case.row.id));
}

fn assert_chunk_rect(image: &[u8], origin_x: usize, origin_z: usize, case: &PreparedCase) {
    let rendered = render_case(case).unwrap_or_else(|error| panic!("{}: {error:?}", case.row.id));
    for x in 0..16 {
        for z in 0..16 {
            assert_eq!(
                pixel(image, origin_x + x, origin_z + z),
                argb_rgba(rendered.pixel(x, z)),
                "{} at region-local ({}, {})",
                case.row.id,
                origin_x + x,
                origin_z + z
            );
        }
    }
}

#[test]
fn tile_prefix_must_be_relative() {
    let store = Arc::new(MemoryTileStore::new());
    assert_eq!(
        PrefixedTileStore::new("/abs", store.clone()).err(),
        Some("tile prefix must be relative".into())
    );
    PrefixedTileStore::new("world", store).expect("relative tile prefix");
}

#[tokio::test]
async fn sequential_chunk_installs_merge_into_one_region_tile() {
    let corpus = corpus();
    let first = case_by_id(&corpus, "flat-solid");
    let second = case_by_id(&corpus, "iterate-down");
    let first_color = argb_rgba(render_case(first).unwrap().pixel(0, 0));
    let second_color = argb_rgba(render_case(second).unwrap().pixel(0, 0));
    assert_ne!(first_color, second_color);
    assert_ne!(first_color, [0, 0, 0, 0]);
    assert_ne!(second_color, [0, 0, 0, 0]);

    let store = Arc::new(MemoryTileStore::new());
    let installer = RenderTileInstaller::new(store.clone());
    let world = world_id();
    let config = world_config(&corpus, first);
    assert!(!config.tile_prefix.is_absolute());
    installer
        .configure_world(world.clone(), config)
        .expect("configure production installer");

    install_chunk(&installer, &world, ChunkCoordinate { x: 0, z: 0 }, first).await;
    install_chunk(&installer, &world, ChunkCoordinate { x: 1, z: 0 }, second).await;

    let native = decoded(&store, "world/1/0_0.png").await;
    assert_chunk_rect(&native, 0, 0, first);
    assert_chunk_rect(&native, 16, 0, second);
    assert_eq!(pixel(&native, 32, 0), [0, 0, 0, 0]);
    assert_eq!(pixel(&native, 0, 16), [0, 0, 0, 0]);

    let parent = decoded(&store, "world/0/0_0.png").await;
    assert_eq!(pixel(&parent, 0, 0), first_color);
    assert_eq!(pixel(&parent, 8, 0), second_color);
}
