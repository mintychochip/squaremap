use crate::compare::ComparisonReport;
use std::{fs, io, path::Path};

pub fn write_report(path: impl AsRef<Path>, report: &ComparisonReport) -> io::Result<()> {
    let path = path.as_ref();
    if path.exists() { return Err(io::Error::new(io::ErrorKind::AlreadyExists, "report already exists")); }
    let bytes = serde_json::to_vec_pretty(report).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    fs::write(path, bytes)
}
