use crate::legacy_import::{parse_legacy_files, ParsedLegacy};
use crate::model::*;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const APPLICATION_ID: i64 = 0x5351_4d50;
const SCHEMA_VERSION: i64 = 1;
const SCHEMA: &str = include_str!("../migrations/0001_initial.sql");
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

    pub async fn mark_dirty(&self, world: &WorldId, coordinate: ChunkCoordinate, revision: u64, session_id: &[u8], sequence: u64) -> Result<bool, RepositoryError> {
        world.validate()?;
        if session_id.len() != MAX_SESSION_ID_BYTES { return Err(ModelError::Bounds("session ID must be exactly 16 bytes").into()); }
        let revision = checked_i64(revision, "dirty revision")?;
        let sequence = checked_i64(sequence, "session sequence")?;
        let world = world.clone();
        let session_id = session_id.to_vec();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            ensure_current_world(&transaction, &world)?;
            let changed = transaction.execute("INSERT INTO dirty_chunks(namespace,value,epoch,x,z,revision) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(namespace,value,epoch,x,z) DO UPDATE SET revision=excluded.revision WHERE excluded.revision > dirty_chunks.revision", params![world.namespace, world.value, world.epoch as i64, coordinate.x, coordinate.z, revision])?;
            transaction.execute("INSERT INTO session_checkpoints(session_id,durable_sequence) VALUES(?1,?2) ON CONFLICT(session_id) DO UPDATE SET durable_sequence=excluded.durable_sequence WHERE excluded.durable_sequence > session_checkpoints.durable_sequence", params![session_id, sequence])?;
            transaction.commit()?;
            Ok(changed != 0)
        }).await
    }

    pub async fn complete_dirty(&self, world: &WorldId, coordinate: ChunkCoordinate, revision: u64) -> Result<bool, RepositoryError> {
        world.validate()?;
        let revision = checked_i64(revision, "dirty revision")?;
        let world = world.clone();
        self.blocking(move |connection| {
            let transaction = connection.transaction()?;
            ensure_current_world(&transaction, &world)?;
            let changed = transaction.execute("DELETE FROM dirty_chunks WHERE namespace=?1 AND value=?2 AND epoch=?3 AND x=?4 AND z=?5 AND revision <= ?6", params![world.namespace, world.value, world.epoch as i64, coordinate.x, coordinate.z, revision])?;
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
            let existing: Option<(String, String, i64, i64, i64, Vec<u8>, i64)> = transaction.query_row("SELECT namespace,value,epoch,kind,state,payload,completed_chunks FROM render_jobs WHERE id=?1", params![job.id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?))).optional()?;
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
            let existing: Option<(i64, i64, i64, Vec<u8>)> = transaction.query_row("SELECT kind,state,completed_chunks,payload FROM render_jobs WHERE id=?1 AND namespace=?2 AND value=?3 AND epoch=?4", params![job.id, job.world.namespace, job.world.value, job.world.epoch as i64], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).optional()?;
            let Some((kind, state, progress, payload)) = existing else { return Err(RepositoryError::Schema("render job does not exist".into())); };
            if decode_kind(kind).is_err() || kind != job.kind as i64 { return Err(RepositoryError::Schema("render job kind does not match ID".into())); }
            if decode_state(state).is_err() || progress < 0 || payload.len() > MAX_PAYLOAD_BYTES { return Err(RepositoryError::Schema("existing render job row is malformed".into())); }
            let exact = state == job.state as i64 && progress == job.completed_chunks as i64 && payload == job.payload;
            if (state == JobState::Completed as i64 || state == JobState::Failed as i64) && !exact { return Err(RepositoryError::Schema("terminal render job cannot be changed".into())); }
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
        transaction.commit()?;
    } else {
        if application_id != APPLICATION_ID { return Err(RepositoryError::Schema("wrong application ID".into())); }
        validate_schema(&connection)?;
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
    let expected_tables = ["schema_version", "worlds", "dirty_chunks", "render_jobs", "session_checkpoints", "legacy_imports"];
    let mut statement = connection.prepare("SELECT type,name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name")?;
    let entries: Vec<(String, String)> = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?;
    if entries.iter().any(|(kind, name)| kind != "table" || !expected_tables.contains(&name.as_str())) || entries.iter().filter(|(kind, _)| kind == "table").count() != expected_tables.len() {
        return Err(RepositoryError::Schema("schema objects do not match checked-in migration".into()));
    }
    let expected_columns: [(&str, &[(&str, &str, i64, i64)]); 6] = [
        ("schema_version", &[("version", "INTEGER", 0, 1)]),
        ("worlds", &[("namespace", "TEXT", 1, 1), ("value", "TEXT", 1, 2), ("epoch", "INTEGER", 1, 0), ("config", "BLOB", 1, 0)]),
        ("dirty_chunks", &[("namespace", "TEXT", 1, 1), ("value", "TEXT", 1, 2), ("epoch", "INTEGER", 1, 3), ("x", "INTEGER", 1, 4), ("z", "INTEGER", 1, 5), ("revision", "INTEGER", 1, 0)]),
        ("render_jobs", &[("id", "BLOB", 0, 1), ("namespace", "TEXT", 1, 0), ("value", "TEXT", 1, 0), ("epoch", "INTEGER", 1, 0), ("kind", "INTEGER", 1, 0), ("state", "INTEGER", 1, 0), ("payload", "BLOB", 1, 0), ("completed_chunks", "INTEGER", 1, 0)]),
        ("session_checkpoints", &[("session_id", "BLOB", 0, 1), ("durable_sequence", "INTEGER", 1, 0)]),
        ("legacy_imports", &[("relative_path", "TEXT", 0, 1), ("content_sha256", "BLOB", 1, 0), ("imported_at_epoch_seconds", "INTEGER", 1, 0)]),
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
        let mut statement = connection.prepare("SELECT namespace,value,epoch,config FROM worlds WHERE epoch >= 0 ORDER BY namespace,value")?;
        statement.query_map([], |row| {
            let world = World { id: WorldId::new(row.get::<_, String>(0)?, row.get::<_, String>(1)?, checked_u64(row.get(2)?, "world epoch").map_err(|_| rusqlite::Error::InvalidQuery)?), config: row.get(3)? };
            world.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(world)
        })?.collect::<Result<Vec<_>, _>>()?
    };
    let dirty = {
        let mut statement = connection.prepare("SELECT dirty_chunks.namespace,dirty_chunks.value,dirty_chunks.epoch,dirty_chunks.x,dirty_chunks.z,dirty_chunks.revision FROM dirty_chunks INNER JOIN worlds USING(namespace,value) WHERE worlds.epoch >= 0 AND dirty_chunks.epoch=worlds.epoch ORDER BY dirty_chunks.namespace,dirty_chunks.value,dirty_chunks.epoch,dirty_chunks.x,dirty_chunks.z")?;
        statement.query_map([], |row| {
            let world = WorldId::new(row.get::<_, String>(0)?, row.get::<_, String>(1)?, checked_u64(row.get(2)?, "dirty epoch").map_err(|_| rusqlite::Error::InvalidQuery)?);
            world.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(DirtyChunk { world, coordinate: ChunkCoordinate { x: row.get(3)?, z: row.get(4)? }, revision: checked_u64(row.get(5)?, "dirty revision").map_err(|_| rusqlite::Error::InvalidQuery)? })
        })?.collect::<Result<Vec<_>, _>>()?
    };
    let jobs = {
        let mut statement = connection.prepare("SELECT render_jobs.id,render_jobs.namespace,render_jobs.value,render_jobs.epoch,render_jobs.kind,render_jobs.state,render_jobs.payload,render_jobs.completed_chunks FROM render_jobs INNER JOIN worlds USING(namespace,value) WHERE worlds.epoch >= 0 AND render_jobs.epoch=worlds.epoch AND render_jobs.state IN (1,2) ORDER BY render_jobs.id")?;
        statement.query_map([], |row| {
            let state = row.get::<_, i64>(5)?;
            let job = RenderJob { id: row.get(0)?, world: WorldId::new(row.get::<_, String>(1)?, row.get::<_, String>(2)?, checked_u64(row.get(3)?, "job epoch").map_err(|_| rusqlite::Error::InvalidQuery)?), kind: decode_kind(row.get(4)?).map_err(|_| rusqlite::Error::InvalidQuery)?, state: if state == JobState::Running as i64 { JobState::Resumable } else { decode_state(state).map_err(|_| rusqlite::Error::InvalidQuery)? }, payload: row.get(6)?, completed_chunks: checked_u64(row.get(7)?, "completed chunks").map_err(|_| rusqlite::Error::InvalidQuery)? };
            job.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(job)
        })?.collect::<Result<Vec<_>, _>>()?
    };
    let checkpoints = {
        let mut statement = connection.prepare("SELECT session_id,durable_sequence FROM session_checkpoints ORDER BY session_id")?;
        statement.query_map([], |row| {
            let session_id: Vec<u8> = row.get(0)?;
            if session_id.len() != MAX_SESSION_ID_BYTES { return Err(rusqlite::Error::InvalidQuery); }
            Ok(SessionCheckpoint { session_id, durable_sequence: checked_u64(row.get(1)?, "checkpoint").map_err(|_| rusqlite::Error::InvalidQuery)? })
        })?.collect::<Result<Vec<_>, _>>()?
    };
    Ok(Recovery { worlds, dirty, jobs, checkpoints })
}
fn decode_kind(value: i64) -> Result<JobKind, ()> { match value { 1 => Ok(JobKind::Full), 2 => Ok(JobKind::Resume), _ => Err(()) } }
fn decode_state(value: i64) -> Result<JobState, ()> { match value { 0 => Ok(JobState::Queued), 1 => Ok(JobState::Running), 2 => Ok(JobState::Resumable), 3 => Ok(JobState::Completed), 4 => Ok(JobState::Failed), _ => Err(()) } }

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

fn read_marker(transaction: &Transaction<'_>, key: &str) -> Result<Option<[u8; 32]>, RepositoryError> {
    let marker: Option<(i64, Vec<u8>)> = transaction.query_row("SELECT length(content_sha256),substr(content_sha256,1,33) FROM legacy_imports WHERE relative_path=?1", params![key], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
    let Some((length, prefix)) = marker else { return Ok(None); };
    if length != 32 || prefix.len() != 32 { return Err(RepositoryError::Schema("legacy marker hash must be exactly 32 bytes".into())); }
    let mut hash = [0_u8; 32];
    hash.copy_from_slice(&prefix);
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
            for coordinate in &parsed.dirty {
                transaction.execute("INSERT INTO dirty_chunks(namespace,value,epoch,x,z,revision) VALUES(?1,?2,?3,?4,?5,0) ON CONFLICT(namespace,value,epoch,x,z) DO NOTHING", params![world.namespace, world.value, epoch, coordinate.x, coordinate.z])?;
            }
        } else {
            let payload = parsed.resume_payload.clone().unwrap_or_default();
            let mut payload_hash = Sha256::new();
            payload_hash.update(&payload);
            let payload_hash = payload_hash.finalize().to_vec();
            let id = deterministic_job_id(world, JobKind::Resume);
            let existing: Option<(i64, Vec<u8>, i64)> = transaction.query_row("SELECT state,payload,completed_chunks FROM render_jobs WHERE id=?1", params![id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
            match existing {
                None => {
                    transaction.execute("INSERT INTO render_jobs(id,namespace,value,epoch,kind,state,payload,completed_chunks) VALUES(?1,?2,?3,?4,?5,?6,?7,0)", params![id, world.namespace, world.value, epoch, JobKind::Resume as i64, JobState::Resumable as i64, payload])?;
                    transaction.execute("INSERT INTO legacy_imports(relative_path,content_sha256,imported_at_epoch_seconds) VALUES(?1,?2,?3) ON CONFLICT(relative_path) DO UPDATE SET content_sha256=excluded.content_sha256,imported_at_epoch_seconds=excluded.imported_at_epoch_seconds", params![ownership_key, payload_hash, now])?;
                }
                Some((state, current_payload, progress)) => {
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
