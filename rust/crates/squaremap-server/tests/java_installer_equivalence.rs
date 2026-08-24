//! Proves: `RenderTileInstaller` maps chunk coordinates with
//! `floorMod(chunk, 32) * 16` and publishes a pyramid whose decoded RGBA
//! matches Java `Image.setPixel` + `save()` for the same v2 render-fixture pixels.
//!
//! Does not prove: live Minecraft snapshots, HTTP, scheduler jobs, or Paper.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use squaremap_render::fixture::{FixtureCorpus, PreparedCase};
use squaremap_render::{
    MemoryTileStore, PngOptions, RenderSettings, TILE_RGBA_BYTES, TILE_SIZE, TileStore,
    decode_rgba_png,
};
use squaremap_server::scheduler::{
    InstallRequest, RenderTileInstaller, SnapshotBundle, TileInstaller, WorldRenderConfig,
};
use squaremap_state::{ChunkCoordinate, WorldId};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CORPUS_ROOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../testdata/bridge/v2/render"
);
const INSTALLER_MANIFEST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../testdata/bridge/v2/installer/manifest.json"
);
const JAVA_HASHES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../testdata/bridge/v2/installer/java-oracle-hashes.json"
);

#[derive(Deserialize)]
struct InstallerManifest {
    schema_version: u32,
    max_zoom: u8,
    cases: Vec<InstallerCase>,
}

#[derive(Deserialize)]
struct InstallerCase {
    id: String,
    installs: Vec<Install>,
}

#[derive(Deserialize)]
struct Install {
    fixture: String,
    chunk: [i32; 2],
}

