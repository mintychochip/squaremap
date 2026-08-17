use png::Decoder;
use serde_json::Value;
use std::{collections::BTreeSet, fs, io::Cursor, path::Path};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Mismatch { pub path: String, pub detail: String }
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ComparisonReport { pub compared_paths: usize, pub mismatch_count: usize, pub mismatches: Vec<Mismatch> }

pub fn compare_output(java: impl AsRef<Path>, rust: impl AsRef<Path>) -> std::io::Result<ComparisonReport> {
    compare_roots(java.as_ref(), rust.as_ref(), false)
}

/// Compares live snapshots while ignoring only marker timestamps, which are wall-clock values.
pub fn compare_output_normalized(java: impl AsRef<Path>, rust: impl AsRef<Path>) -> std::io::Result<ComparisonReport> {
    compare_roots(java.as_ref(), rust.as_ref(), true)
}

fn compare_roots(java: &Path, rust: &Path, normalize_timestamps: bool) -> std::io::Result<ComparisonReport> {
    let left = collect_files(java)?;
    let right = collect_files(rust)?;
    let paths: BTreeSet<_> = left.keys().chain(right.keys()).cloned().collect();
    let mut mismatches = Vec::new();
    for path in &paths {
        match (left.get(path), right.get(path)) {
            (Some(a), Some(b)) if path.ends_with(".json") => {
                let mut av: Value = serde_json::from_slice(a).map_err(invalid_json)?;
                let mut bv: Value = serde_json::from_slice(b).map_err(invalid_json)?;
                if normalize_timestamps && (path == "markers.json" || path.ends_with("/markers.json")) {
                    strip_marker_timestamps(&mut av);
                    strip_marker_timestamps(&mut bv);
                }
                if av != bv { mismatches.push(Mismatch { path: path.clone(), detail: "JSON values differ".into() }); }
            }
            (Some(a), Some(b)) if path.ends_with(".png") => {
                if decode_png(a)? != decode_png(b)? {
                    mismatches.push(Mismatch { path: path.clone(), detail: "RGBA pixels differ".into() });
                }
            }
            (Some(a), Some(b)) if a != b => mismatches.push(Mismatch { path: path.clone(), detail: "bytes differ".into() }),
            (Some(_), None) => mismatches.push(Mismatch { path: path.clone(), detail: "missing from Rust output".into() }),
            (None, Some(_)) => mismatches.push(Mismatch { path: path.clone(), detail: "extra in Rust output".into() }),
            _ => {}
        }
    }
    Ok(ComparisonReport { compared_paths: paths.len(), mismatch_count: mismatches.len(), mismatches })
}

fn strip_marker_timestamps(value: &mut Value) {
    let Some(layers) = value.as_array_mut() else {
        return;
    };
    for layer in layers {
        if let Some(object) = layer.as_object_mut() {
            object.remove("timestamp");
        }
    }
}
fn invalid_json(e: serde_json::Error) -> std::io::Error { std::io::Error::new(std::io::ErrorKind::InvalidData, e) }
fn collect_files(root: &Path) -> std::io::Result<std::collections::BTreeMap<String, Vec<u8>>> {
    let mut out = std::collections::BTreeMap::new(); collect(root, root, &mut out)?; Ok(out)
}
fn collect(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out)?;
        } else {
            let relative = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.insert(relative, fs::read(path)?);
        }
    }
    Ok(())
}
pub(crate) fn decode_png(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut decoder = Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(png_error)?;
    let size = reader.output_buffer_size().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "PNG has no output buffer"))?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(png_error)?;
    let mut rgba = Vec::with_capacity(8 + info.width as usize * info.height as usize * 4);
    rgba.extend_from_slice(&info.width.to_le_bytes());
    rgba.extend_from_slice(&info.height.to_le_bytes());
    for pixel in buf[..info.buffer_size()].chunks_exact(info.color_type.samples() as usize) {
        match info.color_type {
            png::ColorType::Rgb => rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]),
            png::ColorType::Rgba => rgba.extend_from_slice(pixel),
            png::ColorType::Grayscale => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], 255]),
            png::ColorType::GrayscaleAlpha => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]),
            png::ColorType::Indexed => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "indexed PNG expansion failed")),
        }
    }
    Ok(rgba)
}
fn png_error(e: png::DecodingError) -> std::io::Error { std::io::Error::new(std::io::ErrorKind::InvalidData, e) }
