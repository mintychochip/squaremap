use crate::compare::ComparisonReport;
use std::{fs, io, path::Path};

pub fn write_report(path: impl AsRef<Path>, report: &ComparisonReport) -> io::Result<()> {
    let path = path.as_ref();
    let bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    std::io::Write::write_all(&mut file, &bytes)
}

/// Returns true only when the report explicitly passes every required gate.
pub fn migration_report_is_complete(report: &serde_json::Value, required_gates: &[&str]) -> bool {
    if report.get("verdict").and_then(serde_json::Value::as_str) != Some("complete") {
        return false;
    }

    let Some(verified) = report
        .get("verified")
        .and_then(serde_json::Value::as_object)
    else {
        return false;
    };
    if !required_gates.iter().all(|gate| {
        verified
            .get(*gate)
            .and_then(|entry| entry.get("passed"))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    }) {
        return false;
    }
    let Some(blocked) = report.get("blocked") else {
        return true;
    };
    blocked.as_object().is_some_and(serde_json::Map::is_empty)
}