#[derive(Deserialize)]
struct JavaOracleHashes {
    schema_version: u32,
    manifest_hash: String,
    max_zoom: u8,
    cases: BTreeMap<String, BTreeMap<String, String>>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn load_manifest() -> (InstallerManifest, Vec<u8>) {
    let path = Path::new(INSTALLER_MANIFEST);
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let manifest: InstallerManifest = serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
    (manifest, bytes)
}

fn load_hashes() -> JavaOracleHashes {
    let path = Path::new(JAVA_HASHES);
    assert!(
        path.is_file(),
        "committed Java installer hashes missing: {}",
        path.display()
    );
    serde_json::from_slice(&fs::read(path).unwrap())
        .unwrap_or_else(|error| panic!("invalid {}: {error}", path.display()))
}

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

fn world_config(corpus: &FixtureCorpus, case: &PreparedCase, max_zoom: u8) -> WorldRenderConfig {
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
        max_zoom,
        png_options: PngOptions {
            compression: false,
        },
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

fn first_pixel_mismatch(rust_rgba: &[u8], relative: &str) -> String {
    format!(" {relative} first-pixel scan length={}", rust_rgba.len())
}

fn pixel(rgba: &[u8], x: usize, z: usize) -> [u8; 4] {
    let offset = (z * TILE_SIZE + x) * 4;
    rgba[offset..offset + 4]
        .try_into()
        .expect("four-byte pixel")
}

async fn run_case(id: &str) -> (BTreeMap<String, String>, Arc<MemoryTileStore>) {
    let (manifest, _) = load_manifest();
    let case = manifest
        .cases
        .iter()
        .find(|case| case.id == id)
        .unwrap_or_else(|| panic!("unknown installer case {id}"));
    let corpus = corpus();
    let store = Arc::new(MemoryTileStore::new());
    let installer = RenderTileInstaller::new(store.clone());
    let world = world_id();
    let first = case_by_id(&corpus, &case.installs[0].fixture);
    installer
        .configure_world(world.clone(), world_config(&corpus, first, manifest.max_zoom))
        .expect("configure production installer");

    for install in &case.installs {
        let fixture = case_by_id(&corpus, &install.fixture);
        let staged = installer
            .stage(InstallRequest {
                world: world.clone(),
                coordinate: ChunkCoordinate {
                    x: install.chunk[0],
                    z: install.chunk[1],
                },
                revision: 1,
                snapshots: snapshot_bundle(fixture),
            })
            .await
            .unwrap_or_else(|error| panic!("stage {id} {}: {error}", install.fixture));
        staged
            .publish()
            .await
            .unwrap_or_else(|error| panic!("publish {id} {}: {error}", install.fixture));
    }

    let hashes = load_hashes();
    let expected = hashes
        .cases
        .get(id)
        .unwrap_or_else(|| panic!("{id} missing from committed Java hashes"));
    let mut actual = BTreeMap::new();
    let mut mismatches = Vec::new();
    for (relative, java_hash) in expected {
        let prefixed = PathBuf::from("world").join(relative);
        let Some(png) = store.read(&prefixed).await.unwrap() else {
            mismatches.push(format!("{id} {relative} missing rust tile"));
            continue;
        };
        let rgba = decode_rgba_png(&png).unwrap_or_else(|error| panic!("{id} {relative}: {error}"));
        assert_eq!(rgba.len(), TILE_RGBA_BYTES, "{id} {relative} rgba length");
        let rust_hash = sha256_hex(&rgba);
        actual.insert(relative.clone(), rust_hash.clone());
        if rust_hash != *java_hash {
            let mut detail = first_pixel_mismatch(&rgba, relative);
            for z in 0..TILE_SIZE {
                let mut found = false;
                for x in 0..TILE_SIZE {
                    let px = pixel(&rgba, x, z);
                    if px != [0, 0, 0, 0] {
                        detail = format!(" first non-zero rust pixel ({x},{z})={px:?}");
                        found = true;
                        break;
                    }
                }
                if found {
                    break;
                }
            }
            mismatches.push(format!(
                "{id} {relative} rust={rust_hash} java={java_hash}{detail}"
            ));
        }
    }
    if store.file_count() != expected.len() {
        mismatches.push(format!(
            "{id} rust file count {} != java path count {}",
            store.file_count(),
            expected.len()
        ));
    }
    assert!(
        mismatches.is_empty(),
        "{}",
        mismatches.join("; ")
    );
    (actual, store)
}

#[tokio::test]
async fn installer_catalog_denominator_is_six_cases() {
    let (manifest, bytes) = load_manifest();
    let hashes = load_hashes();
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.max_zoom, 3);
    assert_eq!(manifest.cases.len(), 6);
    assert_eq!(hashes.schema_version, 1);
    assert_eq!(hashes.max_zoom, 3);
    assert_eq!(hashes.manifest_hash, sha256_hex(&bytes));
    let ids: Vec<&str> = manifest.cases.iter().map(|case| case.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "fixture-flat-solid-at-0-0",
            "fixture-flat-solid-at-1-0",
            "fixture-flat-solid-then-iterate-down",
            "fixture-flat-solid-at-31-31",
            "fixture-flat-solid-at-negative",
            "fixture-flat-solid-at-32-0",
        ]
    );
    assert_eq!(hashes.cases.len(), 6);
}

#[tokio::test]
async fn installer_places_flat_solid_at_0_0_matching_java_image_rgba() {
    run_case("fixture-flat-solid-at-0-0").await;
}

#[tokio::test]
async fn installer_places_flat_solid_at_1_0_matching_java_image_rgba() {
    run_case("fixture-flat-solid-at-1-0").await;
}

#[tokio::test]
async fn installer_places_flat_solid_then_iterate_down_matching_java_image_rgba() {
    run_case("fixture-flat-solid-then-iterate-down").await;
}

#[tokio::test]
async fn installer_places_flat_solid_at_31_31_matching_java_image_rgba() {
    run_case("fixture-flat-solid-at-31-31").await;
}

#[tokio::test]
async fn installer_places_flat_solid_at_negative_matching_java_image_rgba() {
    run_case("fixture-flat-solid-at-negative").await;
}

#[tokio::test]
async fn installer_places_flat_solid_at_32_0_matching_java_image_rgba() {
    run_case("fixture-flat-solid-at-32-0").await;
}
