use squaremap_compare::benchmark::run;

#[test]
fn benchmark_honors_nonzero_threshold_verdict() {
    let report = run(10_000, f64::MAX);
    assert_eq!(report.iterations, 10_000);
    assert!(!report.passed);
    assert!(report.elapsed_nanos > 0);
}
