use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub const MAX_TEXT_BYTES: usize = 4 * 1024;
pub const MAX_CONFIG_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_SESSION_ID_BYTES: usize = 16;
pub const MAX_JOB_ID_BYTES: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorldId {
    pub namespace: String,
    pub value: String,
    pub epoch: u64,
}

impl WorldId {
    pub fn new(namespace: impl Into<String>, value: impl Into<String>, epoch: u64) -> Self {
        Self { namespace: namespace.into(), value: value.into(), epoch }
    }

    pub(crate) fn validate(&self) -> Result<(), ModelError> {
        validate_text("world namespace", &self.namespace)?;
        validate_text("world value", &self.value)?;
        if self.epoch > i64::MAX as u64 {
            return Err(ModelError::Overflow("world epoch"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct World {
    pub id: WorldId,
    pub config: Vec<u8>,
}

impl World {
    pub fn new(namespace: impl Into<String>, value: impl Into<String>, epoch: u64, config: Vec<u8>) -> Self {
        Self { id: WorldId::new(namespace, value, epoch), config }
    }
    pub fn id(&self) -> WorldId { self.id.clone() }
    pub(crate) fn validate(&self) -> Result<(), ModelError> {
        self.id.validate()?;
        if self.config.len() > MAX_CONFIG_BYTES { return Err(ModelError::Bounds("world config")); }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkCoordinate {
    pub x: i32,
    pub z: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirtyChunk {
    pub world: WorldId,
    pub coordinate: ChunkCoordinate,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i64)]
pub enum JobKind {
    Full = 1,
    Resume = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i64)]
pub enum JobState {
    Queued = 0,
    Running = 1,
    Resumable = 2,
    Completed = 3,
    Failed = 4,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderJob {
    pub id: Vec<u8>,
    pub world: WorldId,
    pub kind: JobKind,
    pub state: JobState,
    pub payload: Vec<u8>,
    pub completed_chunks: u64,
}

impl RenderJob {
    pub fn new(world: WorldId, kind: JobKind, payload: Vec<u8>) -> Self {
        let mut hash = Sha256::new();
        hash.update(world.namespace.as_bytes());
        hash.update([0]);
        hash.update(world.value.as_bytes());
        hash.update([0]);
        hash.update(world.epoch.to_le_bytes());
        hash.update([kind as u8]);
        Self { id: hash.finalize().to_vec(), world, kind, state: JobState::Queued, payload, completed_chunks: 0 }
    }
    pub(crate) fn validate(&self) -> Result<(), ModelError> {
        self.world.validate()?;
        if self.id.is_empty() || self.id.len() > MAX_JOB_ID_BYTES { return Err(ModelError::Bounds("render job ID")); }
        if self.payload.len() > MAX_PAYLOAD_BYTES { return Err(ModelError::Bounds("render job payload")); }
        if self.completed_chunks > i64::MAX as u64 { return Err(ModelError::Overflow("completed chunks")); }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCheckpoint {
    pub session_id: Vec<u8>,
    pub durable_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recovery {
    pub worlds: Vec<World>,
    pub dirty: Vec<DirtyChunk>,
    pub jobs: Vec<RenderJob>,
    pub checkpoints: Vec<SessionCheckpoint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    Bounds(&'static str),
    Overflow(&'static str),
    Invalid(&'static str),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self { Self::Bounds(v) => write!(f, "{v} exceeds configured bounds"), Self::Overflow(v) => write!(f, "{v} does not fit SQLite INTEGER"), Self::Invalid(v) => f.write_str(v) }
    }
}
impl std::error::Error for ModelError {}

pub(crate) fn checked_i64(value: u64, name: &'static str) -> Result<i64, ModelError> {
    i64::try_from(value).map_err(|_| ModelError::Overflow(name))
}

pub(crate) fn checked_u64(value: i64, name: &'static str) -> Result<u64, ModelError> {
    u64::try_from(value).map_err(|_| ModelError::Invalid(name))
}

pub(crate) fn validate_text(name: &'static str, value: &str) -> Result<(), ModelError> {
    if value.is_empty() || value.len() > MAX_TEXT_BYTES || value.bytes().any(|b| b == 0) {
        return Err(ModelError::Bounds(name));
    }
    Ok(())
}
