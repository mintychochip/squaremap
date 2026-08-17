use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ChecksumEntry { path: String, sha256: String }
#[derive(Debug, Deserialize)]
struct EvidenceManifest { expected_files: Vec<String>, completed_steps: Vec<String>, checksums: Vec<ChecksumEntry>, mismatch_count: u64, quiescent: bool, processes_complete: bool }

pub fn validate_bundle(path: impl AsRef<Path>) -> Result<(), String> {
    let root = path.as_ref();
    let manifest_path = root.join("manifest.json");
    let bytes = fs::read(&manifest_path).map_err(|e| format!("missing evidence manifest: {e}"))?;
    let manifest: EvidenceManifest = serde_json::from_slice(&bytes).map_err(|e| format!("invalid evidence manifest: {e}"))?;
    if manifest.expected_files.is_empty() { return Err("evidence expected_files is empty".into()); }
    if manifest.mismatch_count != 0 { return Err("evidence contains mismatches".into()); }
    if !manifest.quiescent { return Err("evidence is not quiescent".into()); }
    if !manifest.processes_complete { return Err("evidence process records incomplete".into()); }
    let completed: std::collections::BTreeSet<_> = manifest.completed_steps.iter().collect();
    if completed.len() != manifest.completed_steps.len() { return Err("duplicate scenario step".into()); }
    let mut hashes = BTreeMap::new();
    for entry in manifest.checksums {
        if !hashes.insert(entry.path.clone(), entry.sha256.clone()).is_none() { return Err("duplicate evidence checksum path".into()); }
        let file = root.join(&entry.path);
        if !file.starts_with(root) { return Err("evidence path escapes bundle".into()); }
        let data = fs::read(&file).map_err(|e| format!("missing evidence file {}: {e}", entry.path))?;
        let digest = hex::encode(Sha256::digest(data));
        if digest != entry.sha256.to_ascii_lowercase() { return Err(format!("evidence hash mismatch: {}", entry.path)); }
    }
    for expected in manifest.expected_files {
        if !hashes.contains_key(&expected) { return Err(format!("missing checksum coverage: {expected}")); }
    }
    Ok(())
}
