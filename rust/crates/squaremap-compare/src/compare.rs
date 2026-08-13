use png::Decoder;
use serde_json::Value;
use std::{collections::BTreeSet, fs, io::Cursor, path::Path};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Mismatch { pub path: String, pub detail: String }
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ComparisonReport { pub compared_paths: usize, pub mismatch_count: usize, pub mismatches: Vec<Mismatch> }

pub fn compare_output(java: impl AsRef<Path>, rust: impl AsRef<Path>) -> std::io::Result<ComparisonReport> {
    let left = collect_files(java.as_ref())?;
    let right = collect_files(rust.as_ref())?;
    let paths: BTreeSet<_> = left.keys().chain(right.keys()).cloned().collect();
    let mut mismatches = Vec::new();
    for path in &paths {
        match (left.get(path), right.get(path)) {
            (Some(a), Some(b)) if path.ends_with(".json") => {
                let av: Value = serde_json::from_slice(a).map_err(invalid_json)?;
                let bv: Value = serde_json::from_slice(b).map_err(invalid_json)?;
                if av != bv { mismatches.push(Mismatch { path: path.clone(), detail: "JSON values differ".into() }); }
            }
            (Some(a), Some(b)) if path.ends_with(".png") => {
                if decode_png(a)? != decode_png(b)? { mismatches.push(Mismatch { path: path.clone(), detail: "RGBA pixels differ".into() }); }
            }
            (Some(a), Some(b)) if a != b => mismatches.push(Mismatch { path: path.clone(), detail: "bytes differ".into() }),
            (Some(_), None) => mismatches.push(Mismatch { path: path.clone(), detail: "missing from Rust output".into() }),
            (None, Some(_)) => mismatches.push(Mismatch { path: path.clone(), detail: "extra in Rust output".into() }),
            _ => {}
        }
    }
    Ok(ComparisonReport { compared_paths: paths.len(), mismatch_count: mismatches.len(), mismatches })
}
fn invalid_json(e: serde_json::Error) -> std::io::Error { std::io::Error::new(std::io::ErrorKind::InvalidData, e) }
fn collect_files(root: &Path) -> std::io::Result<std::collections::BTreeMap<String, Vec<u8>>> {
    let mut out = std::collections::BTreeMap::new(); collect(root, root, &mut out)?; Ok(out)
}
fn collect(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? { let entry = entry?; let p = entry.path(); if p.is_dir() { collect(root, &p, out)?; } else { let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"); out.insert(rel, fs::read(p)?); } } Ok(())
}
fn decode_png(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let decoder = Decoder::new(Cursor::new(bytes)); let mut reader = decoder.read_info().map_err(png_error)?; let size = reader.output_buffer_size().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "PNG has no output buffer"))?; let mut buf = vec![0; size]; let info = reader.next_frame(&mut buf).map_err(png_error)?; Ok(buf[..info.buffer_size()].to_vec())
}
fn png_error(e: png::DecodingError) -> std::io::Error { std::io::Error::new(std::io::ErrorKind::InvalidData, e) }
