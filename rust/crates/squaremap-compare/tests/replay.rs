use squaremap_compare::{compare::compare_output, recording::{decode, encode, encode_checked, Frame, Recording}, report::write_report};
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
fn recording_rejects_non_monotonic_timestamps_in_wire_data() {
    let recording = Recording {
        protocol_version: 1,
        frames: vec![
            Frame { monotonic_nanos: 9, direction: 0, bytes: vec![1] },
            Frame { monotonic_nanos: 10, direction: 1, bytes: vec![2] },
        ],
    };
    let mut invalid_timestamp = encode(&recording);
    invalid_timestamp[10..18].copy_from_slice(&11_u64.to_le_bytes());
    assert!(decode(&invalid_timestamp).is_err());
    let mut invalid_direction = encode(&Recording {
        protocol_version: 1,
        frames: vec![Frame { monotonic_nanos: 1, direction: 0, bytes: vec![1] }],
    });
    invalid_direction[18] = 2;
    assert!(decode(&invalid_direction).is_err());
}

#[test]
fn checked_encoding_rejects_invalid_metadata_before_serializing() {
    let invalid = Recording {
        protocol_version: 1,
        frames: vec![Frame { monotonic_nanos: 1, direction: 2, bytes: vec![1] }],
    };
    assert!(encode_checked(&invalid).is_err());
}
#[test]
fn checked_encoding_rejects_oversized_payloads() {
    let oversized_frame = Recording {
        protocol_version: 1,
        frames: vec![Frame { monotonic_nanos: 1, direction: 0, bytes: vec![0; 67_108_865] }],
    };
    assert!(encode_checked(&oversized_frame).is_err());

    let oversized_recording = Recording {
        protocol_version: 1,
        frames: vec![
            Frame { monotonic_nanos: 1, direction: 0, bytes: vec![0; 67_108_864] },
            Frame { monotonic_nanos: 2, direction: 1, bytes: vec![0; 67_108_865] },
        ],
    };
    assert!(encode_checked(&oversized_recording).is_err());
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
fn live_normalization_removes_only_volatile_timestamps() {
    let root = tempfile::tempdir().unwrap();
    let left = root.path().join("left");
    let right = root.path().join("right");
    fs::create_dir_all(&left).unwrap();
    fs::create_dir_all(&right).unwrap();
    fs::write(left.join("markers.json"), br#"[{"id":"layer","timestamp":1,"markers":[{"type":"polyline","points":[{"x":1,"z":2},{"x":3,"z":4}]}]}]"#).unwrap();
    fs::write(right.join("markers.json"), br#"[{"id":"layer","timestamp":2,"markers":[{"type":"polyline","points":[{"x":1,"z":2},{"x":3,"z":4}]}]}]"#).unwrap();

    let report = squaremap_compare::compare::compare_output_normalized(&left, &right).unwrap();
    assert_eq!(report.mismatch_count, 0);
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

#[test]
fn report_accepts_relative_output_path_without_parent_directory() {
    let root = tempfile::tempdir().unwrap();
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(root.path()).unwrap();
    let value = squaremap_compare::compare::ComparisonReport {
        compared_paths: 1,
        mismatch_count: 0,
        mismatches: vec![],
    };
    write_report("report.json", &value).unwrap();
    assert!(root.path().join("report.json").exists());
    std::env::set_current_dir(previous).unwrap();
}

#[test]
fn replay_output_round_trips_recording_bytes() {
    let root = tempfile::tempdir().unwrap();
    let recording = Recording {
        protocol_version: 1,
        frames: vec![
            Frame { monotonic_nanos: 3, direction: 0, bytes: vec![1, 2] },
            Frame { monotonic_nanos: 9, direction: 1, bytes: vec![4, 5, 6] },
        ],
    };
    let input = root.path().join("input.rec");
    let output = root.path().join("output.rec");
    fs::write(&input, encode(&recording)).unwrap();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_squaremap-compare"))
        .args(["replay", "--recording"])
        .arg(&input)
        .args(["--speed", "0", "--output"])
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(decode(&fs::read(output).unwrap()).unwrap(), recording);
}

#[test]
fn replay_output_refuses_overwrite() {
    let root = tempfile::tempdir().unwrap();
    let recording = Recording { protocol_version: 1, frames: vec![] };
    let input = root.path().join("input.rec");
    let output = root.path().join("output.rec");
    fs::write(&input, encode(&recording)).unwrap();
    fs::write(&output, b"existing").unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_squaremap-compare"))
        .args(["replay", "--recording"])
        .arg(&input)
        .args(["--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(fs::read(output).unwrap(), b"existing");
}

#[test]
fn indexed_png_matches_equivalent_rgba_png() {
    let root = tempfile::tempdir().unwrap();
    let left = root.path().join("left");
    let right = root.path().join("right");
    fs::create_dir_all(&left).unwrap();
    fs::create_dir_all(&right).unwrap();
    fs::write(left.join("tile.png"), indexed_png()).unwrap();
    fs::write(right.join("tile.png"), png_bytes([255, 0, 0, 255])).unwrap();
    assert_eq!(compare_output(&left, &right).unwrap().mismatch_count, 0);
}

fn indexed_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_palette(vec![255, 0, 0]);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&[0]).unwrap();
    drop(writer);
    bytes
}

fn png_bytes(pixel: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
    encoder.set_color(png::ColorType::Rgba); encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap(); writer.write_image_data(&pixel).unwrap(); drop(writer); bytes
}
