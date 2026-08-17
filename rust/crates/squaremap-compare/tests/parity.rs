use squaremap_compare::parity::{run_manifest, write_parity_report, ParityProbeManifest, ProbeError};
use serde_json::json;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn manifest(value: serde_json::Value) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("manifest.json");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    (dir, path)
}

fn valid() -> serde_json::Value {
    json!({"schema_version":1,"scenario":"fixture","input_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","normalization":"none","outputs":[{"path":"markers.json","type":"json"}]})
}

#[test]
fn valid_manifest_loads() {
    let (_dir, path) = manifest(valid());
    assert_eq!(ParityProbeManifest::load(path).unwrap().scenario, "fixture");
}

#[test]
fn rejects_unknown_case() {
    let mut value = valid(); value["case_id"] = json!("unknown");
    let (_dir, path) = manifest(value);
    assert!(matches!(ParityProbeManifest::load(path), Err(ProbeError::Validation(_))));
}

#[test]
fn rejects_duplicate_path_and_unsupported_normalization() {
    let mut value = valid();
    value["normalization"] = json!("timestamps");
    value["outputs"] = json!([{"path":"a","type":"bytes"},{"path":"a","type":"bytes"}]);
    let (_dir, path) = manifest(value);
    assert!(ParityProbeManifest::load(path).is_err());
}

#[test]
fn rejects_path_escape_missing_hash_and_empty_outputs() {
    for (field, replacement) in [("input_hash", json!(null)), ("outputs", json!([]))] {
        let mut value = valid(); value[field] = replacement;
        let (_dir, path) = manifest(value);
        assert!(ParityProbeManifest::load(path).is_err());
    }
    let mut value = valid(); value["outputs"][0]["path"] = json!("../escape");
    let (_dir, path) = manifest(value);
    assert!(ParityProbeManifest::load(path).is_err());
}

#[test]
fn compares_json_semantically_and_rejects_invalid_json() {
    let (_manifest_dir, manifest_path) = manifest(valid());
    let fixture = ParityProbeManifest::load(manifest_path).unwrap();
    let java = tempdir().unwrap(); let rust = tempdir().unwrap();
    fs::write(java.path().join("markers.json"), br#"{"b":2,"a":1}"#).unwrap();
    fs::write(rust.path().join("markers.json"), br#"{"a":1,"b":2}"#).unwrap();
    assert!(run_manifest(&fixture, java.path(), rust.path()).unwrap().mismatches.is_empty());
    fs::write(rust.path().join("markers.json"), b"not json").unwrap();
    assert!(matches!(run_manifest(&fixture, java.path(), rust.path()), Err(ProbeError::Validation(_))));
}

#[test]
fn compares_png_by_decoded_rgba() {
    let mut value = valid(); value["outputs"] = json!([{"path":"tile.png","type":"png"}]);
    let (_manifest_dir, manifest_path) = manifest(value);
    let fixture = ParityProbeManifest::load(manifest_path).unwrap();
    let java = tempdir().unwrap(); let rust = tempdir().unwrap();
    fs::write(java.path().join("tile.png"), png_bytes([255,0,0,255])).unwrap();
    fs::write(rust.path().join("tile.png"), png_bytes([255,0,0,255])).unwrap();
    assert!(run_manifest(&fixture, java.path(), rust.path()).unwrap().mismatches.is_empty());
    fs::write(rust.path().join("tile.png"), png_bytes([0,0,255,255])).unwrap();
    assert_eq!(run_manifest(&fixture, java.path(), rust.path()).unwrap().mismatches.len(), 1);
}

#[test]
fn reports_missing_extra_and_refuses_overwrite() {
    let (_manifest_dir, manifest_path) = manifest(valid());
    let fixture = ParityProbeManifest::load(manifest_path).unwrap();
    let java = tempdir().unwrap(); let rust = tempdir().unwrap();
    fs::write(java.path().join("markers.json"), br#"{}"#).unwrap();
    fs::write(rust.path().join("extra.bin"), b"x").unwrap();
    let report = run_manifest(&fixture, java.path(), rust.path()).unwrap();
    assert!(!report.complete);
    assert!(report.missing_paths.iter().any(|p| p == "rust:markers.json"));
    assert_eq!(report.extra_paths, vec!["extra.bin"]);
    let path = java.path().join("report.json");
    write_parity_report(&path, &report).unwrap();
    assert!(write_parity_report(&path, &report).is_err());
}

#[test]
fn cli_returns_two_for_missing_manifest_or_root() {
    let binary = env!("CARGO_BIN_EXE_squaremap-compare");
    let output = Command::new(binary).args(["parity","--manifest","/missing","--java-root","/missing","--rust-root","/missing","--report","/tmp/report.json"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
}

fn png_bytes(pixel: [u8;4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
    encoder.set_color(png::ColorType::Rgba); encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap(); writer.write_image_data(&pixel).unwrap(); drop(writer); bytes
}
