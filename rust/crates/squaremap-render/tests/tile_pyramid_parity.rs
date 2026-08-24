#[path = "support/tile_cases.rs"]
mod tile_cases;

use serde::Deserialize;
use squaremap_render::{TILE_RGBA_BYTES, TILE_SIZE, TileStore, decode_rgba_png};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use tile_cases::{MANIFEST_PATH, MAX_ZOOM, load_manifest, probe_catalog, run_case};

const JAVA_HASHES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../testdata/bridge/v2/tiles/java-oracle-hashes.json"
);

#[derive(Deserialize)]
struct JavaOracleHashes {
    schema_version: u32,
    manifest_hash: String,
    max_zoom: u8,
    cases: BTreeMap<String, BTreeMap<String, String>>,
}

fn pixel(rgba: &[u8], x: usize, z: usize) -> [u8; 4] {
    let offset = (z * TILE_SIZE + x) * 4;
    rgba[offset..offset + 4]
        .try_into()
        .expect("four-byte pixel")
}

async fn decoded(store: &impl TileStore, relative: &str) -> Vec<u8> {
    let bytes = store
        .read(Path::new(relative))
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("missing tile {relative}"));
    decode_rgba_png(&bytes).unwrap()
}

fn assert_path_set(actual: &[String], expected: &[&str]) {
    let actual: BTreeSet<&str> = actual.iter().map(String::as_str).collect();
    let expected: BTreeSet<&str> = expected.iter().copied().collect();
    assert_eq!(actual, expected);
}

async fn run_case_id(id: &str) -> tile_cases::CaseOutput {
    let (manifest, _) = load_manifest(Path::new(MANIFEST_PATH));
    let case = manifest.case(id);
    run_case(&manifest, case).await.expect(id)
}

#[tokio::test]
async fn catalog_manifest_has_schema_1_max_zoom_3_and_ten_cases() {
    let (manifest, _) = load_manifest(Path::new(MANIFEST_PATH));
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.max_zoom, MAX_ZOOM);
    assert_eq!(manifest.cases.len(), 10);
}

#[tokio::test]
async fn solid_origin_east_and_negative_paths_and_sample_pixels() {
    let origin = run_case_id("solid-origin").await;
    assert_path_set(
        &origin.paths,
        &["3/0_0.png", "2/0_0.png", "1/0_0.png", "0/0_0.png"],
    );
    let color = [0x11, 0x22, 0x33, 0xFF];
    assert_eq!(
        pixel(&decoded(&*origin.store, "3/0_0.png").await, 0, 0),
        color
    );
    assert_eq!(
        pixel(&decoded(&*origin.store, "2/0_0.png").await, 0, 0),
        color
    );
    assert_eq!(
        pixel(&decoded(&*origin.store, "1/0_0.png").await, 0, 0),
        color
    );
    assert_eq!(
        pixel(&decoded(&*origin.store, "0/0_0.png").await, 0, 0),
        color
    );

    let east = run_case_id("solid-east").await;
    assert_path_set(
        &east.paths,
        &["3/1_0.png", "2/0_0.png", "1/0_0.png", "0/0_0.png"],
    );
    let east_color = [0x44, 0x55, 0x66, 0xFF];
    assert_eq!(
        pixel(&decoded(&*east.store, "3/1_0.png").await, 0, 0),
        east_color
    );
    assert_eq!(
        pixel(&decoded(&*east.store, "2/0_0.png").await, 256, 0),
        east_color
    );
    assert_eq!(
        pixel(&decoded(&*east.store, "1/0_0.png").await, 128, 0),
        east_color
    );
    assert_eq!(
        pixel(&decoded(&*east.store, "0/0_0.png").await, 64, 0),
        east_color
    );

    let negative = run_case_id("solid-negative").await;
    assert_path_set(
        &negative.paths,
        &["3/-1_-1.png", "2/-1_-1.png", "1/-1_-1.png", "0/-1_-1.png"],
    );
    let negative_color = [0x77, 0x88, 0x99, 0xFF];
    assert_eq!(
        pixel(&decoded(&*negative.store, "3/-1_-1.png").await, 0, 0),
        negative_color
    );
    assert_eq!(
        pixel(&decoded(&*negative.store, "2/-1_-1.png").await, 256, 256),
        negative_color
    );
    assert_eq!(
        pixel(&decoded(&*negative.store, "1/-1_-1.png").await, 384, 384),
        negative_color
    );
    assert_eq!(
        pixel(&decoded(&*negative.store, "0/-1_-1.png").await, 448, 448),
        negative_color
    );
}

#[tokio::test]
async fn checker_step2_zoom1_origin_pixel_is_even_color() {
    let output = run_case_id("checker-step2").await;
    assert_eq!(
        pixel(&decoded(&*output.store, "2/0_0.png").await, 0, 0),
        [0xAA, 0, 0, 0xFF]
    );
}

