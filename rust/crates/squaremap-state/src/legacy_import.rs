use crate::model::{ChunkCoordinate, MAX_PAYLOAD_BYTES, MAX_TEXT_BYTES};
use sha2::{Digest, Sha256};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 200_000;

#[derive(Clone)]
pub(crate) struct ParsedFile {
    pub relative_path: String,
    pub sha256: Vec<u8>,
}

#[derive(Clone)]
pub(crate) struct ParsedLegacy {
    pub dirty: Vec<ChunkCoordinate>,
    pub resume_payload: Option<Vec<u8>>,
    dirty_file: Option<ParsedFile>,
    resume_file: Option<ParsedFile>,
}

impl ParsedLegacy {
    pub fn files(&self) -> impl Iterator<Item = &ParsedFile> {
        self.dirty_file.iter().chain(self.resume_file.iter())
    }
}

pub(crate) fn parse_legacy_files(directory: &Path) -> Result<ParsedLegacy, LegacyError> {
    if directory.as_os_str().len() > MAX_TEXT_BYTES { return Err(LegacyError { path: directory.to_path_buf(), message: "directory path exceeds bounds".into() }); }
    let dirty_path = directory.join("dirty_chunks.json");
    let resume_path = directory.join("resume_render.json");
    let dirty = read_candidate(&dirty_path)?;
    let resume = read_candidate(&resume_path)?;
    let (dirty_chunks, dirty_file) = match dirty {
        Some((bytes, sha)) => (parse_dirty(&dirty_path, &bytes)?, Some(ParsedFile { relative_path: "dirty_chunks.json".into(), sha256: sha })),
        None => (Vec::new(), None),
    };
    let (resume_payload, resume_file) = match resume {
        Some((bytes, sha)) => (Some(parse_resume(&resume_path, &bytes)?), Some(ParsedFile { relative_path: "resume_render.json".into(), sha256: sha })),
        None => (None, None),
    };
    Ok(ParsedLegacy { dirty: dirty_chunks, resume_payload, dirty_file, resume_file })
}

#[derive(Debug)]
pub(crate) struct LegacyError {
    pub path: PathBuf,
    pub message: String,
}

fn read_candidate(path: &Path) -> Result<Option<(Vec<u8>, Vec<u8>)>, LegacyError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(LegacyError { path: path.to_path_buf(), message: error.to_string() }),
    };
    if !metadata.is_file() { return Err(LegacyError { path: path.to_path_buf(), message: "expected a regular file".into() }); }
    if metadata.len() > MAX_FILE_BYTES as u64 { return Err(LegacyError { path: path.to_path_buf(), message: format!("file exceeds {MAX_FILE_BYTES} bytes") }); }
    let bytes = fs::read(path).map_err(|error| LegacyError { path: path.to_path_buf(), message: error.to_string() })?;
    let mut hasher = Sha256::new(); hasher.update(&bytes);
    Ok(Some((bytes, hasher.finalize().to_vec())))
}

fn parse_dirty(path: &Path, bytes: &[u8]) -> Result<Vec<ChunkCoordinate>, LegacyError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| LegacyError { path: path.to_path_buf(), message: error.to_string() })?;
    let array = value.as_array().ok_or_else(|| invalid(path, "expected an array"))?;
    if array.len() > MAX_ENTRIES { return Err(invalid(path, "entry count exceeds bound")); }
    let mut coordinates = Vec::with_capacity(array.len());
    for entry in array {
        let object = entry.as_object().ok_or_else(|| invalid(path, "entry must be an object"))?;
        if object.len() != 2 || !object.contains_key("x") || !object.contains_key("z") { return Err(invalid(path, "entry must contain only x and z")); }
        let x = parse_coordinate(path, object.get("x"), "x")?;
        let z = parse_coordinate(path, object.get("z"), "z")?;
        let coordinate = ChunkCoordinate { x, z };
        if !coordinates.contains(&coordinate) { coordinates.push(coordinate); }
    }
    coordinates.sort_by_key(|coordinate| (coordinate.x, coordinate.z));
    Ok(coordinates)
}

fn parse_resume(path: &Path, bytes: &[u8]) -> Result<Vec<u8>, LegacyError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| LegacyError { path: path.to_path_buf(), message: error.to_string() })?;
    let array = value.as_array().ok_or_else(|| invalid(path, "expected Gson complex-map-key entry array"))?;
    if array.len() > MAX_ENTRIES { return Err(invalid(path, "entry count exceeds bound")); }
    let mut normalized = Vec::with_capacity(array.len());
    for entry in array {
        let pair = entry.as_array().ok_or_else(|| invalid(path, "entry must be a two-element array"))?;
        if pair.len() != 2 { return Err(invalid(path, "entry must be a two-element array")); }
        let object = pair[0].as_object().ok_or_else(|| invalid(path, "map key must be an object"))?;
        if object.len() != 2 || !object.contains_key("x") || !object.contains_key("z") { return Err(invalid(path, "map key must contain only x and z")); }
        let x = parse_coordinate(path, object.get("x"), "x")?;
        let z = parse_coordinate(path, object.get("z"), "z")?;
        let completed = pair[1].as_bool().ok_or_else(|| invalid(path, "map value must be boolean"))?;
        normalized.push(serde_json::json!([{"x": x, "z": z}, completed]));
    }
    let payload = serde_json::to_vec(&normalized).map_err(|error| invalid(path, error.to_string()))?;
    if payload.len() > MAX_PAYLOAD_BYTES { return Err(invalid(path, "resume payload exceeds bound")); }
    Ok(payload)
}

fn parse_coordinate(path: &Path, value: Option<&Value>, name: &'static str) -> Result<i32, LegacyError> {
    let value = value.and_then(Value::as_i64).ok_or_else(|| invalid(path, format!("{name} must be an integer")))?;
    i32::try_from(value).map_err(|_| invalid(path, format!("{name} is outside signed 32-bit range")))
}

fn invalid(path: &Path, message: impl Into<String>) -> LegacyError {
    LegacyError { path: path.to_path_buf(), message: message.into() }
}
