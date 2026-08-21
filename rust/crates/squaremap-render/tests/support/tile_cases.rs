use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use squaremap_render::coordinates::RegionCoord;
use squaremap_render::{
    MemoryTileStore, PngOptions, RegionPixels, TILE_SIZE, TileError, TilePyramid, TileStore,
    decode_rgba_png,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const MANIFEST_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../testdata/bridge/v2/tiles/manifest.json"
);
pub const MAX_ZOOM: u8 = 3;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub max_zoom: u8,
    pub cases: Vec<TileCase>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileCase {
    pub id: String,
    pub region: [i32; 2],
    pub pixels: PixelPattern,
    #[serde(default)]
    pub existing: Option<PixelPattern>,
    #[serde(default)]
    pub existing_from: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PixelPattern {
    Solid {
        argb: String,
    },
    Rect {
        x: usize,
        z: usize,
        w: usize,
        h: usize,
        argb: String,
    },
    Checker {
        step: usize,
        even: String,
        odd: String,
    },
    Gradient,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ProbeReport {
    pub backend: String,
    pub manifest_hash: String,
    pub max_zoom: u8,
    pub cases: Vec<ProbeCaseReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ProbeCaseReport {
    pub id: String,
    pub paths: Vec<String>,
    pub pixel_sha256: Vec<String>,
}

pub struct CaseOutput {
    pub store: Arc<MemoryTileStore>,
    pub paths: Vec<String>,
}

pub struct ProbedCase {
    pub id: String,
    pub paths: Vec<String>,
    pub rgba: Vec<Vec<u8>>,
}

pub struct CatalogProbe {
    pub report: ProbeReport,
    pub tiles: Vec<ProbedCase>,
}

impl Manifest {
    pub fn case(&self, id: &str) -> &TileCase {
        self.cases
            .iter()
            .find(|case| case.id == id)
            .unwrap_or_else(|| panic!("unknown tile case {id}"))
    }
}

impl CatalogProbe {
    pub fn rgba(&self, case_id: &str, relative: &str) -> Option<&[u8]> {
        let case = self.tiles.iter().find(|case| case.id == case_id)?;
        let index = case.paths.iter().position(|path| path == relative)?;
        Some(case.rgba[index].as_slice())
    }
}

pub fn load_manifest(path: &Path) -> (Manifest, Vec<u8>) {
    load_manifest_result(path).unwrap_or_else(|error| panic!("{error}"))
}

pub fn load_manifest_result(path: &Path) -> Result<(Manifest, Vec<u8>), String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse {}: {error}", path.display()))?;
    if manifest.schema_version != 1 {
        return Err(format!("schema_version {} != 1", manifest.schema_version));
    }
    Ok((manifest, bytes))
}

pub fn parse_argb(value: &str) -> u32 {
    let digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or_else(|| panic!("ARGB hex must start with 0x: {value}"));
    u32::from_str_radix(digits, 16).unwrap_or_else(|error| panic!("ARGB hex {value}: {error}"))
}

pub fn fill_pixels(pattern: &PixelPattern) -> RegionPixels {
    let mut pixels = RegionPixels::empty();
    match pattern {
        PixelPattern::Solid { argb } => {
            let color = parse_argb(argb);
            for x in 0..TILE_SIZE {
                for z in 0..TILE_SIZE {
                    pixels.set_argb(x, z, color).expect("region pixel");
                }
            }
        }
        PixelPattern::Rect { x, z, w, h, argb } => {
            let color = parse_argb(argb);
            let x_end = x.checked_add(*w).expect("rect x overflow");
            let z_end = z.checked_add(*h).expect("rect z overflow");
            for px in *x..x_end {
                for pz in *z..z_end {
                    pixels.set_argb(px, pz, color).expect("rect pixel");
                }
            }
        }
        PixelPattern::Checker { step, even, odd } => {
            assert!(*step > 0, "checker step must be positive");
            let even_color = parse_argb(even);
            let odd_color = parse_argb(odd);
            for x in 0..TILE_SIZE {
                for z in 0..TILE_SIZE {
                    let color = if ((x / *step) + (z / *step)) % 2 == 0 {
                        even_color
                    } else {
                        odd_color
                    };
                    pixels.set_argb(x, z, color).expect("checker pixel");
                }
            }
        }
        PixelPattern::Gradient => {
            for x in 0..TILE_SIZE {
                for z in 0..TILE_SIZE {
                    let argb = 0xFF00_0000 | ((x as u32 & 255) << 16) | ((z as u32 & 255) << 8);
                    pixels.set_argb(x, z, argb).expect("gradient pixel");
                }
            }
        }
    }
    pixels
}

pub fn path_string(path: &Path) -> String {
    path.iter()
        .map(|component| component.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub async fn apply_case(
    pyramid: &TilePyramid,
    manifest: &Manifest,
    case: &TileCase,
) -> Result<Vec<String>, TileError> {
    let mut chain = Vec::new();
    let mut current = case;
    loop {
        chain.push(current);
        match current.existing_from.as_deref() {
            Some(from) => current = manifest.case(from),
            None => break,
        }
    }
    chain.reverse();

    let mut paths = Vec::new();
    for case in chain {
        let region = RegionCoord {
            x: case.region[0],
            z: case.region[1],
        };
        if let Some(existing) = &case.existing {
            paths.extend(apply_pattern(pyramid, region, existing).await?);
        }
        paths.extend(apply_pattern(pyramid, region, &case.pixels).await?);
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub async fn apply_case_to(
    store: Arc<dyn TileStore>,
    manifest: &Manifest,
    case: &TileCase,
) -> Result<Vec<String>, TileError> {
    let pyramid = TilePyramid::new(store, manifest.max_zoom, PngOptions { compression: false })?;
    apply_case(&pyramid, manifest, case).await
}

pub async fn run_case(manifest: &Manifest, case: &TileCase) -> Result<CaseOutput, TileError> {
    let store = Arc::new(MemoryTileStore::new());
    let paths = apply_case_to(store.clone(), manifest, case).await?;
    Ok(CaseOutput { store, paths })
}

pub async fn probe_catalog(
    manifest: &Manifest,
    manifest_bytes: &[u8],
) -> Result<CatalogProbe, TileError> {
    let mut tiles = Vec::new();
    let mut report_cases = Vec::new();
    for case in &manifest.cases {
        let output = run_case(manifest, case).await?;
        let mut rgba = Vec::new();
        let mut hashes = Vec::new();
        for relative in &output.paths {
            let path = PathBuf::from(relative);
            let png = output
                .store
                .read(&path)
                .await
                .map_err(|error| TileError::Store {
                    path: path.clone(),
                    message: error.message,
                })?
                .ok_or_else(|| TileError::Store {
                    path: path.clone(),
                    message: "missing tile after apply".into(),
                })?;
            let pixels = decode_rgba_png(&png).map_err(|error| TileError::Decode {
                path,
                message: error.to_string(),
            })?;
            hashes.push(sha256_hex(&pixels));
            rgba.push(pixels);
        }
        report_cases.push(ProbeCaseReport {
            id: case.id.clone(),
            paths: output.paths.clone(),
            pixel_sha256: hashes,
        });
        tiles.push(ProbedCase {
            id: case.id.clone(),
            paths: output.paths,
            rgba,
        });
    }
    Ok(CatalogProbe {
        report: ProbeReport {
            backend: "rust".into(),
            manifest_hash: sha256_hex(manifest_bytes),
            max_zoom: manifest.max_zoom,
            cases: report_cases,
        },
        tiles,
    })
}

async fn apply_pattern(
    pyramid: &TilePyramid,
    region: RegionCoord,
    pattern: &PixelPattern,
) -> Result<Vec<String>, TileError> {
    let result = pyramid.apply_region(region, &fill_pixels(pattern)).await?;
    Ok(result
        .changed_paths
        .iter()
        .map(|path| path_string(path))
        .collect())
}
