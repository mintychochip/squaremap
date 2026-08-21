use squaremap_compare::ab::{AbReport, generate_ab_report};
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

fn sample_json(
    backend: &str,
    workload: &str,
    checksum: i64,
    measured_passes: u64,
    pass_nanos: &[u64],
) -> serde_json::Value {
    serde_json::json!({
        "backend": backend,
        "workload": workload,
        "case_count": 2,
        "warmup_passes": 10,
        "measured_passes": measured_passes,
        "elapsed_nanos": 1000,
        "items_per_second": 1.0,
        "checksum": checksum,
        "manifest_hash": "abc",
        "pass_nanos": pass_nanos,
    })
}

fn write_sample(dir: &Path, name: &str, value: &serde_json::Value) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    path
}

fn cli(java: &Path, rust: &Path, output: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_squaremap-compare"))
        .args([
            "ab-report",
            "--java",
            java.to_str().unwrap(),
            "--rust",
            rust.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .expect("ab-report binary")
}

#[test]
fn matching_checksums_write_graphs_and_exit_zero() {
    let dir = tempdir().unwrap();
    let java = write_sample(
        dir.path(),
        "java.json",
        &sample_json("java", "chunk-render-v2", 7, 3, &[10, 20, 30]),
    );
    let rust = write_sample(
        dir.path(),
        "rust.json",
        &sample_json("rust", "chunk-render-v2", 7, 3, &[10, 20, 30]),
    );
    let output = dir.path().join("report");
    let result = cli(&java, &rust, &output);
    assert_eq!(
        result.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&result.stderr)
    );

    assert!(output.join("ab-report.json").exists());
    assert!(output.join("throughput.svg").exists());
    assert!(output.join("pass-times.svg").exists());
    assert!(output.join("index.html").exists());

    let report: AbReport =
        serde_json::from_slice(&fs::read(output.join("ab-report.json")).unwrap()).unwrap();
    assert!(report.passed);
    assert_eq!(report.workload, "chunk-render-v2");
    assert_eq!(report.java.median_nanos, 20);
    assert_eq!(report.rust.median_nanos, 20);

    let svg = fs::read_to_string(output.join("throughput.svg")).unwrap();
    assert!(svg.contains("chunk-render-v2"));
    assert!(svg.contains("java"));
    assert!(svg.contains("rust"));
}

#[test]
fn differing_checksums_write_report_but_exit_one() {
    let dir = tempdir().unwrap();
    let java = write_sample(
        dir.path(),
        "java.json",
        &sample_json("java", "chunk-render-v2", 1, 3, &[10, 20, 30]),
    );
    let rust = write_sample(
        dir.path(),
        "rust.json",
        &sample_json("rust", "chunk-render-v2", 2, 3, &[10, 20, 30]),
    );
    let output = dir.path().join("report");
    let result = cli(&java, &rust, &output);
    assert_eq!(
        result.status.code(),
        Some(1),
        "stderr={}",
        String::from_utf8_lossy(&result.stderr)
    );

    let report: AbReport =
        serde_json::from_slice(&fs::read(output.join("ab-report.json")).unwrap()).unwrap();
    assert!(!report.passed);
    let html = fs::read_to_string(output.join("index.html")).unwrap();
    assert!(html.contains("CHECKSUM MISMATCH"));
    assert!(html.contains("CHECKSUM MISMATCH — do not publish a speedup"));
}

#[test]
fn differing_workload_exits_two() {
    let dir = tempdir().unwrap();
    let java = write_sample(
        dir.path(),
        "java.json",
        &sample_json("java", "chunk-render-v2", 7, 3, &[10, 20, 30]),
    );
    let rust = write_sample(
        dir.path(),
        "rust.json",
        &sample_json("rust", "pyramid-png-v2", 7, 3, &[10, 20, 30]),
    );
    let output = dir.path().join("report");
    let result = cli(&java, &rust, &output);
    assert_eq!(result.status.code(), Some(2));
    assert!(!output.join("ab-report.json").exists());
}

#[test]
fn pass_nanos_length_mismatch_is_error() {
    let dir = tempdir().unwrap();
    let java = write_sample(
        dir.path(),
        "java.json",
        &sample_json("java", "chunk-render-v2", 7, 3, &[10, 20, 30]),
    );
    let rust = write_sample(
        dir.path(),
        "rust.json",
        &sample_json("rust", "chunk-render-v2", 7, 3, &[10, 20]),
    );
    let output = dir.path().join("report");
    let err = generate_ab_report(&java, &rust, &output).expect_err("length mismatch");
    assert!(err.to_string().contains("pass_nanos"));
    let result = cli(&java, &rust, &output);
    assert_eq!(result.status.code(), Some(2));
    assert!(!output.join("ab-report.json").exists());
}

#[test]
fn median_uses_percentile_of_pass_nanos() {
    let dir = tempdir().unwrap();
    let java = write_sample(
        dir.path(),
        "java.json",
        &sample_json("java", "chunk-render-v2", 7, 3, &[10, 20, 30]),
    );
    let rust = write_sample(
        dir.path(),
        "rust.json",
        &sample_json("rust", "chunk-render-v2", 7, 3, &[30, 10, 20]),
    );
    let output = dir.path().join("report");
    let report = generate_ab_report(&java, &rust, &output).unwrap();
    assert_eq!(report.java.median_nanos, 20);
    assert_eq!(report.rust.median_nanos, 20);
    assert_eq!(report.java.items_per_second_median, 100_000_000.0);
    assert_eq!(report.rust.items_per_second_median, 100_000_000.0);
    assert_eq!(report.speedup, 1.0);
}
