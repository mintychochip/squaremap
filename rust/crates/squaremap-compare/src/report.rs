use crate::compare::ComparisonReport;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, io, path::Path};

pub fn write_report(path: impl AsRef<Path>, report: &ComparisonReport) -> io::Result<()> {
    let path = path.as_ref();
    let bytes = serde_json::to_vec_pretty(report).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) { fs::create_dir_all(parent)?; }
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    std::io::Write::write_all(&mut file, &bytes)
}

pub fn migration_report_is_complete(report: &Value, required_gates: &[&str]) -> bool {
    if report.get("verdict").and_then(Value::as_str) != Some("complete") { return false; }
    let Some(verified) = report.get("verified").and_then(Value::as_object) else { return false; };
    if !required_gates.iter().all(|g| verified.get(*g).and_then(|e| e.get("passed")).and_then(Value::as_bool) == Some(true)) { return false; }
    report.get("blocked").map_or(true, |b| b.as_object().is_some_and(serde_json::Map::is_empty))
}

const LIVE_KINDS: &[&str] = &["live_paper", "loader", "performance", "remote", "canary", "observation"];
const SCOPED_KINDS: &[&str] = &["live_paper", "performance", "remote", "canary", "observation"];

pub fn validate_evidence(path: impl AsRef<Path>, matrix_path: impl AsRef<Path>, current_commit: &str, current_dirty: bool) -> Result<(), String> {
    let value: Value = serde_json::from_slice(&fs::read(path).map_err(|e| format!("read evidence: {e}"))?).map_err(|e| format!("malformed evidence: {e}"))?;
    let root = value.as_object().ok_or("evidence must be an object")?;
    for key in ["commit_sha", "dirty_tree", "timestamp", "toolchain", "command", "exit_code", "input_hashes", "config_hashes", "comparator_version", "raw_artifact_hashes", "records"] { if !root.contains_key(key) { return Err(format!("missing common field {key}")); } }
    if root.get("commit_sha").and_then(Value::as_str) != Some(current_commit) { return Err("stale commit SHA".into()); }
    if root.get("dirty_tree").and_then(Value::as_bool) != Some(current_dirty) { return Err("dirty-tree mismatch".into()); }
    if root.get("exit_code").and_then(Value::as_i64) != Some(0) { return Err("evidence command failed".into()); }
    if !root.get("input_hashes").is_some_and(nonempty_object) || !root.get("config_hashes").is_some_and(nonempty_object) || !root.get("raw_artifact_hashes").is_some_and(nonempty_object) { return Err("missing required hashes".into()); }
    let matrix: Value = serde_json::from_slice(&fs::read(matrix_path).map_err(|e| format!("read matrix: {e}"))?).map_err(|e| format!("malformed matrix: {e}"))?;
    let expected = matrix.get("rows").and_then(Value::as_array).ok_or("matrix rows missing")?;
    let mut expected_ids = BTreeSet::new();
    for row in expected { let id = row.get("id").and_then(Value::as_str).ok_or("matrix row id missing")?; if !expected_ids.insert(id) { return Err("duplicate matrix row id".into()); } }
    let records = root.get("records").and_then(Value::as_array).ok_or("records missing")?;
    let mut seen = BTreeSet::new();
    for record in records {
        let obj = record.as_object().ok_or("record must be object")?;
        let id = obj.get("matrix_row_id").and_then(Value::as_str).ok_or("record matrix row missing")?;
        if !expected_ids.contains(id) { return Err(format!("unknown matrix row {id}")); }
        if !seen.insert(id) { return Err(format!("duplicate evidence row {id}")); }
        for key in ["evidence_kind", "status", "dependencies", "artifacts", "mismatches"] { if !obj.contains_key(key) { return Err(format!("missing record field {key}")); } }
        if obj.get("status").and_then(Value::as_str) != Some("passed") { return Err(format!("incomplete status {id}")); }
        if obj.get("dependencies").and_then(Value::as_array).is_none_or(|a| a.is_empty()) { return Err(format!("missing dependencies {id}")); }
        if obj.get("mismatches").and_then(Value::as_array).is_none_or(|a| !a.is_empty()) { return Err(format!("mismatches are not empty {id}")); }
        let kind = obj.get("evidence_kind").and_then(Value::as_str).ok_or("evidence kind missing")?;
        if kind == "fixture" && LIVE_KINDS.iter().any(|k| expected.iter().any(|r| r.get("id").and_then(Value::as_str) == Some(id) && r.get("evidence_kind").and_then(Value::as_str) == Some(k))) { return Err("fixture evidence cannot satisfy scoped row".into()); }
        if SCOPED_KINDS.contains(&kind) { validate_scoped(obj, kind)?; }
        if obj.get("ownership").and_then(Value::as_str) != Some("sole_writer_proven") { return Err(format!("unproven ownership {id}")); }
        if obj.get("recorder").and_then(Value::as_object).is_some_and(|r| r.values().any(|v| v.as_bool() == Some(false))) { return Err("recorder integrity failure".into()); }
    }
    if seen != expected_ids { return Err("matrix inventory incomplete".into()); }
    verify_hashes(root.get("raw_artifact_hashes").unwrap())?;
    Ok(())
}

