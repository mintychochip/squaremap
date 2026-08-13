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
