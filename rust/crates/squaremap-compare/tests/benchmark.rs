use squaremap_compare::benchmark::{percentile, run};
#[test]
fn benchmark_honors_nonzero_threshold_verdict() {
    let report = run(10_000, f64::MAX);
    assert_eq!(report.iterations, 10_000);
    assert!(!report.passed);
    assert!(report.elapsed_nanos > 0);
}

#[test]
fn percentile_uses_interpolated_rank_and_rejects_invalid_input() {
    let mut samples = [40, 10, 30, 20];
    assert_eq!(percentile(&mut samples, 0.0), Some(10));
    assert_eq!(percentile(&mut samples, 0.5), Some(25));
    assert_eq!(percentile(&mut samples, 1.0), Some(40));
    assert_eq!(percentile(&mut samples, f64::NAN), None);
    assert_eq!(percentile(&mut [], 0.5), None);
}

#[test]
fn benchmark_output_rejects_zero_iterations() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let error = squaremap_compare::benchmark::run_output(
        temporary.path(),
        temporary.path(),
        0,
        0.0,
    )
    .expect_err("zero iterations must not produce a passing zero-work report");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn benchmark_cli_rejects_zero_iterations_in_synthetic_mode() {
    let binary = env!("CARGO_BIN_EXE_squaremap-compare");
    let output = std::process::Command::new(binary)
        .args(["benchmark", "--iterations", "0", "--threshold", "1"])
        .output()
        .expect("benchmark binary");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("iterations"));
}

#[test]
#[should_panic(expected = "benchmark iterations must be greater than zero")]
fn synthetic_benchmark_rejects_zero_iterations() {
    let _ = run(0, 0.0);
}

#[test]
fn benchmark_cli_rejects_non_finite_threshold() {
    let binary = env!("CARGO_BIN_EXE_squaremap-compare");
    let output = std::process::Command::new(binary)
        .args(["benchmark", "--iterations", "1", "--threshold", "NaN"])
        .output()
        .expect("benchmark binary");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("finite"));
}

#[test]
fn benchmark_output_rejects_invalid_thresholds() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    for threshold in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = squaremap_compare::benchmark::run_output(
            temporary.path(),
            temporary.path(),
            1,
            threshold,
        )
        .expect_err("invalid threshold must be rejected");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}

#[test]
fn benchmark_cli_rejects_zero_and_infinite_thresholds() {
    let binary = env!("CARGO_BIN_EXE_squaremap-compare");
    for threshold in ["0", "inf", "-inf"] {
        let output = std::process::Command::new(binary)
            .args(["benchmark", "--iterations", "1", "--threshold", threshold])
            .output()
            .expect("benchmark binary");
        assert_eq!(output.status.code(), Some(2), "threshold={threshold}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("threshold"));
    }
}
