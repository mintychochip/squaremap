use prost::Message;
use serde::Deserialize;
use squaremap_protocol::wire::RegistryReplace;
use squaremap_render::{
    BiomeSource, Limits, Registry, RenderContext, RenderError, RenderSettings, Snapshot,
    SnapshotBiomeSource, SnapshotError, render_chunk,
};
use std::{
    collections::HashSet,
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
const RADIUS_ZERO_SOURCES: [(i32, i32); 5] = [(-1, 0), (0, -1), (0, 0), (0, 1), (1, 0)];
const RADIUS_THREE_SOURCES: [(i32, i32); 9] = [
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (0, -1),
    (0, 0),
    (0, 1),
    (1, -1),
    (1, 0),
    (1, 1),
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
#[serde(deny_unknown_fields)]
pub struct BiomePath {
    pub x: i32,
    pub z: i32,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrassSample {
    pub block_x: i32,
    pub block_y: i32,
    pub block_z: i32,
    pub biome_id: u32,
    pub resolved_grass_argb: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MalformedRow {
    pub id: String,
    pub path: String,
    pub mutation: String,
    pub classifier: String,
}

pub struct PreparedCase {
    pub row: ValidRow,
    pub context: RenderContext,
    pub biome_source: Option<SnapshotBiomeSource>,
    pub center: Arc<Snapshot>,
    pub north: Option<Arc<Snapshot>>,
    pub south: Option<Arc<Snapshot>>,
    pub expected_pixels: [u32; 256],
    pub expected_edge: [i32; 16],
}

pub struct FixtureCorpus {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub registry: Registry,
    pub cases: Vec<PreparedCase>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExpectedSettings {
    max_height: i32,
    iterate_up: bool,
    glass_clear: bool,
    water_checkerboard: bool,
    water_clear: bool,
    lava_checkerboard: bool,
    biome_enabled: bool,
    biome_blend: u32,
    ceiling: bool,
    invisible_id: u32,
    iterate_up_base_id: u32,
}

impl ExpectedSettings {
    const BASELINE: Self = Self {
        max_height: -1,
        iterate_up: false,
        glass_clear: true,
        water_checkerboard: false,
        water_clear: false,
        lava_checkerboard: false,
        biome_enabled: false,
        biome_blend: 0,
        ceiling: false,
        invisible_id: 0,
        iterate_up_base_id: 0,
    };
}

fn expected_settings(id: &str) -> ExpectedSettings {
    let mut expected = ExpectedSettings::BASELINE;
    match id {
        "iterate-up" => expected.iterate_up = true,
        "ceiling-iterate-down" => expected.ceiling = true,
        "ceiling-iterate-up" => {
            expected.iterate_up = true;
            expected.ceiling = true;
            expected.iterate_up_base_id = 3;
        }
        "max-height-clipped" => expected.max_height = 16,
        "water-clear" => expected.water_clear = true,
        "water-checkerboard" => expected.water_checkerboard = true,
        "lava-depth-checkerboard" => expected.lava_checkerboard = true,
        "glass-disabled" => expected.glass_clear = false,
        "invisible-block" => expected.invisible_id = 8,
        "biome-grass-radius-0" | "biome-foliage-radius-0" | "biome-water-radius-0" => {
            expected.biome_enabled = true;
        }
        "biome-blend-radius-3" | "biome-blend-cross-boundary" => {
            expected.biome_enabled = true;
            expected.biome_blend = 3;
        }
        _ => {}
    }
    expected
}

fn actual_settings(row: &ValidRow) -> ExpectedSettings {
    ExpectedSettings {
        max_height: row.max_height,
        iterate_up: row.iterate_up,
        glass_clear: row.glass_clear,
        water_checkerboard: row.water_checkerboard,
        water_clear: row.water_clear,
        lava_checkerboard: row.lava_checkerboard,
        biome_enabled: row.biome_enabled,
        biome_blend: row.biome_blend,
        ceiling: row.ceiling,
        invisible_id: row.invisible_id,
        iterate_up_base_id: row.iterate_up_base_id,
    }
}

fn expected_source_keys(id: &str) -> &'static [(i32, i32)] {
    match id {
        "biome-grass-radius-0" | "biome-foliage-radius-0" | "biome-water-radius-0" => {
            &RADIUS_ZERO_SOURCES
        }
        "biome-blend-radius-3" | "biome-blend-cross-boundary" => &RADIUS_THREE_SOURCES,
        _ => &[],
    }
}

fn expected_grass_coordinate(id: &str, index: usize) -> Option<(i32, i32, i32)> {
    match id {
        "biome-grass-radius-0" if index < 256 => {
            Some(((index / 16) as i32, 15, (index % 16) as i32))
        }
        "biome-blend-radius-3" | "biome-blend-cross-boundary" if index < 441 => {
            Some((-3 + (index / 21) as i32, 15, -3 + (index % 21) as i32))
        }
        _ => None,
    }
}

fn expected_neighbors(id: &str) -> (bool, bool) {
    let north = matches!(
        id,
        "north-height-discontinuity"
            | "missing-south"
            | "water-depth-cap"
            | "water-clear"
            | "water-checkerboard"
            | "lava-depth-checkerboard"
            | "clear-glass"
            | "stained-glass"
            | "glass-disabled"
    );
    let south = matches!(id, "south-height-discontinuity" | "missing-north");
    (north, south)
}

fn expected_bounds(id: &str) -> (i32, i32) {
    match id {
        "negative-min-y" => (-32, -1),
        "north-height-discontinuity"
        | "south-height-discontinuity"
        | "ceiling-iterate-down"
        | "ceiling-iterate-up"
        | "max-height-clipped" => (0, 31),
        _ => (0, 15),
    }
}

fn malformed_metadata(id: &str) -> (&'static str, &'static str, &'static str) {
    match id {
        "malformed-unknown-descriptor" => (
            "malformed/unknown-descriptor.bin",
            "block_palette[0]",
            "UnknownDescriptor(block)",
        ),
        "malformed-heightmap" => (
            "malformed/heightmap.bin",
            "heightmap[0]",
            "HeightOutOfRange",
        ),
        "malformed-palette" => (
            "malformed/palette.bin",
            "block_palette",
            "PaletteLength(block)",
        ),
        "malformed-bounds" => ("malformed/bounds.bin", "max_y", "VerticalBounds"),
        "malformed-crc" => ("malformed/crc.bin", "crc32c", "Crc"),
        "malformed-length" => (
            "malformed/length.bin",
            "uncompressed_length",
            "DecompressedLength",
        ),
        "malformed-section-count" => (
            "malformed/section-count.bin",
            "sections",
            "SectionCountMismatch",
        ),
        "malformed-section-y" => ("malformed/section-y.bin", "section_y", "SectionYOutOfRange"),
        "malformed-packed-width" => (
            "malformed/packed-width.bin",
            "block_indices",
            "InvalidPacking(block length)",
        ),
        "malformed-trailing-bits" => (
            "malformed/trailing-bits.bin",
            "block_indices[index=3]",
            "InvalidPacking(block index)",
        ),
        "malformed-duplicate-palette" => (
            "malformed/duplicate-palette.bin",
            "block_palette[last]",
            "DuplicatePalette(block)",
        ),
        "malformed-protobuf-body" => ("malformed/protobuf-body.bin", "body", "Protobuf"),
        "malformed-generation" => ("malformed/generation.bin", "revision", "RegistryMismatch"),
        "malformed-neighbor-coordinate" => (
            "malformed/neighbor-coordinate.bin",
            "coordinate",
            "CoordinateMismatch",
        ),
        _ => ("", "", ""),
    }
}

fn safe(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("unsafe path {relative}"));
    }
    let candidate = root
        .join(path)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !candidate.starts_with(root) {
        return Err(format!("path escapes root {relative}"));
    }
    Ok(candidate)
}

fn safe_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = safe(root, relative)?;
    if !path.is_file() {
        return Err(format!("not a regular file {relative}"));
    }
    Ok(path)
}

fn decode(root: &Path, path: &str, registry: &Registry) -> Result<Snapshot, String> {
    let bytes = fs::read(safe_file(root, path)?).map_err(|error| error.to_string())?;
    Snapshot::decode_bytes(&bytes, registry, Limits::default())
        .map_err(|error| format!("{path}: {error:?}"))
}

fn validate_neighbor(
    row: &ValidRow,
    center: &Snapshot,
    neighbor: &Snapshot,
    north: bool,
) -> Result<(), String> {
    if neighbor.world.namespace != center.world.namespace
        || neighbor.world.value != center.world.value
        || neighbor.world.epoch != center.world.epoch
        || neighbor.revision != center.revision
        || neighbor.min_y != center.min_y
        || neighbor.max_y != center.max_y
        || neighbor.ceiling != center.ceiling
    {
        return Err(format!("neighbor metadata mismatch {}", row.id));
    }
    let expected_z = if north {
        center.coordinate.z.checked_sub(1)
    } else {
        center.coordinate.z.checked_add(1)
    }
    .ok_or_else(|| format!("neighbor coordinate overflow {}", row.id))?;
    if neighbor.coordinate.x != center.coordinate.x || neighbor.coordinate.z != expected_z {
        return Err(format!("neighbor coordinate mismatch {}", row.id));
    }
    Ok(())
}

fn collect_files(directory: &Path, files: &mut HashSet<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_symlink() {
            return Err(format!(
                "fixture symlink rejected {}",
                entry.path().display()
            ));
        }
        if file_type.is_dir() {
            collect_files(&entry.path(), files)?;
        } else if file_type.is_file() {
            files.insert(entry.path());
        } else {
            return Err(format!(
                "unsupported fixture entry {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

impl FixtureCorpus {
    pub fn load(root: &Path) -> Result<Self, String> {
        let root = root.canonicalize().map_err(|error| error.to_string())?;
        if !root.is_dir() {
            return Err("fixture root is not a directory".into());
        }
        let manifest_path = safe_file(&root, "manifest.json")?;
        let expected_inventory: HashSet<PathBuf> = ["manifest.json", "registry/registry.bin"]
            .into_iter()
            .map(|relative| root.join(relative))
            .collect();
        let mut expected_inventory = expected_inventory;
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(manifest_path).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        if manifest.schema_version != 2
            || manifest.generator != "chunk-render-java-v2"
            || manifest.biome_zoom_seed != 24_301
        {
            return Err("manifest metadata mismatch".into());
        }
        if manifest.valid.len() != VALID_IDS.len()
            || manifest.malformed.len() != MALFORMED_IDS.len()
        {
            return Err("manifest count mismatch".into());
        }
        let valid_ids: Vec<_> = manifest.valid.iter().map(|row| row.id.as_str()).collect();
        if valid_ids != VALID_IDS {
            return Err("valid ID set/order mismatch".into());
        }
        let malformed_ids: Vec<_> = manifest
            .malformed
            .iter()
            .map(|row| row.id.as_str())
            .collect();
        if malformed_ids != MALFORMED_IDS {
            return Err("malformed ID set/order mismatch".into());
        }

        let registry_relative = "registry/registry.bin";
        if manifest
            .valid
            .iter()
            .any(|row| row.registry != registry_relative)
        {
            return Err("registry path mismatch".into());
        }
        let registry_file = safe_file(&root, registry_relative)?;
        let registry_bytes = fs::read(&registry_file).map_err(|error| error.to_string())?;
        let replacement = RegistryReplace::decode(registry_bytes.as_slice())
            .map_err(|error| error.to_string())?;
        let registry = Registry::with_replace(replacement).map_err(|error| error.to_string())?;
        let registry_world = registry.world().ok_or("registry missing world")?;
        if registry.revision() != 11
            || registry_world.namespace != "minecraft"
            || registry_world.value != "overworld"
            || registry_world.epoch != 1
        {
            return Err("registry identity mismatch".into());
        }

        let mut referenced = HashSet::<PathBuf>::new();
        referenced.insert(safe_file(&root, "manifest.json")?);
        let mut register_nonregistry = |relative: &str| -> Result<(), String> {
            let resolved = safe_file(&root, relative)?;
            if resolved != registry_file && !referenced.insert(resolved) {
                return Err(format!("duplicate referenced path {relative}"));
            }
            Ok(())
        };

        for row in &manifest.malformed {
            register_nonregistry(&row.path)?;
            expected_inventory.insert(root.join(&row.path));
            let (path, mutation, classifier) = malformed_metadata(&row.id);
            if row.path != path {
                return Err(format!("malformed path mismatch {}", row.id));
            }
            if row.mutation != mutation {
                return Err(format!("mutation mismatch {}", row.id));
            }
            if row.classifier != classifier {
                return Err(format!("classifier mismatch {}", row.id));
            }
        }

        let mut cases = Vec::with_capacity(VALID_IDS.len());
        for row in &manifest.valid {
            if actual_settings(row) != expected_settings(&row.id) {
                return Err(format!("settings mismatch {}", row.id));
            }

            expected_inventory.insert(root.join(&row.chunk));
            register_nonregistry(&row.chunk)?;
            if row.chunk != format!("chunks/{}/center.bin", row.id) {
                return Err(format!("center path mismatch {}", row.id));
            }
            if let Some(path) = row.north.as_deref() {
                expected_inventory.insert(root.join(path));
                register_nonregistry(path)?;
                if path != format!("chunks/{}/north.bin", row.id) {
                    return Err(format!("north path mismatch {}", row.id));
                }
            }
            if let Some(path) = row.south.as_deref() {
                expected_inventory.insert(root.join(path));
                register_nonregistry(path)?;
                if path != format!("chunks/{}/south.bin", row.id) {
                    return Err(format!("south path mismatch {}", row.id));
                }
            }
            for biome in &row.biome_sources {
                expected_inventory.insert(root.join(&biome.path));
                register_nonregistry(&biome.path)?;
            }

            if row.pixels.len() != 256
                || row.south_edge.len() != 16
                || row.pixels.iter().any(|pixel| *pixel > u32::MAX as u64)
            {
                return Err(format!("oracle vector shape/range {}", row.id));
            }
            if row.id != "empty-chunk" && row.pixels.iter().all(|pixel| *pixel == 0) {
                return Err(format!("unexpected all-zero oracle {}", row.id));
            }
            let mut expected_pixels = [0_u32; 256];
            for (index, pixel) in row.pixels.iter().enumerate() {
                expected_pixels[index] = *pixel as u32;
            }
            let mut expected_edge = [0_i32; 16];
            expected_edge.copy_from_slice(&row.south_edge);

            let center = Arc::new(decode(&root, &row.chunk, &registry)?);
            let (expected_min_y, expected_max_y) = expected_bounds(&row.id);
            if center.coordinate.x != 0
                || center.coordinate.z != 0
                || center.min_y != expected_min_y
                || center.max_y != expected_max_y
                || center.ceiling != row.ceiling
            {
                return Err(format!("center metadata mismatch {}", row.id));
            }

            let (north_expected, south_expected) = expected_neighbors(&row.id);
            if row.north.is_some() != north_expected || row.south.is_some() != south_expected {
                return Err(format!("neighbor topology mismatch {}", row.id));
            }
            let north = row
                .north
                .as_deref()
                .map(|path| decode(&root, path, &registry).map(Arc::new))
                .transpose()?;
            let south = row
                .south
                .as_deref()
                .map(|path| decode(&root, path, &registry).map(Arc::new))
                .transpose()?;
            if let Some(neighbor) = north.as_deref() {
                validate_neighbor(row, &center, neighbor, true)?;
            }
            if let Some(neighbor) = south.as_deref() {
                validate_neighbor(row, &center, neighbor, false)?;
            }

            let expected_sources = expected_source_keys(&row.id);
            let source_keys: Vec<_> = row
                .biome_sources
                .iter()
                .map(|source| (source.x, source.z))
                .collect();
            if source_keys != expected_sources {
                return Err(format!("biome source topology {}", row.id));
            }
            let mut source =
                SnapshotBiomeSource::new(registry.generation(), manifest.biome_zoom_seed);
            for biome in &row.biome_sources {
                if biome.path != format!("chunks/{}/biome-{}-{}.bin", row.id, biome.x, biome.z) {
                    return Err(format!("biome source path mismatch {}", row.id));
                }
                let snapshot = Arc::new(decode(&root, &biome.path, &registry)?);
                if snapshot.coordinate.x != biome.x
                    || snapshot.coordinate.z != biome.z
                    || snapshot.world.namespace != center.world.namespace
                    || snapshot.world.value != center.world.value
                    || snapshot.world.epoch != center.world.epoch
                    || snapshot.revision != center.revision
                    || snapshot.min_y != center.min_y
                    || snapshot.max_y != center.max_y
                    || snapshot.ceiling != center.ceiling
                {
                    return Err(format!("biome source metadata mismatch {}", row.id));
                }
                source = source
                    .with_snapshot(snapshot)
                    .map_err(|error| error.to_string())?;
            }

            let expected_grass_count = match row.id.as_str() {
                "biome-grass-radius-0" => 256,
                "biome-blend-radius-3" | "biome-blend-cross-boundary" => 441,
                _ => 0,
            };
            if row.grass_samples.len() != expected_grass_count {
                return Err(format!("grass tuple count {}", row.id));
            }
            let mut grass_keys = HashSet::with_capacity(row.grass_samples.len());
            for (index, sample) in row.grass_samples.iter().enumerate() {
                let expected_coordinate = expected_grass_coordinate(&row.id, index)
                    .ok_or_else(|| format!("grass tuple coordinate {}", row.id))?;
                if (sample.block_x, sample.block_y, sample.block_z) != expected_coordinate {
                    return Err(format!("grass tuple coordinate/order {}", row.id));
                }
                if !grass_keys.insert((
                    sample.block_x,
                    sample.block_y,
                    sample.block_z,
                    sample.biome_id,
                )) {
                    return Err(format!("duplicate grass tuple {}", row.id));
                }
                if sample.resolved_grass_argb > u32::MAX as u64 {
                    return Err(format!("grass oracle range {}", row.id));
                }
                let selected = source
                    .sample_block(sample.block_x, sample.block_y, sample.block_z)
                    .map_err(|error| format!("grass selector {}: {error}", row.id))?;
                if selected != sample.biome_id {
                    return Err(format!("grass selected biome mismatch {}", row.id));
                }
                source = source.with_grass_resolved(
                    sample.block_x,
                    sample.block_y,
                    sample.block_z,
                    sample.biome_id,
                    sample.resolved_grass_argb as u32,
                );
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
            let retained_source = row.biome_enabled.then(|| source.clone());
            let biome_source = row
                .biome_enabled
                .then(|| Arc::new(source) as Arc<dyn squaremap_render::BiomeSource>);
            let context = RenderContext::try_new(
                registry.generation(),
                settings,
                [row.invisible_id].into_iter().filter(|id| *id != 0),
                [row.iterate_up_base_id].into_iter().filter(|id| *id != 0),
                biome_source,
                || false,
            )
            .map_err(|error| error.to_string())?;
            cases.push(PreparedCase {
                row: row.clone(),
                context,
                biome_source: retained_source,
                center,
                north,
                south,
                expected_pixels,
                expected_edge,
            });
        }

        let mut actual_inventory = HashSet::new();
        collect_files(&root, &mut actual_inventory)?;
        if actual_inventory != expected_inventory {
            return Err("fixture file inventory mismatch".into());
        }

        Ok(Self {
            root,
            manifest,
            registry,
            cases,
        })
    }
}

pub fn malformed_error(
    row: &MalformedRow,
    corpus: &FixtureCorpus,
) -> Result<Snapshot, SnapshotError> {
    let path = safe_file(&corpus.root, &row.path)
        .map_err(|_| SnapshotError::Protobuf("fixture path".into()))?;
    let bytes = fs::read(path).map_err(|_| SnapshotError::Protobuf("fixture read".into()))?;
    Snapshot::decode_bytes(&bytes, &corpus.registry, Limits::default())
}

pub fn render_case(case: &PreparedCase) -> Result<squaremap_render::ChunkPixels, RenderError> {
    render_chunk(
        &case.context,
        case.north.as_deref(),
        &case.center,
        case.south.as_deref(),
    )
}
