use squaremap_compare::{compare::compare_output, recording::{decode, encode, Frame, Recording}, report::write_report};
use std::{fs, io::Write};

#[test]
fn recording_round_trip_rejects_truncation_and_protocol_versions() {
    let recording = Recording { protocol_version: 1, frames: vec![Frame { monotonic_nanos: 7, direction: 0, bytes: vec![1,2,3] }] };
    let encoded = encode(&recording);
    assert_eq!(decode(&encoded).unwrap(), recording);
    assert!(decode(&encoded[..encoded.len()-1]).is_err());
    let mut incompatible = encoded.clone(); incompatible[5] = 2;
    assert!(decode(&incompatible).is_err());
}

#[test]
fn compares_json_semantics_preserves_array_order_and_reports_missing_extra_paths() {
    let root = tempfile::tempdir().unwrap();
    let left = root.path().join("left"); let right = root.path().join("right");
    fs::create_dir_all(&left).unwrap(); fs::create_dir_all(&right).unwrap();
    fs::write(left.join("same.json"), br#"{"b":2,"a":1}"#).unwrap();
    fs::write(right.join("same.json"), br#"{"a":1,"b":2}"#).unwrap();
    fs::write(left.join("array.json"), br#"[1,2]"#).unwrap();
    fs::write(right.join("array.json"), br#"[2,1]"#).unwrap();
    fs::write(left.join("only-left.txt"), b"x").unwrap();
    fs::write(right.join("only-right.txt"), b"y").unwrap();
    let report = compare_output(&left, &right).unwrap();
    assert_eq!(report.compared_paths, 4);
    assert_eq!(report.mismatch_count, 3);
    assert!(report.mismatches.iter().any(|m| m.path == "array.json"));
    assert!(report.mismatches.iter().any(|m| m.path == "only-left.txt"));
    assert!(report.mismatches.iter().any(|m| m.path == "only-right.txt"));
}

#[test]
fn compares_png_pixels_after_decode() {
    let root = tempfile::tempdir().unwrap();
    let left = root.path().join("left"); let right = root.path().join("right");
    fs::create_dir_all(&left).unwrap(); fs::create_dir_all(&right).unwrap();
    fs::write(left.join("tile.png"), png_bytes([255, 0, 0, 255])).unwrap();
    fs::write(right.join("tile.png"), png_bytes([255, 0, 0, 255])).unwrap();
    assert_eq!(compare_output(&left, &right).unwrap().mismatch_count, 0);
    fs::write(right.join("tile.png"), png_bytes([0, 0, 255, 255])).unwrap();
    assert_eq!(compare_output(&left, &right).unwrap().mismatch_count, 1);
}

#[test]
fn report_refuses_overwrite() {
    let root = tempfile::tempdir().unwrap(); let path = root.path().join("report.json");
    let report = squaremap_compare::compare::ComparisonReport { compared_paths: 0, mismatch_count: 0, mismatches: vec![] };
    write_report(&path, &report).unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap(); file.write_all(b"sentinel").unwrap();
    assert!(write_report(&path, &report).is_err());
    assert!(fs::read(&path).unwrap().ends_with(b"sentinel"));
}

fn png_bytes(pixel: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
    encoder.set_color(png::ColorType::Rgba); encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap(); writer.write_image_data(&pixel).unwrap(); drop(writer); bytes
}
