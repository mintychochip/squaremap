use crate::{
    BiomeSource, Limits, Registry, RenderContext, RenderError, RenderSettings, Snapshot,
    SnapshotBiomeSource, render_chunk,
};
use prost::Message;
use serde::Deserialize;
use squaremap_protocol::wire::RegistryReplace;
use std::{
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const VALID_IDS: [&str; 26] = [
    "flat-solid",
    "empty-chunk",
    "north-height-discontinuity",
    "south-height-discontinuity",
    "missing-north",
    "missing-south",
    "iterate-down",
    "iterate-up",
    "ceiling-iterate-down",
    "ceiling-iterate-up",
    "max-height-clipped",
    "water-depth-cap",
    "water-clear",
    "water-checkerboard",
    "lava-depth-checkerboard",
    "clear-glass",
    "stained-glass",
    "glass-disabled",
    "invisible-block",
    "biome-off",
    "biome-grass-radius-0",
    "biome-foliage-radius-0",
    "biome-water-radius-0",
    "biome-blend-radius-3",
    "biome-blend-cross-boundary",
    "negative-min-y",
];
const MALFORMED_IDS: [&str; 14] = [
    "malformed-unknown-descriptor",
    "malformed-heightmap",
    "malformed-palette",
    "malformed-bounds",
    "malformed-crc",
    "malformed-length",
    "malformed-section-count",
    "malformed-section-y",
    "malformed-packed-width",
    "malformed-trailing-bits",
    "malformed-duplicate-palette",
    "malformed-protobuf-body",
    "malformed-generation",
    "malformed-neighbor-coordinate",
];

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub generator: String,
    pub biome_zoom_seed: i64,
    pub valid: Vec<ValidRow>,
    pub malformed: Vec<MalformedRow>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidRow {
    pub id: String,
    pub registry: String,
    pub chunk: String,
    pub north: Option<String>,
    pub south: Option<String>,
    pub biome_sources: Vec<BiomePath>,
    pub max_height: i32,
    pub iterate_up: bool,
    pub glass_clear: bool,
    pub water_checkerboard: bool,
    pub water_clear: bool,
    pub lava_checkerboard: bool,
    pub biome_enabled: bool,
    pub biome_blend: u32,
    pub ceiling: bool,
    pub invisible_id: u32,
    pub iterate_up_base_id: u32,
    pub pixels: Vec<u64>,
    pub south_edge: Vec<i32>,
    pub grass_samples: Vec<GrassSample>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct BiomePath {
    pub x: i32,
    pub z: i32,
    pub path: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct GrassSample {
    pub block_x: i32,
    pub block_y: i32,
    pub block_z: i32,
    pub biome_id: u32,
    pub resolved_grass_argb: u64,
}
#[derive(Clone, Debug, Deserialize)]
pub struct MalformedRow {
    pub id: String,
    pub path: String,
    pub mutation: String,
    pub classifier: String,
}

pub struct PreparedCase {
    pub row: ValidRow,
    pub context: RenderContext,
    pub center: Arc<Snapshot>,
    pub north: Option<Arc<Snapshot>>,
    pub south: Option<Arc<Snapshot>>,
    pub biome_source: Option<SnapshotBiomeSource>,
}
pub struct FixtureCorpus {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub cases: Vec<PreparedCase>,
    pub registry: Registry,
}

fn safe_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = root.join(relative);
    if !path.starts_with(root) || !path.is_file() {
        return Err(format!("invalid fixture path {relative}"));
    }
    for component in path
        .strip_prefix(root)
        .map_err(|_| "fixture path".to_string())?
        .components()
    {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err("fixture path escapes root".into());
        }
    }
    Ok(path)
}
fn load_snapshot(
    root: &Path,
    relative: &str,
    registry: &Registry,
) -> Result<Arc<Snapshot>, String> {
    let bytes = fs::read(safe_file(root, relative)?).map_err(|error| error.to_string())?;
    Snapshot::decode_bytes(&bytes, registry, Limits::default())
        .map(Arc::new)
        .map_err(|error| error.to_string())
}

impl FixtureCorpus {
    pub fn load(root: &Path) -> Result<Self, String> {
        let root = root.canonicalize().map_err(|error| error.to_string())?;
        let manifest: Manifest = serde_json::from_slice(
            &fs::read(safe_file(&root, "manifest.json")?).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        if manifest.schema_version != 2
            || manifest.generator != "chunk-render-java-v2"
            || manifest.biome_zoom_seed != 24_301
            || manifest.valid.len() != VALID_IDS.len()
            || manifest.malformed.len() != MALFORMED_IDS.len()
        {
            return Err("fixture manifest mismatch".into());
        }
        if manifest
            .valid
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>()
            != VALID_IDS
        {
            return Err("valid fixture order mismatch".into());
        }
        let registry_bytes = fs::read(safe_file(&root, "registry/registry.bin")?)
            .map_err(|error| error.to_string())?;
        let registry_message = RegistryReplace::decode(registry_bytes.as_slice())
            .map_err(|error| error.to_string())?;
        let registry =
            Registry::with_replace(registry_message).map_err(|error| error.to_string())?;
        let mut cases = Vec::with_capacity(manifest.valid.len());
        for row in &manifest.valid {
            let mut source =
                SnapshotBiomeSource::new(registry.generation(), manifest.biome_zoom_seed);
            for biome in &row.biome_sources {
                let snapshot = load_snapshot(&root, &biome.path, &registry)?;
                source = source
                    .with_snapshot(snapshot)
                    .map_err(|error| error.to_string())?;
            }
            let center = load_snapshot(&root, &row.chunk, &registry)?;
            let north = row
                .north
                .as_deref()
                .map(|path| load_snapshot(&root, path, &registry))
                .transpose()?;
            let south = row
                .south
                .as_deref()
                .map(|path| load_snapshot(&root, path, &registry))
                .transpose()?;
            for sample in &row.grass_samples {
                let selected = source
                    .sample_block(sample.block_x, sample.block_y, sample.block_z)
                    .map_err(|error| error.to_string())?;
                if selected != sample.biome_id {
                    return Err(format!("grass selector mismatch {}", row.id));
                }
                let resolved = source
                    .resolved_grass(
                        sample.block_x,
                        sample.block_y,
                        sample.block_z,
                        sample.biome_id,
                    )
                    .map_err(|error| error.to_string())?;
                if resolved != sample.resolved_grass_argb as u32 {
                    return Err(format!("grass oracle mismatch {}", row.id));
                }
            }
            let settings = RenderSettings {
                iterate_up: row.iterate_up,
                map_max_height: row.max_height,
                biomes_enabled: row.biome_enabled,
                biome_blend: row.biome_blend,
                glass_clear: row.glass_clear,
                water_clear: row.water_clear,
                water_checkerboard: row.water_checkerboard,
                lava_checkerboard: row.lava_checkerboard,
            };
            let context_source = row
                .biome_enabled
                .then(|| Arc::new(source.clone()) as Arc<dyn BiomeSource>);
            let context = RenderContext::try_new(
                registry.generation(),
                settings,
                [row.invisible_id].into_iter().filter(|id| *id != 0),
                [row.iterate_up_base_id].into_iter().filter(|id| *id != 0),
                context_source,
                || false,
            )
            .map_err(|error| error.to_string())?;
            cases.push(PreparedCase {
                row: row.clone(),
                context,
                center,
                north,
                south,
                biome_source: row.biome_enabled.then_some(source),
            });
        }
        Ok(Self {
            root,
            manifest,
            cases,
            registry,
        })
    }
}
pub fn render_case(case: &PreparedCase) -> Result<crate::ChunkPixels, RenderError> {
    render_chunk(
        &case.context,
        case.north.as_deref(),
        &case.center,
        case.south.as_deref(),
    )
}
