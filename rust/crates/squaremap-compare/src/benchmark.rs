use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkReport {
    pub iterations: u64,
    pub elapsed_nanos: u128,
    pub items_per_second: f64,
    pub threshold_items_per_second: f64,
    pub compared_paths: usize,
    pub mismatches: usize,
    pub passed: bool,
}

pub fn run(iterations: u64, threshold: f64) -> BenchmarkReport {
    let iterations = iterations.max(1);
    let start = Instant::now();
    let mut checksum = 0u64;
    for index in 0..iterations {
        checksum = checksum.wrapping_add(index.rotate_left((index % 63) as u32));
    }
    std::hint::black_box(checksum);
    let elapsed_nanos = start.elapsed().as_nanos().max(1);
    let items_per_second = iterations as f64 * 1_000_000_000.0 / elapsed_nanos as f64;
    BenchmarkReport {
        iterations,
        elapsed_nanos,
        items_per_second,
        threshold_items_per_second: threshold,
        compared_paths: 0,
        mismatches: 0,
        passed: items_per_second >= threshold,
    }
}

pub fn run_output(
    java_root: impl AsRef<Path>,
    rust_root: impl AsRef<Path>,
    iterations: u64,
    threshold: f64,
) -> std::io::Result<BenchmarkReport> {
    let iterations = iterations.max(1);
    let start = Instant::now();
    let mut compared_paths = 0usize;
    let mut mismatches = 0usize;
    for _ in 0..iterations {
        let report = crate::compare::compare_output(java_root.as_ref(), rust_root.as_ref())?;
        compared_paths = report.compared_paths;
        mismatches = report.mismatch_count;
    }
    let elapsed_nanos = start.elapsed().as_nanos().max(1);
    let items_per_second = iterations as f64 * 1_000_000_000.0 / elapsed_nanos as f64;
    Ok(BenchmarkReport {
        iterations,
        elapsed_nanos,
        items_per_second,
        threshold_items_per_second: threshold,
        compared_paths,
        mismatches,
        passed: mismatches == 0 && items_per_second >= threshold,
    })
}

#[cfg(test)]
mod tests {
    use super::run;

    #[test]
    fn report_is_deterministically_shaped_and_has_verdict() {
        let report = run(100, 0.0);
        assert_eq!(report.iterations, 100);
        assert!(report.elapsed_nanos > 0);
        assert!(report.passed);
    }
}
