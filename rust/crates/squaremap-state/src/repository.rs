use crate::legacy_import::{parse_legacy_files, ParsedLegacy};
use crate::model::*;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const APPLICATION_ID: i64 = 0x5351_4d50;
const SCHEMA_VERSION: i64 = 4;
const SCHEMA: &str = include_str!("../migrations/0001_initial.sql");
const RETRY_MIGRATION: &str = include_str!("../migrations/0002_dirty_retry.sql");
const OWNER_MIGRATION: &str = include_str!("../migrations/0003_owner_lease.sql");
const MAX_BLOCKING_CALLS: usize = 1;

#[derive(Debug)]
pub enum RepositoryError {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    Join(tokio::task::JoinError),
    Model(ModelError),
    Schema(String),
    StaleWorld(WorldId),
    MissingWorld(WorldId),
    InvalidLegacy { path: PathBuf, message: String },
}

impl std::fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Sqlite(e) => write!(f, "SQLite error: {e}"),
            Self::Join(e) => write!(f, "blocking task failed: {e}"),
            Self::Model(e) => e.fmt(f),
            Self::Schema(e) => write!(f, "unsupported state database: {e}"),
            Self::StaleWorld(id) => write!(f, "stale world epoch for {}/{} (epoch {})", id.namespace, id.value, id.epoch),
            Self::MissingWorld(id) => write!(f, "world not found: {}/{} (epoch {})", id.namespace, id.value, id.epoch),
            Self::InvalidLegacy { path, message } => write!(f, "invalid legacy state '{}': {message}", path.display()),
        }
    }
}
impl std::error::Error for RepositoryError {}
impl From<std::io::Error> for RepositoryError { fn from(e: std::io::Error) -> Self { Self::Io(e) } }
impl From<rusqlite::Error> for RepositoryError { fn from(e: rusqlite::Error) -> Self { Self::Sqlite(e) } }
impl From<tokio::task::JoinError> for RepositoryError { fn from(e: tokio::task::JoinError) -> Self { Self::Join(e) } }
impl From<ModelError> for RepositoryError { fn from(e: ModelError) -> Self { Self::Model(e) } }

#[derive(Clone)]
pub struct Repository {
    connection: Arc<Mutex<Connection>>,
    permit: Arc<Semaphore>,
}

impl Repository {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, RepositoryError> {
        let path = path.as_ref().to_path_buf();
        let permit = Arc::new(Semaphore::new(MAX_BLOCKING_CALLS));
        let permit_guard = permit.clone().acquire_owned().await.map_err(|_| RepositoryError::Schema("blocking gate closed".into()))?;
        let connection = tokio::task::spawn_blocking(move || open_connection(&path)).await??;
        drop(permit_guard);
        Ok(Self { connection: Arc::new(Mutex::new(connection)), permit })
    }

