use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeSet, BTreeMap}, fs, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParityProbeManifest {
    pub schema_version: u32,
    pub scenario: String,
    pub input_hash: String,
    pub normalization: String,
    pub outputs: Vec<ManifestOutput>,
    #[serde(default)]
    pub allowed_volatility_paths: Vec<String>,
    #[serde(default)]
    pub case_id: Option<String>,
}

#[derive(Debug)]
pub enum ProbeError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Validation(String),
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::Validation(error) => write!(f, "{error}"),
        }
    }
}
impl std::error::Error for ProbeError {}
impl From<std::io::Error> for ProbeError { fn from(e: std::io::Error) -> Self { Self::Io(e) } }
impl From<serde_json::Error> for ProbeError { fn from(e: serde_json::Error) -> Self { Self::Json(e) } }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestOutput {
    pub path: String,
    #[serde(rename = "type")]
    pub output_type: OutputType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputType { Json, Png, Bytes }

impl ParityProbeManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProbeError> {
        let value: Self = serde_json::from_slice(&fs::read(path)?)?;
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), ProbeError> {
        if self.schema_version != 1 { return Err(ProbeError::Validation("schema_version must be 1".into())); }
        if self.scenario.trim().is_empty() { return Err(ProbeError::Validation("scenario must be non-empty".into())); }
        if !is_sha256(&self.input_hash) { return Err(ProbeError::Validation("input_hash must be SHA-256".into())); }
        if self.normalization != "none" { return Err(ProbeError::Validation("unsupported normalization".into())); }
        if self.outputs.is_empty() { return Err(ProbeError::Validation("outputs must be non-empty".into())); }
        let mut paths = BTreeSet::new();
        for output in &self.outputs {
            if !valid_rel(&output.path) { return Err(ProbeError::Validation(format!("invalid output path: {}", output.path))); }
            if !paths.insert(&output.path) { return Err(ProbeError::Validation(format!("duplicate output path: {}", output.path))); }
        }
        for path in &self.allowed_volatility_paths {
            if !valid_rel(path) { return Err(ProbeError::Validation("invalid volatility path".into())); }
        }
        if self.case_id.as_deref().is_some_and(|id| id != "fixture") {
            return Err(ProbeError::Validation("unknown case".into()));
        }
        Ok(())
    }
}
fn valid_rel(path:&str)->bool { !path.is_empty() && !Path::new(path).is_absolute() && !path.split('/').any(|p| p==".." || p.is_empty()) && !path.contains('\\') }
fn is_sha256(value:&str)->bool { value.len()==64 && value.bytes().all(|b| b.is_ascii_hexdigit()) }

#[derive(Debug, Clone, Serialize, Deserialize)] pub struct ParityReport {
 pub scenario:String, pub input_hash:String, pub java_artifact:String, pub rust_artifact:String,
 pub expected_paths:Vec<String>, pub compared_paths:Vec<String>, pub missing_paths:Vec<String>, pub extra_paths:Vec<String>, pub mismatches:Vec<ParityMismatch>, pub normalization:String, pub complete:bool, pub verdict:String,
}
#[derive(Debug, Clone, Serialize, Deserialize)] pub struct ParityMismatch { pub path:String, pub detail:String }
fn compare_output(output_type: &OutputType, path: &str, a: &[u8], b: &[u8]) -> Result<bool, ProbeError> {
    match output_type {
        OutputType::Json => {
            let left: serde_json::Value = serde_json::from_slice(a)
                .map_err(|error| ProbeError::Validation(format!("invalid JSON at {path}: {error}")))?;
            let right: serde_json::Value = serde_json::from_slice(b)
                .map_err(|error| ProbeError::Validation(format!("invalid JSON at {path}: {error}")))?;
            Ok(left == right)
        }
        OutputType::Png => {
            let left = crate::compare::decode_png(a)
                .map_err(|error| ProbeError::Validation(format!("invalid PNG at {path}: {error}")))?;
            let right = crate::compare::decode_png(b)
                .map_err(|error| ProbeError::Validation(format!("invalid PNG at {path}: {error}")))?;
            Ok(left == right)
        }
        OutputType::Bytes => Ok(a == b),
    }
}
pub fn run_manifest(manifest: &ParityProbeManifest, java_root: &Path, rust_root: &Path) -> Result<ParityReport, ProbeError> {
    if !java_root.is_dir() || !rust_root.is_dir() {
        return Err(ProbeError::Validation("adapter root is missing or not a directory".into()));
    }
    let expected: Vec<String> = manifest.outputs.iter().map(|o| o.path.clone()).collect();
    let mut missing = Vec::new();
    let mut extra = Vec::new();
    let mut mismatches = Vec::new();
    let java_files = collect(java_root)?;
    let rust_files = collect(rust_root)?;
    let declared: BTreeSet<_> = expected.iter().cloned().collect();
    for output in &manifest.outputs {
        let java = java_files.get(&output.path);
        let rust = rust_files.get(&output.path);
        if java.is_none() { missing.push(format!("java:{}", output.path)); }
        if rust.is_none() { missing.push(format!("rust:{}", output.path)); }
        if let (Some(left), Some(right)) = (java, rust) {
            if !compare_output(&output.output_type, &output.path, left, right)? {
                mismatches.push(ParityMismatch { path: output.path.clone(), detail: "output differs".into() });
            }
        }
    }
    for path in java_files.keys().chain(rust_files.keys()) {
        if !declared.contains(path) && !extra.contains(path) {
            extra.push(path.clone());
        }
    }
    let complete = missing.is_empty() && extra.is_empty();
    let java_artifact = hash_root(java_root)?;
    let rust_artifact = hash_root(rust_root)?;
    let verdict = if complete && mismatches.is_empty() { "equal" } else if complete { "mismatch" } else { "incomplete" };
    Ok(ParityReport {
        scenario: manifest.scenario.clone(),
        input_hash: manifest.input_hash.clone(),
        java_artifact,
        rust_artifact,
        expected_paths: expected,
        compared_paths: java_files.keys().filter(|p| rust_files.contains_key(*p)).cloned().collect(),
        missing_paths: missing,
        extra_paths: extra,
        mismatches,
        normalization: manifest.normalization.clone(),
        complete,
        verdict: verdict.into(),
    })
}
pub fn write_parity_report(path:&Path, report:&ParityReport)->Result<(),ProbeError>{ let mut o=fs::OpenOptions::new(); o.write(true).create_new(true); use std::io::Write; let mut f=o.open(path)?; f.write_all(&serde_json::to_vec_pretty(report)?)?; Ok(()) }
fn collect(root:&Path)->Result<std::collections::BTreeMap<String,Vec<u8>>,ProbeError>{ fn rec(root:&Path,dir:&Path,out:&mut std::collections::BTreeMap<String,Vec<u8>>)->Result<(),ProbeError>{ for e in fs::read_dir(dir)? { let p=e?.path(); if p.is_dir(){rec(root,&p,out)?}else{out.insert(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\',"/"),fs::read(p)?);} } Ok(()) } let mut out=BTreeMap::new(); rec(root,root,&mut out)?; Ok(out) }
fn hash_root(root:&Path)->Result<String,ProbeError>{ let files=collect(root)?; let mut h=Sha256::new(); for (p,b) in files { h.update(p.as_bytes()); h.update(b); } Ok(format!("{:x}",h.finalize())) }