fn nonempty_object(v: &Value) -> bool { v.as_object().is_some_and(|o| !o.is_empty()) }
fn validate_scoped(obj: &serde_json::Map<String, Value>, kind: &str) -> Result<(), String> {
    let required: &[&str] = match kind { "live_paper" => &["pids", "process_generations", "roots", "ports", "scenario_hash", "quiescence_hash", "sample_count", "metric_sources"], "performance" => &["sample_count", "ci_method", "metric_sources"], "remote" => &["immutable_url", "digest", "transcript", "target_attestation", "execution_result"], "canary" | "observation" => &["digest", "process_starts"], _ => &[] };
    for key in required { if !obj.contains_key(*key) { return Err(format!("missing {kind} field {key}")); } }
    if obj.get("sample_count").and_then(Value::as_u64).is_some_and(|n| n == 0) { return Err("missing samples".into()); }
    Ok(())
}
fn verify_hashes(v: &Value) -> Result<(), String> {
    let Some(map) = v.as_object() else { return Err("artifact hashes must be object".into()); };
    for (path, expected) in map { let bytes = fs::read(path).map_err(|e| format!("artifact {path}: {e}"))?; let digest = hex::encode(Sha256::digest(bytes)); if expected.as_str() != Some(&digest) { return Err(format!("artifact hash mismatch {path}")); } }
    Ok(())
}

#[cfg(test)]
mod evidence_tests {
    use super::validate_evidence;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::{fs, path::Path};
    use tempfile::tempdir;

    fn fixture(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let artifact = root.join("artifact.bin");
        fs::write(&artifact, b"proof").unwrap();
        let matrix = root.join("matrix.json");
        fs::write(
            &matrix,
            serde_json::to_vec(&json!({
                "rows": [{"id": "transport-security", "evidence_kind": "deterministic"}]
            }))
            .unwrap(),
        )
        .unwrap();
        let evidence = root.join("evidence.json");
        let digest = hex::encode(Sha256::digest(b"proof"));
        fs::write(
            &evidence,
            serde_json::to_vec(&json!({
                "commit_sha": "abc123",
                "dirty_tree": true,
                "timestamp": "2026-08-14T00:00:00Z",
                "toolchain": "test",
                "command": "fixture",
                "exit_code": 0,
                "input_hashes": {"input": "hash"},
                "config_hashes": {"config": "hash"},
                "comparator_version": "test",
                "raw_artifact_hashes": {artifact.to_string_lossy(): digest},
                "records": [{
                    "matrix_row_id": "transport-security",
                    "evidence_kind": "deterministic",
                    "status": "passed",
                    "dependencies": ["protocol-fixtures"],
                    "artifacts": [artifact],
                    "mismatches": [],
                    "ownership": "sole_writer_proven"
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        (evidence, matrix)
    }

    #[test]
    fn accepts_complete_current_evidence() {
        let dir = tempdir().unwrap();
        let (evidence, matrix) = fixture(dir.path());
        assert_eq!(validate_evidence(evidence, matrix, "abc123", true), Ok(()));
    }

    #[test]
    fn rejects_incomplete_matrix_inventory() {
        let dir = tempdir().unwrap();
        let (evidence, matrix) = fixture(dir.path());
        let mut value: Value = serde_json::from_slice(&fs::read(&matrix).unwrap()).unwrap();
        value["rows"].as_array_mut().unwrap().push(json!({
            "id": "render-pixels",
            "evidence_kind": "deterministic"
        }));
        fs::write(&matrix, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            validate_evidence(evidence, matrix, "abc123", true),
            Err("matrix inventory incomplete".into())
        );
    }

    #[test]
    fn rejects_stale_or_tampered_evidence() {
        let dir = tempdir().unwrap();
        let (evidence, matrix) = fixture(dir.path());
        assert_eq!(
            validate_evidence(&evidence, &matrix, "different", true),
            Err("stale commit SHA".into())
        );
        let value: Value = serde_json::from_slice(&fs::read(&evidence).unwrap()).unwrap();
        let artifact = value["records"][0]["artifacts"][0].as_str().unwrap();
        fs::write(artifact, b"tampered").unwrap();
        assert!(validate_evidence(evidence, matrix, "abc123", true)
            .unwrap_err()
            .contains("artifact hash mismatch"));
    }
}