#[tokio::test]
async fn sparse_chunk_00_does_not_paint_pixel_16_0() {
    let output = run_case_id("sparse-chunk-0-0").await;
    let native = decoded(&*output.store, "3/0_0.png").await;
    assert_eq!(pixel(&native, 16, 0), [0, 0, 0, 0]);
    assert_eq!(pixel(&native, 0, 0), [0x01, 0x02, 0x03, 0xFF]);
}

#[tokio::test]
async fn unset_vs_transparent_writes_clear_pixel_and_leaves_neighbor() {
    let output = run_case_id("unset-vs-transparent").await;
    let native = decoded(&*output.store, "3/0_0.png").await;
    assert_eq!(pixel(&native, 0, 0), [0, 0, 0, 0]);
    assert_eq!(pixel(&native, 1, 0), [7, 7, 7, 255]);
}

#[tokio::test]
async fn java_oracle_decoded_rgba_hashes_and_paths_match() {
    let hash_path = Path::new(JAVA_HASHES);
    assert!(
        hash_path.is_file(),
        "committed Java tile oracle hashes missing: {}",
        hash_path.display()
    );
    let java: JavaOracleHashes = serde_json::from_slice(&fs::read(hash_path).unwrap())
        .unwrap_or_else(|error| panic!("invalid {}: {error}", hash_path.display()));
    assert_eq!(java.schema_version, 1, "java-oracle-hashes schema_version");
    assert_eq!(java.max_zoom, MAX_ZOOM, "Java oracle max_zoom");

    let (manifest, manifest_bytes) = load_manifest(Path::new(MANIFEST_PATH));
    let rust = probe_catalog(&manifest, &manifest_bytes)
        .await
        .expect("rust probe");
    assert_eq!(
        rust.report.manifest_hash, java.manifest_hash,
        "tile catalog manifest hash rust={} java={}",
        rust.report.manifest_hash, java.manifest_hash
    );
    assert_eq!(
        rust.report.cases.len(),
        java.cases.len(),
        "case count rust={} java={}",
        rust.report.cases.len(),
        java.cases.len()
    );

    let dump_root = std::env::var_os("SQUAREMAP_JAVA_TILE_ORACLE").map(PathBuf::from);
    let mut mismatches = Vec::new();
    for rust_case in &rust.report.cases {
        let Some(java_paths) = java.cases.get(&rust_case.id) else {
            mismatches.push(format!("{} missing from committed Java hashes", rust_case.id));
            continue;
        };
        let rust_paths: BTreeSet<&str> = rust_case.paths.iter().map(String::as_str).collect();
        let java_path_set: BTreeSet<&str> = java_paths.keys().map(String::as_str).collect();
        if rust_paths != java_path_set {
            mismatches.push(format!(
                "{} path set rust={:?} java={:?}",
                rust_case.id, rust_paths, java_path_set
            ));
        }
        for (relative, rust_hash) in rust_case.paths.iter().zip(&rust_case.pixel_sha256) {
            let Some(java_hash) = java_paths.get(relative) else {
                mismatches.push(format!("{} {} missing Java hash", rust_case.id, relative));
                continue;
            };
            if rust_hash != java_hash {
                let detail = dump_root
                    .as_ref()
                    .map(|oracle| oracle.join(&rust_case.id).join(relative.replacen(".png", ".rgba", 1)))
                    .filter(|path| path.is_file())
                    .and_then(|path| {
                        let java_rgba = fs::read(path).ok()?;
                        if java_rgba.len() != TILE_RGBA_BYTES {
                            return Some(format!(
                                " Java rgba length {} expected {TILE_RGBA_BYTES}",
                                java_rgba.len()
                            ));
                        }
                        rust.rgba(&rust_case.id, relative)
                            .map(|pixels| first_pixel_mismatch(pixels, &java_rgba))
                    })
                    .unwrap_or_default();
                mismatches.push(format!(
                    "{} {} rust={rust_hash} java={java_hash}{detail}",
                    rust_case.id, relative
                ));
            }
        }
    }
    for id in java.cases.keys() {
        if !rust.report.cases.iter().any(|case| case.id == *id) {
            mismatches.push(format!("{id} present in Java hashes but missing from Rust probe"));
        }
    }

    if !mismatches.is_empty() {
        eprintln!(
            "Java/Rust tile mismatches ({}):\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }
    assert!(
        mismatches.is_empty(),
        "Java/Rust tile hash or path mismatches: {}",
        mismatches.join("; ")
    );
}

fn first_pixel_mismatch(rust_rgba: &[u8], java_rgba: &[u8]) -> String {
    let len = rust_rgba.len().min(java_rgba.len());
    for offset in (0..len).step_by(4) {
        let rust_px = &rust_rgba[offset..offset + 4];
        let java_px = java_rgba.get(offset..offset + 4).unwrap_or(&[]);
        if rust_px != java_px {
            let index = offset / 4;
            let x = index % TILE_SIZE;
            let z = index / TILE_SIZE;
            return format!(" first pixel ({x},{z}) rust={rust_px:?} java={java_px:?}");
        }
    }
    if rust_rgba.len() != java_rgba.len() {
        format!(" length rust={} java={}", rust_rgba.len(), java_rgba.len())
    } else {
        String::new()
    }
}