    pub async fn bridge_checkpoint(&self, bridge_id: &[u8]) -> Result<Option<SessionCheckpoint>, RepositoryError> {
        if bridge_id.len() != MAX_BRIDGE_ID_BYTES || bridge_id.iter().all(|byte| *byte == 0) { return Err(ModelError::Bounds("bridge ID must be exactly 16 non-zero bytes").into()); }
        let bridge_id = bridge_id.to_vec();
        self.blocking(move |connection| {
            connection.query_row(
                "SELECT length(CAST(bridge_id AS BLOB)),COALESCE(substr(CAST(bridge_id AS BLOB),1,17),zeroblob(0)),length(CAST(session_id AS BLOB)),COALESCE(substr(CAST(session_id AS BLOB),1,17),zeroblob(0)),durable_sequence FROM bridge_checkpoints WHERE bridge_id=?1",
                params![bridge_id],
                |row| {
                    let stored_bridge_id = bounded_blob(row.get(0)?, row.get(1)?, MAX_BRIDGE_ID_BYTES)?;
                    let stored_session_id = bounded_blob(row.get(2)?, row.get(3)?, MAX_SESSION_ID_BYTES)?;
                    if stored_bridge_id.len() != MAX_BRIDGE_ID_BYTES
                        || stored_bridge_id.iter().all(|byte| *byte == 0)
                        || stored_session_id.len() != MAX_SESSION_ID_BYTES
                    {
                        return Err(rusqlite::Error::InvalidQuery);
                    }
                    Ok(SessionCheckpoint {
                        bridge_id: stored_bridge_id,
                        session_id: stored_session_id,
                        durable_sequence: checked_u64(row.get(4)?, "checkpoint").map_err(|_| rusqlite::Error::InvalidQuery)?,
                    })
                },
            ).optional().map_err(RepositoryError::from)
        }).await
    }
    /// Durably persists an authenticated bridge identity and its accepted
    /// sequence watermark before dirty work is processed.
    pub async fn persist_bridge_identity(
        &self,
        bridge_id: &[u8],
        session_id: &[u8],
        durable_sequence: u64,
    ) -> Result<(), RepositoryError> {
        if bridge_id.len() != MAX_BRIDGE_ID_BYTES || bridge_id.iter().all(|byte| *byte == 0) {
            return Err(ModelError::Bounds("bridge ID must be exactly 16 non-zero bytes").into());
        }
        if session_id.len() != MAX_SESSION_ID_BYTES {
            return Err(ModelError::Bounds("session ID must be exactly 16 bytes").into());
        }
        let durable_sequence = checked_i64(durable_sequence, "checkpoint sequence")?;
        let bridge_id = bridge_id.to_vec();
        let session_id = session_id.to_vec();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT INTO bridge_checkpoints(bridge_id,session_id,durable_sequence) VALUES(?1,?2,?3)
                 ON CONFLICT(bridge_id) DO UPDATE SET session_id=excluded.session_id,durable_sequence=MAX(bridge_checkpoints.durable_sequence,excluded.durable_sequence)",
                params![bridge_id, session_id, durable_sequence],
            )?;
            transaction.commit()?;
            Ok(())
        }).await
    }

    /// Returns the single durable bridge identity, or no identity before the
    /// first authenticated negotiation. Multiple identities are fail-closed.
    pub async fn bridge_identity(&self) -> Result<Option<Vec<u8>>, RepositoryError> {
        self.blocking(|connection| {
            let mut statement = connection.prepare("SELECT bridge_id FROM bridge_checkpoints ORDER BY bridge_id")?;
            let mut rows = statement.query([])?;
            let mut identity = None;
            while let Some(row) = rows.next()? {
                let raw: Vec<u8> = row.get(0)?;
                if raw.len() != MAX_BRIDGE_ID_BYTES || raw.iter().all(|byte| *byte == 0) {
                    return Err(RepositoryError::Schema("bridge_checkpoints contains an invalid bridge identity".into()));
                }
                if identity.is_some() {
                    return Err(RepositoryError::Schema("multiple bridge identities are persisted".into()));
                }
                identity = Some(raw);
            }
            Ok(identity)
        }).await
    }

    async fn blocking<T, F>(&self, operation: F) -> Result<T, RepositoryError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, RepositoryError> + Send + 'static,
    {
        let permit: OwnedSemaphorePermit = self.permit.clone().acquire_owned().await.map_err(|_| RepositoryError::Schema("blocking gate closed".into()))?;
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut connection = connection.lock().map_err(|_| RepositoryError::Schema("connection lock poisoned".into()))?;
            operation(&mut connection)
        }).await?
    }

    pub async fn apply_world(&self, world: World) -> Result<(), RepositoryError> {
        world.validate()?;
        let epoch = checked_i64(world.id.epoch, "world epoch")?;
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            let existing: Option<i64> = transaction.query_row("SELECT epoch FROM worlds WHERE namespace=?1 AND value=?2", params![world.id.namespace, world.id.value], |row| row.get(0)).optional()?;
            if let Some(raw) = existing {
                if raw < 0 {
                    let removed = decode_tombstone(raw)?;
                    if world.id.epoch <= removed { return Err(RepositoryError::StaleWorld(world.id)); }
                    transaction.execute("DELETE FROM dirty_chunks WHERE namespace=?1 AND value=?2", params![world.id.namespace, world.id.value])?;
                    transaction.execute("DELETE FROM render_jobs WHERE namespace=?1 AND value=?2", params![world.id.namespace, world.id.value])?;
                } else if raw > epoch {
                    return Err(RepositoryError::StaleWorld(world.id));
                } else if raw < epoch {
                    transaction.execute("DELETE FROM dirty_chunks WHERE namespace=?1 AND value=?2", params![world.id.namespace, world.id.value])?;
                    transaction.execute("DELETE FROM render_jobs WHERE namespace=?1 AND value=?2", params![world.id.namespace, world.id.value])?;
                }
            }
            transaction.execute("INSERT INTO worlds(namespace,value,epoch,config) VALUES(?1,?2,?3,?4) ON CONFLICT(namespace,value) DO UPDATE SET epoch=excluded.epoch, config=excluded.config", params![world.id.namespace, world.id.value, epoch, world.config])?;
            transaction.commit()?;
            Ok(())
        }).await
    }

    pub async fn remove_world(&self, world: &WorldId) -> Result<bool, RepositoryError> {
        world.validate()?;
        let epoch = checked_i64(world.epoch, "world epoch")?;
        let tombstone = -1 - epoch;
        let world = world.clone();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            let current: Option<i64> = transaction.query_row("SELECT epoch FROM worlds WHERE namespace=?1 AND value=?2", params![world.namespace, world.value], |row| row.get(0)).optional()?;
            if current != Some(epoch) { transaction.commit()?; return Ok(false); }
            transaction.execute("DELETE FROM dirty_chunks WHERE namespace=?1 AND value=?2", params![world.namespace, world.value])?;
            transaction.execute("DELETE FROM render_jobs WHERE namespace=?1 AND value=?2", params![world.namespace, world.value])?;
            transaction.execute("UPDATE worlds SET epoch=?1 WHERE namespace=?2 AND value=?3", params![tombstone, world.namespace, world.value])?;
            transaction.commit()?;
            Ok(true)
        }).await
    }
    /// Clears durable dirty work and render jobs while retaining the configured world epoch.
    pub async fn reset_world(&self, world: &WorldId) -> Result<(), RepositoryError> {
        world.validate()?;
        let world = world.clone();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            ensure_current_world(&transaction, &world)?;
            transaction.execute(
                "DELETE FROM dirty_chunks WHERE namespace=?1 AND value=?2 AND epoch=?3",
                params![world.namespace, world.value, world.epoch as i64],
            )?;
            transaction.execute(
                "DELETE FROM render_jobs WHERE namespace=?1 AND value=?2 AND epoch=?3",
                params![world.namespace, world.value, world.epoch as i64],
            )?;
            transaction.commit()?;
            Ok(())
        }).await
    }

    pub async fn mark_dirty(&self, world: &WorldId, coordinate: ChunkCoordinate, revision: u64, bridge_id: &[u8], session_id: &[u8], sequence: u64) -> Result<bool, RepositoryError> {
        world.validate()?;
        if bridge_id.len() != MAX_BRIDGE_ID_BYTES || bridge_id.iter().all(|byte| *byte == 0) { return Err(ModelError::Bounds("bridge ID must be exactly 16 non-zero bytes").into()); }
        if session_id.len() != MAX_SESSION_ID_BYTES { return Err(ModelError::Bounds("session ID must be exactly 16 bytes").into()); }
        let revision = checked_i64(revision, "dirty revision")?;
        let sequence = checked_i64(sequence, "session sequence")?;
        let world = world.clone();
        let bridge_id = bridge_id.to_vec();
        let session_id = session_id.to_vec();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            ensure_current_world(&transaction, &world)?;
            let high_water: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(revision), 0) FROM dirty_chunks WHERE namespace=?1 AND value=?2 AND epoch=?3",
                params![world.namespace, world.value, world.epoch as i64],
                |row| row.get(0),
            )?;
            let assigned = revision.max(high_water.saturating_add(1));
            let changed = transaction.execute("INSERT INTO dirty_chunks(namespace,value,epoch,x,z,revision,owner_bridge_id,owner_session_id,lease_expires_epoch_seconds,replay_pending) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,0,0) ON CONFLICT(namespace,value,epoch,x,z) DO UPDATE SET revision=excluded.revision,owner_bridge_id=excluded.owner_bridge_id,owner_session_id=excluded.owner_session_id,replay_pending=0", params![world.namespace, world.value, world.epoch as i64, coordinate.x, coordinate.z, assigned, bridge_id, session_id])?;
            transaction.execute("DELETE FROM dirty_retries WHERE namespace=?1 AND value=?2 AND epoch=?3 AND x=?4 AND z=?5", params![world.namespace, world.value, world.epoch as i64, coordinate.x, coordinate.z])?;
            if sequence != 0 {
                transaction.execute("INSERT INTO bridge_checkpoints(bridge_id,session_id,durable_sequence) VALUES(?1,?2,?3) ON CONFLICT(bridge_id) DO UPDATE SET session_id=excluded.session_id,durable_sequence=MAX(bridge_checkpoints.durable_sequence,excluded.durable_sequence)", params![bridge_id, session_id, sequence])?;
            }
            transaction.commit()?;
            Ok(changed != 0)
        }).await
    }
    /// Returns at most `limit` current dirty rows whose retry deadline has elapsed.
    pub async fn dirty_page_at(&self, limit: usize, now: i64) -> Result<Vec<DirtyChunk>, RepositoryError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.blocking(move |connection| {
            let mut statement = connection.prepare(
                "SELECT length(CAST(namespace AS BLOB)),COALESCE(substr(CAST(namespace AS BLOB),1,4097),zeroblob(0)),
                        length(CAST(value AS BLOB)),COALESCE(substr(CAST(value AS BLOB),1,4097),zeroblob(0)),
                        epoch,x,z,revision,
                        length(CAST(owner_bridge_id AS BLOB)),COALESCE(substr(CAST(owner_bridge_id AS BLOB),1,17),zeroblob(0)),
                        lease_expires_epoch_seconds
                 FROM (
                    SELECT dirty_chunks.namespace AS namespace,dirty_chunks.value AS value,
                           dirty_chunks.epoch AS epoch,dirty_chunks.x AS x,dirty_chunks.z AS z,
                           dirty_chunks.revision AS revision,
                           dirty_chunks.owner_bridge_id AS owner_bridge_id,
                           dirty_chunks.lease_expires_epoch_seconds AS lease_expires_epoch_seconds,
                           ROW_NUMBER() OVER (PARTITION BY dirty_chunks.namespace,dirty_chunks.value,dirty_chunks.epoch
                                              ORDER BY dirty_chunks.revision DESC,dirty_chunks.x,dirty_chunks.z) AS ordinal
                    FROM dirty_chunks
                    INNER JOIN worlds USING(namespace,value)
                    LEFT JOIN dirty_retries USING(namespace,value,epoch,x,z)
                    WHERE worlds.epoch >= 0 AND dirty_chunks.epoch=worlds.epoch
                      AND (dirty_retries.next_attempt IS NULL OR dirty_retries.next_attempt <= ?1)
                 ) ORDER BY ordinal,namespace,value,epoch,x,z LIMIT ?2",
            )?;
            let rows = statement.query_map(params![now, limit], |row| {
                let world = WorldId::new(
                    bounded_text(row.get(0)?, row.get(1)?, MAX_TEXT_BYTES)?,
                    bounded_text(row.get(2)?, row.get(3)?, MAX_TEXT_BYTES)?,
                    checked_u64(row.get(4)?, "dirty epoch").map_err(|_| rusqlite::Error::InvalidQuery)?,
                );
                world.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
                let owner_bridge_id = bounded_blob(row.get(8)?, row.get(9)?, MAX_BRIDGE_ID_BYTES)?;
                if owner_bridge_id.len() != MAX_BRIDGE_ID_BYTES
                    || owner_bridge_id.iter().all(|byte| *byte == 0)
                {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                Ok(DirtyChunk {
                    world,
                    coordinate: ChunkCoordinate { x: row.get(5)?, z: row.get(6)? },
                    revision: checked_u64(row.get(7)?, "dirty revision").map_err(|_| rusqlite::Error::InvalidQuery)?,
                    owner_bridge_id,
                    lease_expires_epoch_seconds: row.get(10)?,
                })
            })?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }).await
    }

    pub async fn dirty_page(&self, limit: usize) -> Result<Vec<DirtyChunk>, RepositoryError> {
        self.dirty_page_at(limit, now_seconds()).await
    }


    pub async fn defer_dirty(&self, world: &WorldId, coordinate: ChunkCoordinate, revision: u64, now: i64) -> Result<(), RepositoryError> {
        let revision = checked_i64(revision, "dirty revision")?;
        let world = world.clone();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            ensure_current_world(&transaction, &world)?;
            let attempt: i64 = transaction.query_row(
                "SELECT COALESCE(attempt, 0) FROM dirty_retries WHERE namespace=?1 AND value=?2 AND epoch=?3 AND x=?4 AND z=?5",
                params![world.namespace, world.value, world.epoch as i64, coordinate.x, coordinate.z],
                |row| row.get(0),
            ).optional()?.unwrap_or(0);
            let next_attempt = attempt.saturating_add(1).min(16);
            let delay = 1_i64.checked_shl(next_attempt as u32).unwrap_or(86_400).min(86_400);
            transaction.execute(
                "INSERT INTO dirty_retries(namespace,value,epoch,x,z,attempt,next_attempt) VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(namespace,value,epoch,x,z) DO UPDATE SET attempt=excluded.attempt,next_attempt=excluded.next_attempt
                 WHERE EXISTS (SELECT 1 FROM dirty_chunks WHERE namespace=?1 AND value=?2 AND epoch=?3 AND x=?4 AND z=?5 AND revision<=?8)",
                params![world.namespace, world.value, world.epoch as i64, coordinate.x, coordinate.z, next_attempt, now.saturating_add(delay), revision],
            )?;
            transaction.commit()?;
            Ok(())
        }).await
    }

    pub async fn world_is_current(&self, world: &WorldId) -> Result<bool, RepositoryError> {
        world.validate()?;
        let world = world.clone();
        self.blocking(move |connection| {
            let current: Option<i64> = connection.query_row(
                "SELECT epoch FROM worlds WHERE namespace=?1 AND value=?2",
                params![world.namespace, world.value],
                |row| row.get(0),
            ).optional()?;
            Ok(current == Some(checked_i64(world.epoch, "world epoch")?))
        }).await
    }

    pub async fn complete_dirty(&self, world: &WorldId, coordinate: ChunkCoordinate, revision: u64) -> Result<bool, RepositoryError> {
        let revision = checked_i64(revision, "dirty revision")?;
        let world = world.clone();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            ensure_current_world(&transaction, &world)?;
            let changed = transaction.execute("DELETE FROM dirty_chunks WHERE namespace=?1 AND value=?2 AND epoch=?3 AND x=?4 AND z=?5 AND revision <= ?6", params![world.namespace, world.value, world.epoch as i64, coordinate.x, coordinate.z, revision])?;
            transaction.execute("DELETE FROM dirty_retries WHERE namespace=?1 AND value=?2 AND epoch=?3 AND x=?4 AND z=?5", params![world.namespace, world.value, world.epoch as i64, coordinate.x, coordinate.z])?;
            transaction.commit()?;
            Ok(changed != 0)
        }).await
    }

    pub async fn create_render_job(&self, mut job: RenderJob) -> Result<(), RepositoryError> {
        if job.id.is_empty() { job.id = deterministic_job_id(&job.world, job.kind); }
        job.validate()?;
        let job = job.clone();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            ensure_current_world(&transaction, &job.world)?;
            let existing: Option<(String, String, i64, i64, i64, Vec<u8>, i64)> = transaction.query_row("SELECT length(CAST(namespace AS BLOB)),COALESCE(substr(CAST(namespace AS BLOB),1,4097),zeroblob(0)),length(CAST(value AS BLOB)),COALESCE(substr(CAST(value AS BLOB),1,4097),zeroblob(0)),epoch,kind,state,length(payload),COALESCE(substr(payload,1,16777217),zeroblob(0)),completed_chunks FROM render_jobs WHERE id=?1", params![job.id], |row| {
                Ok((
                    bounded_text(row.get(0)?, row.get(1)?, MAX_TEXT_BYTES)?,
                    bounded_text(row.get(2)?, row.get(3)?, MAX_TEXT_BYTES)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    bounded_blob(row.get(7)?, row.get(8)?, MAX_PAYLOAD_BYTES)?,
                    row.get(9)?,
                ))
            }).optional()?;
            if let Some(existing) = existing {
                let same = existing.0 == job.world.namespace && existing.1 == job.world.value && existing.2 == job.world.epoch as i64 && existing.3 == job.kind as i64 && existing.4 == job.state as i64 && existing.5 == job.payload && existing.6 == job.completed_chunks as i64;
                if !same { return Err(RepositoryError::Schema("conflicting render job ID".into())); }
            } else {
                transaction.execute("INSERT INTO render_jobs(id,namespace,value,epoch,kind,state,payload,completed_chunks) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![job.id, job.world.namespace, job.world.value, job.world.epoch as i64, job.kind as i64, job.state as i64, job.payload, job.completed_chunks as i64])?;
            }
            transaction.commit()?;
            Ok(())
        }).await
    }

    pub async fn update_render_job(&self, job: RenderJob) -> Result<(), RepositoryError> {
        job.validate()?;
        let job = job.clone();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            ensure_current_world(&transaction, &job.world)?;
            let existing: Option<(i64, i64, i64, Vec<u8>)> = transaction.query_row("SELECT kind,state,completed_chunks,length(payload),COALESCE(substr(payload,1,16777217),zeroblob(0)) FROM render_jobs WHERE id=?1 AND namespace=?2 AND value=?3 AND epoch=?4", params![job.id, job.world.namespace, job.world.value, job.world.epoch as i64], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, bounded_blob(row.get(3)?, row.get(4)?, MAX_PAYLOAD_BYTES)?))).optional()?;
            let Some((kind, state, progress, payload)) = existing else { return Err(RepositoryError::Schema("render job does not exist".into())); };
            if decode_kind(kind).is_err() || kind != job.kind as i64 { return Err(RepositoryError::Schema("render job kind does not match ID".into())); }
            if decode_state(state).is_err() || progress < 0 || payload.len() > MAX_PAYLOAD_BYTES { return Err(RepositoryError::Schema("existing render job row is malformed".into())); }
            let exact = state == job.state as i64 && progress == job.completed_chunks as i64 && payload == job.payload;
            if (state == JobState::Completed as i64
                || state == JobState::Failed as i64
                || state == JobState::Cancelled as i64)
                && !exact
            {
                return Err(RepositoryError::Schema("terminal render job cannot be changed".into()));
            }
            if (job.completed_chunks as i64) < progress { return Err(RepositoryError::Schema("render job progress cannot decrease".into())); }
            if exact { transaction.commit()?; return Ok(()); }
            if job.kind == JobKind::Resume && job.id == deterministic_job_id(&job.world, JobKind::Resume) {
                transaction.execute("DELETE FROM legacy_imports WHERE relative_path=?1", params![legacy_key(&job.world, "resume_render.payload.sha256")])?;
            }
            transaction.execute("UPDATE render_jobs SET state=?1,payload=?2,completed_chunks=?3 WHERE id=?4 AND namespace=?5 AND value=?6 AND epoch=?7", params![job.state as i64, job.payload, job.completed_chunks as i64, job.id, job.world.namespace, job.world.value, job.world.epoch as i64])?;
            transaction.commit()?;
            Ok(())
        }).await
    }

    pub async fn load_render_job(&self, id: &[u8]) -> Result<Option<RenderJob>, RepositoryError> {
        if id.is_empty() || id.len() > MAX_JOB_ID_BYTES {
            return Err(ModelError::Bounds("render job ID").into());
        }
        let id = id.to_vec();
        self.blocking(move |connection| {
            let row = connection.query_row(
                "SELECT length(CAST(id AS BLOB)),COALESCE(substr(CAST(id AS BLOB),1,33),zeroblob(0)),\
                        length(CAST(namespace AS BLOB)),COALESCE(substr(CAST(namespace AS BLOB),1,4097),zeroblob(0)),\
                        length(CAST(value AS BLOB)),COALESCE(substr(CAST(value AS BLOB),1,4097),zeroblob(0)),\
                        epoch,kind,state,length(payload),COALESCE(substr(payload,1,16777217),zeroblob(0)),completed_chunks \
                 FROM render_jobs WHERE id=?1",
                params![id],
                |row| {
                    let job = RenderJob {
                        id: bounded_blob(row.get(0)?, row.get(1)?, MAX_JOB_ID_BYTES)?,
                        world: WorldId::new(
                            bounded_text(row.get(2)?, row.get(3)?, MAX_TEXT_BYTES)?,
                            bounded_text(row.get(4)?, row.get(5)?, MAX_TEXT_BYTES)?,
                            checked_u64(row.get(6)?, "job epoch").map_err(|_| rusqlite::Error::InvalidQuery)?,
                        ),
                        kind: decode_kind(row.get(7)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
                        state: decode_state(row.get(8)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
                        payload: bounded_blob(row.get(9)?, row.get(10)?, MAX_PAYLOAD_BYTES)?,
                        completed_chunks: checked_u64(row.get(11)?, "completed chunks")
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    };
                    job.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
                    Ok(job)
                },
            ).optional()?;
            Ok(row)
        }).await
    }

    /// Releases the lease on one owned row (clears expiry and replay flag) so a
    /// stale row can never linger leased by a dead bridge.
    /// Releases the exact lease identified by world, coordinate, revision, owner,
    /// and session. Stale cleanup must never clear another row's lease.
    pub async fn release_dirty_lease(
        &self,
        world: &WorldId,
        coordinate: ChunkCoordinate,
        revision: u64,
        bridge_id: &[u8],
        session_id: &[u8],
    ) -> Result<(), RepositoryError> {
        world.validate()?;
        if bridge_id.len() != MAX_BRIDGE_ID_BYTES || bridge_id.iter().all(|byte| *byte == 0) {
            return Err(ModelError::Bounds("bridge ID must be exactly 16 non-zero bytes").into());
        }
        if session_id.len() != MAX_SESSION_ID_BYTES || session_id.iter().all(|byte| *byte == 0) {
            return Err(ModelError::Bounds("session ID must be exactly 16 non-zero bytes").into());
        }
        let revision = checked_i64(revision, "dirty revision")?;
        let world = world.clone();
        let bridge_id = bridge_id.to_vec();
        let session_id = session_id.to_vec();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            ensure_current_world(&transaction, &world)?;
            transaction.execute(
                "UPDATE dirty_chunks SET lease_expires_epoch_seconds=0,replay_pending=0
                 WHERE namespace=?1 AND value=?2 AND epoch=?3 AND x=?4 AND z=?5
                   AND revision=?6 AND owner_bridge_id=?7 AND owner_session_id=?8",
                params![world.namespace, world.value, world.epoch as i64, coordinate.x, coordinate.z, revision, bridge_id, session_id],
            )?;
            transaction.commit()?;
            Ok(())
        }).await
    }

    /// Owner-scoped page: returns at most `limit` rows owned by `bridge_id` whose
    /// lease is absent or expired. Replay-pending unleased rows are included so a
    /// reconnect cannot freeze live tile painting. Rows waiting on retry backoff
    /// are skipped so a newest-unloaded flood cannot starve the player's loaded cell.
    ///
    /// Selection keeps a handful of newest revisions (fresh fly-in) and fills the
    /// rest from chunks nearest the centroid of recent dirties so a view-distance
    /// ring cannot starve the player's already-loaded interior.
    pub async fn dirty_page_for_owner(
        &self,
        bridge_id: &[u8],
        limit: usize,
        now: i64,
    ) -> Result<Vec<DirtyChunk>, RepositoryError> {
        if bridge_id.len() != MAX_BRIDGE_ID_BYTES || bridge_id.iter().all(|byte| *byte == 0) {
            return Err(ModelError::Bounds("bridge ID must be exactly 16 non-zero bytes").into());
        }
        if limit == 0 {
            return Ok(Vec::new());
        }
        let bridge_id = bridge_id.to_vec();
        let recency_limit = i64::try_from(limit.min(8)).unwrap_or(8);
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.blocking(move |connection| {
            let mut statement = connection.prepare(
                "WITH eligible AS (
                    SELECT dirty_chunks.namespace, dirty_chunks.value, dirty_chunks.epoch,
                           dirty_chunks.x, dirty_chunks.z, dirty_chunks.revision,
                           dirty_chunks.owner_bridge_id, dirty_chunks.lease_expires_epoch_seconds
                    FROM dirty_chunks
                    INNER JOIN worlds USING(namespace,value)
                    LEFT JOIN dirty_retries USING(namespace,value,epoch,x,z)
                    WHERE worlds.epoch >= 0 AND dirty_chunks.epoch=worlds.epoch
                      AND dirty_chunks.owner_bridge_id=?1
                      AND (dirty_chunks.lease_expires_epoch_seconds=0 OR dirty_chunks.lease_expires_epoch_seconds<=?2)
                      AND (dirty_retries.next_attempt IS NULL OR dirty_retries.next_attempt<=?2)
                 ),
                 newest AS (
                    SELECT x, z, revision FROM eligible
                    ORDER BY revision DESC, namespace, value, epoch, x, z
                    LIMIT 64
                 ),
                 anchor AS (
                    SELECT AVG(x * 1.0) AS cx, AVG(z * 1.0) AS cz,
                           (SELECT MIN(revision) FROM (
                               SELECT revision FROM newest ORDER BY revision DESC LIMIT ?4
                           )) AS newest_floor
                    FROM newest
                 )
                 SELECT length(CAST(eligible.namespace AS BLOB)),COALESCE(substr(CAST(eligible.namespace AS BLOB),1,4097),zeroblob(0)),
                        length(CAST(eligible.value AS BLOB)),COALESCE(substr(CAST(eligible.value AS BLOB),1,4097),zeroblob(0)),
                        eligible.epoch,eligible.x,eligible.z,eligible.revision,
                        length(CAST(eligible.owner_bridge_id AS BLOB)),COALESCE(substr(CAST(eligible.owner_bridge_id AS BLOB),1,17),zeroblob(0)),
                        eligible.lease_expires_epoch_seconds
                 FROM eligible
                 CROSS JOIN anchor
                 ORDER BY
                   CASE WHEN eligible.revision >= COALESCE(anchor.newest_floor, eligible.revision) THEN 0 ELSE 1 END ASC,
                   CASE WHEN eligible.revision >= COALESCE(anchor.newest_floor, eligible.revision) THEN 0
                        ELSE (eligible.x - COALESCE(anchor.cx, eligible.x)) * (eligible.x - COALESCE(anchor.cx, eligible.x))
                           + (eligible.z - COALESCE(anchor.cz, eligible.z)) * (eligible.z - COALESCE(anchor.cz, eligible.z))
                   END ASC,
                   eligible.revision DESC,eligible.namespace,eligible.value,eligible.epoch,eligible.x,eligible.z
                 LIMIT ?3",
            )?;
            let rows = statement.query_map(params![bridge_id, now, limit, recency_limit], |row| {
                let world = WorldId::new(
                    bounded_text(row.get(0)?, row.get(1)?, MAX_TEXT_BYTES)?,
                    bounded_text(row.get(2)?, row.get(3)?, MAX_TEXT_BYTES)?,
                    checked_u64(row.get(4)?, "dirty epoch").map_err(|_| rusqlite::Error::InvalidQuery)?,
                );
                world.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
                let owner_bridge_id = bounded_blob(row.get(8)?, row.get(9)?, MAX_BRIDGE_ID_BYTES)?;
                if owner_bridge_id.len() != MAX_BRIDGE_ID_BYTES
                    || owner_bridge_id.iter().all(|byte| *byte == 0)
                {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                Ok(DirtyChunk {
                    world,
                    coordinate: ChunkCoordinate { x: row.get(5)?, z: row.get(6)? },
                    revision: checked_u64(row.get(7)?, "dirty revision").map_err(|_| rusqlite::Error::InvalidQuery)?,
                    owner_bridge_id,
                    lease_expires_epoch_seconds: row.get(10)?,
                })
            })?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }).await
    }

    /// Same as [`Self::dirty_page_for_owner`], but when `focus` is set the page
    /// is the dirty chunks nearest that chunk rather than the newest cluster.
    pub async fn dirty_page_for_owner_near(
        &self,
        bridge_id: &[u8],
        limit: usize,
        now: i64,
        focus: Option<(i32, i32)>,
    ) -> Result<Vec<DirtyChunk>, RepositoryError> {
        let Some((focus_x, focus_z)) = focus else {
            return self.dirty_page_for_owner(bridge_id, limit, now).await;
        };
        if bridge_id.len() != MAX_BRIDGE_ID_BYTES || bridge_id.iter().all(|byte| *byte == 0) {
            return Err(ModelError::Bounds("bridge ID must be exactly 16 non-zero bytes").into());
        }
        if limit == 0 {
            return Ok(Vec::new());
        }
        let bridge_id = bridge_id.to_vec();
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.blocking(move |connection| {
            let mut statement = connection.prepare(
                "SELECT length(CAST(dirty_chunks.namespace AS BLOB)),COALESCE(substr(CAST(dirty_chunks.namespace AS BLOB),1,4097),zeroblob(0)),
                        length(CAST(dirty_chunks.value AS BLOB)),COALESCE(substr(CAST(dirty_chunks.value AS BLOB),1,4097),zeroblob(0)),
                        dirty_chunks.epoch,dirty_chunks.x,dirty_chunks.z,dirty_chunks.revision,
                        length(CAST(dirty_chunks.owner_bridge_id AS BLOB)),COALESCE(substr(CAST(dirty_chunks.owner_bridge_id AS BLOB),1,17),zeroblob(0)),
                        dirty_chunks.lease_expires_epoch_seconds
                 FROM dirty_chunks
                 INNER JOIN worlds USING(namespace,value)
                 LEFT JOIN dirty_retries USING(namespace,value,epoch,x,z)
                 WHERE worlds.epoch >= 0 AND dirty_chunks.epoch=worlds.epoch
                   AND dirty_chunks.owner_bridge_id=?1
                   AND (dirty_chunks.lease_expires_epoch_seconds=0 OR dirty_chunks.lease_expires_epoch_seconds<=?2)
                   AND (dirty_retries.next_attempt IS NULL OR dirty_retries.next_attempt<=?2)
                 ORDER BY
                   (dirty_chunks.x - ?4) * (dirty_chunks.x - ?4)
                     + (dirty_chunks.z - ?5) * (dirty_chunks.z - ?5) ASC,
                   dirty_chunks.revision DESC,dirty_chunks.namespace,dirty_chunks.value,dirty_chunks.epoch,dirty_chunks.x,dirty_chunks.z
                 LIMIT ?3",
            )?;
            let rows = statement.query_map(params![bridge_id, now, limit, focus_x, focus_z], |row| {
                let world = WorldId::new(
                    bounded_text(row.get(0)?, row.get(1)?, MAX_TEXT_BYTES)?,
                    bounded_text(row.get(2)?, row.get(3)?, MAX_TEXT_BYTES)?,
                    checked_u64(row.get(4)?, "dirty epoch").map_err(|_| rusqlite::Error::InvalidQuery)?,
                );
                world.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
                let owner_bridge_id = bounded_blob(row.get(8)?, row.get(9)?, MAX_BRIDGE_ID_BYTES)?;
                if owner_bridge_id.len() != MAX_BRIDGE_ID_BYTES
                    || owner_bridge_id.iter().all(|byte| *byte == 0)
                {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                Ok(DirtyChunk {
                    world,
                    coordinate: ChunkCoordinate { x: row.get(5)?, z: row.get(6)? },
                    revision: checked_u64(row.get(7)?, "dirty revision").map_err(|_| rusqlite::Error::InvalidQuery)?,
                    owner_bridge_id,
                    lease_expires_epoch_seconds: row.get(10)?,
                })
            })?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }).await
    }

    /// Atomically leases one unleased, unreplayed row to the given bridge, marking
    /// it replay-pending until completion; returns the leased row or None.
    pub async fn assign_dirty_lease(
        &self,
        bridge_id: &[u8],
        limit: usize,
        lease_expires_epoch_seconds: i64,
    ) -> Result<Option<DirtyLease>, RepositoryError> {
        self.assign_dirty_lease_at(bridge_id, limit, lease_expires_epoch_seconds, now_seconds()).await
    }

    /// Extracts one eligible dirty row for `bridge_id` under an explicit clock.
    pub async fn assign_dirty_lease_at(
        &self,
        bridge_id: &[u8],
        limit: usize,
        lease_expires_epoch_seconds: i64,
        now: i64,
    ) -> Result<Option<DirtyLease>, RepositoryError> {
        if bridge_id.len() != MAX_BRIDGE_ID_BYTES || bridge_id.iter().all(|byte| *byte == 0) {
            return Err(ModelError::Bounds("bridge ID must be exactly 16 non-zero bytes").into());
        }
        if limit == 0 || lease_expires_epoch_seconds < 0 {
            return Err(ModelError::Bounds("lease limit and expiry must be positive").into());
        }
        let bridge_id = bridge_id.to_vec();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            let row: Option<(WorldId, ChunkCoordinate, i64, Vec<u8>)> = transaction
                .query_row(
                    "SELECT dirty_chunks.namespace,dirty_chunks.value,dirty_chunks.epoch,dirty_chunks.x,dirty_chunks.z,dirty_chunks.revision,dirty_chunks.owner_session_id FROM dirty_chunks
                     INNER JOIN worlds USING(namespace,value)
                     WHERE worlds.epoch >= 0 AND dirty_chunks.epoch=worlds.epoch
                       AND (dirty_chunks.replay_pending=0
                            OR (dirty_chunks.replay_pending=1 AND dirty_chunks.lease_expires_epoch_seconds>0 AND dirty_chunks.lease_expires_epoch_seconds<=?1))
                     ORDER BY dirty_chunks.revision DESC,dirty_chunks.namespace,dirty_chunks.value,dirty_chunks.epoch,dirty_chunks.x,dirty_chunks.z
                     LIMIT 1",
                    params![now],
                    |row| {
                        let world = WorldId::new(
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            checked_u64(row.get::<_, i64>(2)?, "dirty epoch").map_err(|_| rusqlite::Error::InvalidQuery)?,
                        );
                        Ok((
                            world,
                            ChunkCoordinate { x: row.get(3)?, z: row.get(4)? },
                            row.get::<_, i64>(5)?,
                            row.get::<_, Vec<u8>>(6)?,
                        ))
                    },
                )
                .optional()?;
            let Some((world, coordinate, revision, owner_session_id)) = row else {
                transaction.commit()?;
                return Ok(None);
            };
            if owner_session_id.len() != MAX_SESSION_ID_BYTES || owner_session_id.iter().all(|byte| *byte == 0) {
                return Err(RepositoryError::Schema("dirty row contains invalid owner session".into()));
            }
            transaction.execute(
                "UPDATE dirty_chunks SET owner_bridge_id=?1,lease_expires_epoch_seconds=?2,replay_pending=1
                 WHERE namespace=?3 AND value=?4 AND epoch=?5 AND x=?6 AND z=?7
                   AND (replay_pending=0 OR (replay_pending=1 AND lease_expires_epoch_seconds>0 AND lease_expires_epoch_seconds<=?8))",
                params![bridge_id, lease_expires_epoch_seconds, world.namespace, world.value, world.epoch as i64, coordinate.x, coordinate.z, now],
            )?;
            transaction.commit()?;
            Ok(Some(DirtyLease {
                world,
                coordinate,
                revision: checked_u64(revision, "dirty revision")?,
                owner_bridge_id: bridge_id,
                owner_session_id,
                lease_expires_epoch_seconds,
            }))
        }).await
    }

    /// Returns every row owned by `bridge_id` for bounded replay after reconnect.
    pub async fn dirty_rows_for_replay(&self, bridge_id: &[u8]) -> Result<Vec<DirtyRow>, RepositoryError> {
        if bridge_id.len() != MAX_BRIDGE_ID_BYTES || bridge_id.iter().all(|byte| *byte == 0) {
            return Err(ModelError::Bounds("bridge ID must be exactly 16 non-zero bytes").into());
        }
        let bridge_id = bridge_id.to_vec();
        self.blocking(move |connection| {
            let mut statement = connection.prepare(
                "SELECT dirty_chunks.namespace,dirty_chunks.value,dirty_chunks.epoch,dirty_chunks.x,dirty_chunks.z,dirty_chunks.revision FROM dirty_chunks
                 INNER JOIN worlds USING(namespace,value)
                 WHERE worlds.epoch >= 0 AND dirty_chunks.epoch=worlds.epoch
                   AND dirty_chunks.owner_bridge_id=?1
                 ORDER BY dirty_chunks.namespace,dirty_chunks.value,dirty_chunks.epoch,dirty_chunks.x,dirty_chunks.z",
            )?;
            let rows = statement.query_map(params![bridge_id], |row| {
                let world = WorldId::new(
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    checked_u64(row.get::<_, i64>(2)?, "dirty epoch").map_err(|_| rusqlite::Error::InvalidQuery)?,
                );
                world.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
                Ok(DirtyRow {
                    world,
                    coordinate: ChunkCoordinate { x: row.get(3)?, z: row.get(4)? },
                    revision: checked_u64(row.get(5)?, "dirty revision").map_err(|_| rusqlite::Error::InvalidQuery)?,
                })
            })?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }).await
    }

    /// Returns one bounded page of replay-pending rows owned by the bridge/session
    /// for a single world, using a keyset cursor over (x, z). The limit is treated
    /// as `limit + 1` so the caller can detect `has_more` without a count query.
    pub async fn dirty_replay_page(
        &self,
        bridge_id: &[u8],
        session_id: &[u8],
        world: &WorldId,
        cursor: Option<&ChunkCoordinate>,
        limit: usize,
    ) -> Result<Vec<DirtyRow>, RepositoryError> {
        if bridge_id.len() != MAX_BRIDGE_ID_BYTES || bridge_id.iter().all(|byte| *byte == 0) {
            return Err(ModelError::Bounds("bridge ID must be exactly 16 non-zero bytes").into());
        }
        if session_id.len() != MAX_SESSION_ID_BYTES || session_id.iter().all(|byte| *byte == 0) {
            return Err(ModelError::Bounds("session ID must be exactly 16 non-zero bytes").into());
        }
        world.validate()?;
        if limit == 0 {
            return Err(ModelError::Bounds("limit must be positive").into());
        }
        let bridge_id = bridge_id.to_vec();
        let session_id = session_id.to_vec();
        let world = world.clone();
        let (cursor_x, cursor_z) = cursor.map(|c| (Some(c.x), Some(c.z))).unwrap_or((None, None));
        let cursor_x = cursor_x.map(i64::from);
        let cursor_z = cursor_z.map(i64::from);
        let limit = i64::try_from(limit.checked_add(1).unwrap_or(usize::MAX)).unwrap_or(i64::MAX);
        self.blocking(move |connection| {
            let mut statement = connection.prepare(
                "SELECT dirty_chunks.namespace,dirty_chunks.value,dirty_chunks.epoch,dirty_chunks.x,dirty_chunks.z,dirty_chunks.revision
                 FROM dirty_chunks
                 INNER JOIN worlds USING(namespace,value)
                 WHERE worlds.epoch >= 0 AND dirty_chunks.epoch=worlds.epoch
                   AND dirty_chunks.owner_bridge_id=?1
                   AND dirty_chunks.owner_session_id=?2
                   AND dirty_chunks.replay_pending=1
                   AND dirty_chunks.namespace=?3 AND dirty_chunks.value=?4 AND dirty_chunks.epoch=?5
                   AND (?6 IS NULL OR (dirty_chunks.x > ?6 OR (dirty_chunks.x = ?6 AND dirty_chunks.z > ?7)))
                 ORDER BY dirty_chunks.x, dirty_chunks.z
                 LIMIT ?8",
            )?;
            let rows = statement.query_map(
                params![
                    bridge_id,
                    session_id,
                    world.namespace,
                    world.value,
                    world.epoch as i64,
                    cursor_x,
                    cursor_z,
                    limit,
                ],
                |row| {
                    let world = WorldId::new(
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        checked_u64(row.get::<_, i64>(2)?, "dirty epoch").map_err(|_| rusqlite::Error::InvalidQuery)?,
                    );
                    world.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
                    Ok(DirtyRow {
                        world,
                        coordinate: ChunkCoordinate { x: row.get(3)?, z: row.get(4)? },
                        revision: checked_u64(row.get(5)?, "dirty revision").map_err(|_| rusqlite::Error::InvalidQuery)?,
                    })
                },
            )?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }).await
    }

    /// On authenticated reconnect, moves every row owned by `bridge_id` to the
    /// the checkpoint session without changing the durable watermark.
    pub async fn reassign_bridge_lease(
        &self,
        bridge_id: &[u8],
        session_id: &[u8],
    ) -> Result<(), RepositoryError> {
        if bridge_id.len() != MAX_BRIDGE_ID_BYTES || bridge_id.iter().all(|byte| *byte == 0) {
            return Err(ModelError::Bounds("bridge ID must be exactly 16 non-zero bytes").into());
        }
        if session_id.len() != MAX_SESSION_ID_BYTES || session_id.iter().all(|byte| *byte == 0) {
            return Err(ModelError::Bounds("session ID must be exactly 16 non-zero bytes").into());
        }
        let bridge_id = bridge_id.to_vec();
        let session_id = session_id.to_vec();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            let durable_sequence: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(durable_sequence),0) FROM bridge_checkpoints WHERE bridge_id=?1",
                params![bridge_id],
                |row| row.get(0),
            ).unwrap_or(0);
            transaction.execute(
                "UPDATE dirty_chunks
                 SET owner_session_id=?1,lease_expires_epoch_seconds=0,replay_pending=1
                 WHERE owner_bridge_id=?2
                   AND (namespace,value,epoch) IN (SELECT namespace,value,epoch FROM worlds WHERE epoch >= 0)",
                params![session_id, bridge_id],
            )?;
            transaction.execute(
                "INSERT INTO bridge_checkpoints(bridge_id,session_id,durable_sequence) VALUES(?1,?2,?3)
                 ON CONFLICT(bridge_id) DO UPDATE SET session_id=excluded.session_id,durable_sequence=MAX(durable_sequence,excluded.durable_sequence)",
                params![bridge_id, session_id, durable_sequence],
            )?;
            transaction.commit()?;
            Ok(())
        }).await
    }

    /// Owner-scoped completion removes exactly one owned row and its retry state.
    pub async fn complete_dirty_for_owner(
        &self,
        world: &WorldId,
        coordinate: ChunkCoordinate,
        revision: u64,
        bridge_id: &[u8],
    ) -> Result<bool, RepositoryError> {
        world.validate()?;
        if bridge_id.len() != MAX_BRIDGE_ID_BYTES || bridge_id.iter().all(|byte| *byte == 0) {
            return Err(ModelError::Bounds("bridge ID must be exactly 16 non-zero bytes").into());
        }
        let revision = checked_i64(revision, "dirty revision")?;
        let world = world.clone();
        let bridge_id = bridge_id.to_vec();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            ensure_current_world(&transaction, &world)?;
            let changed = transaction.execute(
                "DELETE FROM dirty_chunks WHERE namespace=?1 AND value=?2 AND epoch=?3 AND x=?4 AND z=?5
                   AND owner_bridge_id=?6 AND revision <= ?7",
                params![world.namespace, world.value, world.epoch as i64, coordinate.x, coordinate.z, bridge_id, revision],
            )?;
            if changed != 0 {
                transaction.execute(
                    "DELETE FROM dirty_retries WHERE namespace=?1 AND value=?2 AND epoch=?3 AND x=?4 AND z=?5",
                    params![world.namespace, world.value, world.epoch as i64, coordinate.x, coordinate.z],
                )?;
            }
            transaction.commit()?;
            Ok(changed != 0)
        }).await
    }

    /// Owner-scoped deferral with retry backoff for one owned row.
    pub async fn defer_dirty_for_owner(
        &self,
        world: &WorldId,
        coordinate: ChunkCoordinate,
        revision: u64,
        bridge_id: &[u8],
        now: i64,
    ) -> Result<(), RepositoryError> {
        world.validate()?;
        if bridge_id.len() != MAX_BRIDGE_ID_BYTES || bridge_id.iter().all(|byte| *byte == 0) {
            return Err(ModelError::Bounds("bridge ID must be exactly 16 non-zero bytes").into());
        }
        let revision = checked_i64(revision, "dirty revision")?;
        let world = world.clone();
        let bridge_id = bridge_id.to_vec();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            ensure_current_world(&transaction, &world)?;
            let attempt: i64 = transaction.query_row(
                "SELECT COALESCE(attempt, 0) FROM dirty_retries WHERE namespace=?1 AND value=?2 AND epoch=?3 AND x=?4 AND z=?5",
                params![world.namespace, world.value, world.epoch as i64, coordinate.x, coordinate.z],
                |row| row.get(0),
            ).optional()?.unwrap_or(0);
            let next_attempt = attempt.saturating_add(1).min(16);
            let delay = 1_i64.checked_shl(next_attempt as u32).unwrap_or(86_400).min(86_400);
            transaction.execute(
                "INSERT INTO dirty_retries(namespace,value,epoch,x,z,attempt,next_attempt) VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(namespace,value,epoch,x,z) DO UPDATE SET attempt=excluded.attempt,next_attempt=excluded.next_attempt
                 WHERE EXISTS (SELECT 1 FROM dirty_chunks WHERE namespace=?1 AND value=?2 AND epoch=?3 AND x=?4 AND z=?5
                   AND owner_bridge_id=?9 AND revision<=?8)",
                params![world.namespace, world.value, world.epoch as i64, coordinate.x, coordinate.z, next_attempt, now.saturating_add(delay), revision, bridge_id],
            )?;
            transaction.commit()?;
            Ok(())
        }).await
    }

    pub async fn recover(&self) -> Result<Recovery, RepositoryError> {
        self.blocking(|connection| recover_connection(connection)).await
    }

    pub async fn import_legacy_state(&self, world: &WorldId, directory: impl AsRef<Path>) -> Result<(), RepositoryError> {
        world.validate()?;
        let world = world.clone();
        let directory = directory.as_ref().to_path_buf();
        self.blocking(move |connection| {
            let parsed = parse_legacy_files(&directory).map_err(|error| RepositoryError::InvalidLegacy { path: error.path, message: error.message })?;
            if parsed.files().next().is_none() { return Ok(()); }
            import_parsed(connection, &world, &parsed)
        }).await
    }
    pub async fn pragma_values(&self) -> Result<(String, i64, i64, i64, i64), RepositoryError> {
        self.blocking(|connection| {
            Ok((
                connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?,
                connection.query_row("PRAGMA synchronous", [], |row| row.get(0))?,
                connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?,
                connection.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?,
                connection.query_row("PRAGMA application_id", [], |row| row.get(0))?,
            ))
        }).await
    }
}
fn now_seconds() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)).unwrap_or(0)
}

