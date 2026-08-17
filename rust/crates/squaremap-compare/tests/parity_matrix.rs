use serde_json::Value;
use std::fs;
use std::path::Path;

#[test]
fn parity_matrix_has_no_unproven_deterministic_rows_with_verified_probe_evidence() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/superpowers/verification/rust-backend-parity-matrix.json");
    let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    for row in value["rows"].as_array().unwrap() {
        if row["evidence_kind"] == "deterministic" {
            let status = row["status"].as_str().unwrap();
            assert_ne!(status, "verified", "use verified_component for scoped evidence");
            assert!(status == "verified_component" || status == "unproven", "unexpected deterministic status {status}");
        }
        if row["status"] == "blocked_infrastructure" {
            assert!(!row["artifacts"].as_array().unwrap().is_empty(), "blocked row needs prerequisite evidence");
        }
    }
}