fn open_connection(path: &Path) -> Result<Connection, RepositoryError> {
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let existed_nonempty = std::fs::metadata(path).map(|metadata| metadata.len() != 0).unwrap_or(false);
    let mut connection = Connection::open(path)?;
    let object_count: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type IN ('table','index','trigger','view') AND name NOT LIKE 'sqlite_%'", [], |row| row.get(0))?;
    let application_id: i64 = connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    if object_count == 0 {
        if existed_nonempty { return Err(RepositoryError::Schema("nonempty database has no supported schema".into())); }
        if application_id != 0 && application_id != APPLICATION_ID { return Err(RepositoryError::Schema("wrong application ID".into())); }
        let transaction = connection.transaction()?;
        transaction.execute_batch("PRAGMA application_id=0x53514D50;")?;
        transaction.execute_batch(SCHEMA)?;
        transaction.execute_batch(RETRY_MIGRATION)?;
        transaction.execute_batch(OWNER_MIGRATION)?;
        transaction.commit()?;
    } else {
        if application_id != APPLICATION_ID { return Err(RepositoryError::Schema("wrong application ID".into())); }
        let version: i64 = connection.query_row("SELECT version FROM schema_version", [], |row| row.get(0))
            .map_err(|_| RepositoryError::Schema("schema_version table missing".into()))?;
        if version == 1 || version == 2 {
            let transaction = connection.transaction()?;
            let current: i64 = transaction.query_row("SELECT version FROM schema_version", [], |row| row.get(0))
                .map_err(|_| RepositoryError::Schema("schema_version table missing".into()))?;
            if current == 1 {
                transaction.execute_batch(RETRY_MIGRATION)?;
                transaction.execute_batch(OWNER_MIGRATION)?;
            } else if current == 2 {
                transaction.execute_batch("ALTER TABLE session_checkpoints RENAME TO legacy_session_checkpoints; CREATE TABLE bridge_checkpoints(bridge_id BLOB PRIMARY KEY, session_id BLOB NOT NULL, durable_sequence INTEGER NOT NULL); UPDATE schema_version SET version=3;")?;
                transaction.execute_batch(OWNER_MIGRATION)?;
            } else if current == 3 {
                transaction.execute_batch(OWNER_MIGRATION)?;
            } else {
                transaction.rollback().ok();
                return Err(RepositoryError::Schema(format!("unsupported schema version {current}")));
            }
            transaction.commit()?;
        } else if version == 3 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(OWNER_MIGRATION)?;
            transaction.commit()?;
        } else if version != SCHEMA_VERSION {
            return Err(RepositoryError::Schema(format!("unsupported schema version {version}")));
        }
    }
    configure_and_validate(&connection)?;
    validate_schema(&connection)?;
    Ok(connection)
}

fn configure_and_validate(connection: &Connection) -> Result<(), RepositoryError> {
    let journal: String = connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    if !journal.eq_ignore_ascii_case("wal") { return Err(RepositoryError::Schema("WAL is required".into())); }
    connection.execute_batch("PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
    let synchronous: i64 = connection.query_row("PRAGMA synchronous", [], |row| row.get(0))?;
    let foreign_keys: i64 = connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    let busy_timeout: i64 = connection.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
    if synchronous != 1 || foreign_keys != 1 || busy_timeout != 5000 { return Err(RepositoryError::Schema("required pragmas are not active".into())); }
    let app_id: i64 = connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    if app_id != APPLICATION_ID { return Err(RepositoryError::Schema("wrong application ID".into())); }
    Ok(())
}
fn validate_schema(connection: &Connection) -> Result<(), RepositoryError> {
    let count: i64 = connection.query_row("SELECT count(*) FROM schema_version", [], |row| row.get(0)).map_err(|_| RepositoryError::Schema("schema_version table missing".into()))?;
    if count != 1 { return Err(RepositoryError::Schema("schema_version must contain exactly one row".into())); }
    let version: i64 = connection.query_row("SELECT version FROM schema_version", [], |row| row.get(0))?;
    if version != SCHEMA_VERSION { return Err(RepositoryError::Schema(format!("unsupported schema version {version}"))); }
    let expected_tables = ["schema_version", "worlds", "dirty_chunks", "render_jobs", "legacy_session_checkpoints", "bridge_checkpoints", "legacy_imports", "dirty_retries"];
    let mut statement = connection.prepare("SELECT type,name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name")?;
    let entries: Vec<(String, String)> = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?;
    if entries.iter().any(|(kind, name)| kind != "table" || !expected_tables.contains(&name.as_str())) || entries.iter().filter(|(kind, _)| kind == "table").count() != expected_tables.len() {
        return Err(RepositoryError::Schema("schema objects do not match checked-in migration".into()));
    }
    let expected_columns: [(&str, &[(&str, &str, i64, i64)]); 8] = [
        ("schema_version", &[("version", "INTEGER", 0, 1)]),
        ("worlds", &[("namespace", "TEXT", 1, 1), ("value", "TEXT", 1, 2), ("epoch", "INTEGER", 1, 0), ("config", "BLOB", 1, 0)]),
        ("dirty_chunks", &[("namespace", "TEXT", 1, 1), ("value", "TEXT", 1, 2), ("epoch", "INTEGER", 1, 3), ("x", "INTEGER", 1, 4), ("z", "INTEGER", 1, 5), ("revision", "INTEGER", 1, 0), ("owner_bridge_id", "BLOB", 1, 0), ("owner_session_id", "BLOB", 1, 0), ("lease_expires_epoch_seconds", "INTEGER", 1, 0), ("replay_pending", "INTEGER", 1, 0)]),
        ("render_jobs", &[("id", "BLOB", 0, 1), ("namespace", "TEXT", 1, 0), ("value", "TEXT", 1, 0), ("epoch", "INTEGER", 1, 0), ("kind", "INTEGER", 1, 0), ("state", "INTEGER", 1, 0), ("payload", "BLOB", 1, 0), ("completed_chunks", "INTEGER", 1, 0)]),
        ("legacy_session_checkpoints", &[("session_id", "BLOB", 0, 1), ("durable_sequence", "INTEGER", 1, 0)]),
        ("bridge_checkpoints", &[("bridge_id", "BLOB", 0, 1), ("session_id", "BLOB", 1, 0), ("durable_sequence", "INTEGER", 1, 0)]),
        ("legacy_imports", &[("relative_path", "TEXT", 0, 1), ("content_sha256", "BLOB", 1, 0), ("imported_at_epoch_seconds", "INTEGER", 1, 0)]),
        ("dirty_retries", &[("namespace", "TEXT", 1, 1), ("value", "TEXT", 1, 2), ("epoch", "INTEGER", 1, 3), ("x", "INTEGER", 1, 4), ("z", "INTEGER", 1, 5), ("attempt", "INTEGER", 1, 0), ("next_attempt", "INTEGER", 1, 0)]),
    ];
    for (table, expected) in expected_columns {
        let mut columns = connection.prepare(&format!("PRAGMA table_info({table})"))?;
        let actual: Vec<(String, String, i64, i64)> = columns.query_map([], |row| Ok((row.get(1)?, row.get(2)?, row.get(3)?, row.get(5)?)))?.collect::<Result<_, _>>()?;
        let expected: Vec<_> = expected.iter().map(|(name, kind, notnull, pk)| ((*name).to_string(), (*kind).to_string(), *notnull, *pk)).collect();
        if actual != expected { return Err(RepositoryError::Schema(format!("table {table} structure does not match migration"))); }
    }
    Ok(())
}

fn decode_tombstone(value: i64) -> Result<u64, RepositoryError> {
    if value >= 0 { return Err(RepositoryError::Schema("expected removed epoch tombstone".into())); }
    u64::try_from(-1 - value).map_err(|_| RepositoryError::Schema("invalid removed epoch tombstone".into()))
}

fn ensure_current_world(transaction: &Transaction<'_>, world: &WorldId) -> Result<(), RepositoryError> {
    let current: Option<i64> = transaction.query_row("SELECT epoch FROM worlds WHERE namespace=?1 AND value=?2", params![world.namespace, world.value], |row| row.get(0)).optional()?;
    match current {
        Some(epoch) if epoch >= 0 && epoch == world.epoch as i64 => Ok(()),
        Some(_) => Err(RepositoryError::StaleWorld(world.clone())),
        None => Err(RepositoryError::MissingWorld(world.clone())),
    }
}

fn recover_connection(connection: &mut Connection) -> Result<Recovery, RepositoryError> {
    let worlds = {
        let mut statement = connection.prepare("SELECT length(CAST(namespace AS BLOB)),COALESCE(substr(CAST(namespace AS BLOB),1,4097),zeroblob(0)),length(CAST(value AS BLOB)),COALESCE(substr(CAST(value AS BLOB),1,4097),zeroblob(0)),epoch,length(config),COALESCE(substr(config,1,16777217),zeroblob(0)) FROM worlds WHERE epoch >= 0 ORDER BY namespace,value")?;
        statement.query_map([], |row| {
            let namespace = bounded_text(row.get(0)?, row.get(1)?, MAX_TEXT_BYTES)?;
            let value = bounded_text(row.get(2)?, row.get(3)?, MAX_TEXT_BYTES)?;
            let config = bounded_blob(row.get(5)?, row.get(6)?, MAX_CONFIG_BYTES)?;
            let world = World { id: WorldId::new(namespace, value, checked_u64(row.get(4)?, "world epoch").map_err(|_| rusqlite::Error::InvalidQuery)?), config };
            world.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(world)
        })?.collect::<Result<Vec<_>, _>>()?
    };
    let dirty = {
        let mut statement = connection.prepare("SELECT length(CAST(dirty_chunks.namespace AS BLOB)),COALESCE(substr(CAST(dirty_chunks.namespace AS BLOB),1,4097),zeroblob(0)),length(CAST(dirty_chunks.value AS BLOB)),COALESCE(substr(CAST(dirty_chunks.value AS BLOB),1,4097),zeroblob(0)),dirty_chunks.epoch,dirty_chunks.x,dirty_chunks.z,dirty_chunks.revision,length(CAST(dirty_chunks.owner_bridge_id AS BLOB)),COALESCE(substr(CAST(dirty_chunks.owner_bridge_id AS BLOB),1,17),zeroblob(0)),dirty_chunks.lease_expires_epoch_seconds FROM dirty_chunks INNER JOIN worlds USING(namespace,value) WHERE worlds.epoch >= 0 AND dirty_chunks.epoch=worlds.epoch ORDER BY dirty_chunks.namespace,dirty_chunks.value,dirty_chunks.epoch,dirty_chunks.x,dirty_chunks.z")?;
        statement.query_map([], |row| {
            let world = WorldId::new(bounded_text(row.get(0)?, row.get(1)?, MAX_TEXT_BYTES)?, bounded_text(row.get(2)?, row.get(3)?, MAX_TEXT_BYTES)?, checked_u64(row.get(4)?, "dirty epoch").map_err(|_| rusqlite::Error::InvalidQuery)?);
            world.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
            let owner_bridge_id = bounded_blob(row.get(8)?, row.get(9)?, MAX_BRIDGE_ID_BYTES)?;
            if owner_bridge_id.len() != MAX_BRIDGE_ID_BYTES
                || owner_bridge_id.iter().all(|byte| *byte == 0)
            {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok(DirtyChunk { world, coordinate: ChunkCoordinate { x: row.get(5)?, z: row.get(6)? }, revision: checked_u64(row.get(7)?, "dirty revision").map_err(|_| rusqlite::Error::InvalidQuery)?, owner_bridge_id, lease_expires_epoch_seconds: row.get(10)? })
        })?.collect::<Result<Vec<_>, _>>()?
    };
    let jobs = {
        let mut statement = connection.prepare("SELECT length(CAST(render_jobs.id AS BLOB)),COALESCE(substr(CAST(render_jobs.id AS BLOB),1,33),zeroblob(0)),length(CAST(render_jobs.namespace AS BLOB)),COALESCE(substr(CAST(render_jobs.namespace AS BLOB),1,4097),zeroblob(0)),length(CAST(render_jobs.value AS BLOB)),COALESCE(substr(CAST(render_jobs.value AS BLOB),1,4097),zeroblob(0)),render_jobs.epoch,render_jobs.kind,render_jobs.state,length(render_jobs.payload),COALESCE(substr(render_jobs.payload,1,16777217),zeroblob(0)),render_jobs.completed_chunks FROM render_jobs INNER JOIN worlds USING(namespace,value) WHERE worlds.epoch >= 0 AND render_jobs.epoch=worlds.epoch AND render_jobs.state IN (0,1,2) ORDER BY render_jobs.id")?;
        statement.query_map([], |row| {
            let state = row.get::<_, i64>(8)?;
            let job = RenderJob { id: bounded_blob(row.get(0)?, row.get(1)?, MAX_JOB_ID_BYTES)?, world: WorldId::new(bounded_text(row.get(2)?, row.get(3)?, MAX_TEXT_BYTES)?, bounded_text(row.get(4)?, row.get(5)?, MAX_TEXT_BYTES)?, checked_u64(row.get(6)?, "job epoch").map_err(|_| rusqlite::Error::InvalidQuery)?), kind: decode_kind(row.get(7)?).map_err(|_| rusqlite::Error::InvalidQuery)?, state: if state == JobState::Running as i64 { JobState::Resumable } else { decode_state(state).map_err(|_| rusqlite::Error::InvalidQuery)? }, payload: bounded_blob(row.get(9)?, row.get(10)?, MAX_PAYLOAD_BYTES)?, completed_chunks: checked_u64(row.get(11)?, "completed chunks").map_err(|_| rusqlite::Error::InvalidQuery)? };
            job.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(job)
        })?.collect::<Result<Vec<_>, _>>()?
    };
    let checkpoints = {
        let mut statement = connection.prepare("SELECT length(CAST(bridge_id AS BLOB)),COALESCE(substr(CAST(bridge_id AS BLOB),1,17),zeroblob(0)),length(CAST(session_id AS BLOB)),COALESCE(substr(CAST(session_id AS BLOB),1,17),zeroblob(0)),durable_sequence FROM bridge_checkpoints ORDER BY bridge_id")?;
        statement.query_map([], |row| {
            let bridge_id = bounded_blob(row.get(0)?, row.get(1)?, MAX_BRIDGE_ID_BYTES)?;
            let session_id = bounded_blob(row.get(2)?, row.get(3)?, MAX_SESSION_ID_BYTES)?;
            if bridge_id.len() != MAX_BRIDGE_ID_BYTES
                || bridge_id.iter().all(|byte| *byte == 0)
                || session_id.len() != MAX_SESSION_ID_BYTES
            {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok(SessionCheckpoint { bridge_id, session_id, durable_sequence: checked_u64(row.get(4)?, "checkpoint").map_err(|_| rusqlite::Error::InvalidQuery)? })
        })?.collect::<Result<Vec<_>, _>>()?
    };
    Ok(Recovery { worlds, dirty, jobs, checkpoints })
}
fn decode_kind(value: i64) -> Result<JobKind, ()> { match value { 1 => Ok(JobKind::Full), 2 => Ok(JobKind::Resume), 3 => Ok(JobKind::Radius), _ => Err(()) } }
fn decode_state(value: i64) -> Result<JobState, ()> { match value { 0 => Ok(JobState::Queued), 1 => Ok(JobState::Running), 2 => Ok(JobState::Resumable), 3 => Ok(JobState::Completed), 4 => Ok(JobState::Failed), 5 => Ok(JobState::Cancelled), _ => Err(()) } }

pub(crate) fn deterministic_job_id(world: &WorldId, kind: JobKind) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(world.namespace.as_bytes());
    hash.update([0]);
    hash.update(world.value.as_bytes());
    hash.update([0]);
    hash.update(world.epoch.to_le_bytes());
    hash.update([kind as u8]);
    hash.finalize().to_vec()
}


fn legacy_key(world: &WorldId, filename: &str) -> String {
    let mut identity = Vec::with_capacity(8 + world.namespace.len() + world.value.len());
    identity.extend_from_slice(&(world.namespace.len() as u32).to_be_bytes());
    identity.extend_from_slice(world.namespace.as_bytes());
    identity.extend_from_slice(&(world.value.len() as u32).to_be_bytes());
    identity.extend_from_slice(world.value.as_bytes());
    identity.extend_from_slice(&world.epoch.to_be_bytes());
    format!("v1/{}/{}", hex::encode(identity), filename)
}
fn bounded_blob(length: i64, prefix: Vec<u8>, max: usize) -> Result<Vec<u8>, rusqlite::Error> {
    let length = usize::try_from(length).map_err(|_| rusqlite::Error::InvalidQuery)?;
    if length > max || prefix.len() != length { return Err(rusqlite::Error::InvalidQuery); }
    Ok(prefix)
}

fn bounded_text(length: i64, prefix: Vec<u8>, max: usize) -> Result<String, rusqlite::Error> {
    String::from_utf8(bounded_blob(length, prefix, max)?).map_err(|_| rusqlite::Error::InvalidQuery)
}
fn read_marker(transaction: &Transaction<'_>, key: &str) -> Result<Option<[u8; 32]>, RepositoryError> {
    let marker: Option<(i64, Vec<u8>)> = transaction.query_row("SELECT length(content_sha256),substr(content_sha256,1,33) FROM legacy_imports WHERE relative_path=?1", params![key], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
    let Some((length, prefix)) = marker else { return Ok(None); };
    let bytes = bounded_blob(length, prefix, 32)?;
    if bytes.len() != 32 { return Err(RepositoryError::Schema("legacy import marker hash must be exactly 32 bytes".into())); }
    let mut hash = [0_u8; 32];
    hash.copy_from_slice(&bytes);
    Ok(Some(hash))
}

fn import_parsed(connection: &mut Connection, world: &WorldId, parsed: &ParsedLegacy) -> Result<(), RepositoryError> {
    let transaction = connection.transaction()?;
    ensure_current_world(&transaction, world)?;
    let epoch = checked_i64(world.epoch, "world epoch")?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs().min(i64::MAX as u64) as i64;
    for file in parsed.files() {
        let relative_path = legacy_key(world, &file.relative_path);
        let prior = read_marker(&transaction, &relative_path)?;
        let ownership_key = legacy_key(world, "resume_render.payload.sha256");
        let ownership = if file.relative_path.ends_with("resume_render.json") { read_marker(&transaction, &ownership_key)? } else { None };
        if prior.as_ref().is_some_and(|hash| hash.as_slice() == file.sha256.as_slice()) { continue; }
        if file.relative_path.ends_with("dirty_chunks.json") {
            if !parsed.dirty.is_empty() {
                return Err(RepositoryError::InvalidLegacy { path: file.relative_path.clone().into(), message: "legacy dirty rows have no authenticated owner/session; import rejected".into() });
            }
        } else {
            let payload = parsed.resume_payload.clone().unwrap_or_default();
            let mut payload_hash = Sha256::new();
            payload_hash.update(&payload);
            let payload_hash = payload_hash.finalize().to_vec();
            let id = deterministic_job_id(world, JobKind::Resume);
            let existing: Option<(Vec<u8>, String, String, i64, i64, i64, Vec<u8>, i64)> = transaction.query_row(
                "SELECT
                    length(CAST(id AS BLOB)),
                    COALESCE(substr(CAST(id AS BLOB),1,33),zeroblob(0)),
                    length(CAST(namespace AS BLOB)),
                    COALESCE(substr(CAST(namespace AS BLOB),1,4097),zeroblob(0)),
                    length(CAST(value AS BLOB)),
                    COALESCE(substr(CAST(value AS BLOB),1,4097),zeroblob(0)),
                    epoch,
                    kind,
                    state,
                    length(payload),
                    COALESCE(substr(payload,1,16777217),zeroblob(0)),
                    completed_chunks
                 FROM render_jobs
                 WHERE id=?1",
                params![id],
                |row| {
                    Ok((
                        bounded_blob(row.get(0)?, row.get(1)?, MAX_JOB_ID_BYTES)?,
                        bounded_text(row.get(2)?, row.get(3)?, MAX_TEXT_BYTES)?,
                        bounded_text(row.get(4)?, row.get(5)?, MAX_TEXT_BYTES)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        bounded_blob(row.get(9)?, row.get(10)?, MAX_PAYLOAD_BYTES)?,
                        row.get(11)?,
                    ))
                },
            ).optional()?;
            match existing {
                None => {
                    transaction.execute("INSERT INTO render_jobs(id,namespace,value,epoch,kind,state,payload,completed_chunks) VALUES(?1,?2,?3,?4,?5,?6,?7,0)", params![id, world.namespace, world.value, epoch, JobKind::Resume as i64, JobState::Resumable as i64, payload])?;
                    transaction.execute("INSERT INTO legacy_imports(relative_path,content_sha256,imported_at_epoch_seconds) VALUES(?1,?2,?3) ON CONFLICT(relative_path) DO UPDATE SET content_sha256=excluded.content_sha256,imported_at_epoch_seconds=excluded.imported_at_epoch_seconds", params![ownership_key, payload_hash, now])?;
                }
                Some((existing_id, namespace, value, existing_epoch, kind, state, current_payload, progress)) => {
                    if existing_id != id || namespace != world.namespace || value != world.value || existing_epoch != epoch || kind != JobKind::Resume as i64 {
                        return Err(RepositoryError::Schema("conflicting legacy resume job identity".into()));
                    }
                    if decode_state(state).is_err() || progress < 0 {
                        return Err(RepositoryError::Schema("existing legacy resume job row is malformed".into()));
                    }
                    let mut current_hash = Sha256::new();
                    current_hash.update(&current_payload);
                    let owned_and_untouched = ownership.as_ref().is_some_and(|hash| hash.as_slice() == current_hash.finalize().as_slice()) && state == JobState::Resumable as i64 && progress == 0;
                    if owned_and_untouched {
                        transaction.execute("UPDATE render_jobs SET payload=?1,state=?2,completed_chunks=0 WHERE id=?3", params![payload, JobState::Resumable as i64, id])?;
                        transaction.execute("INSERT INTO legacy_imports(relative_path,content_sha256,imported_at_epoch_seconds) VALUES(?1,?2,?3) ON CONFLICT(relative_path) DO UPDATE SET content_sha256=excluded.content_sha256,imported_at_epoch_seconds=excluded.imported_at_epoch_seconds", params![ownership_key, payload_hash, now])?;
                    }
                }
            }
        }
        transaction.execute("INSERT INTO legacy_imports(relative_path,content_sha256,imported_at_epoch_seconds) VALUES(?1,?2,?3) ON CONFLICT(relative_path) DO UPDATE SET content_sha256=excluded.content_sha256,imported_at_epoch_seconds=excluded.imported_at_epoch_seconds", params![relative_path, file.sha256, now])?;
    }
    transaction.commit()?;
    Ok(())
}
