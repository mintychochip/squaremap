mod dirty;
mod jobs;
pub mod live;
mod renderer;

pub use renderer::{PrefixedTileStore, RenderTileInstaller, WorldRenderConfig};

use async_trait::async_trait;
use squaremap_render::Snapshot;
use squaremap_state::{ChunkCoordinate, JobKind, RenderJob, Repository, RepositoryError, WorldId};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};

pub const DEFAULT_MAX_ACTIVE_SNAPSHOTS: usize = 96;
pub const DEFAULT_DIRTY_PAGE_SIZE: usize = 256;

#[derive(Clone, Debug)]
pub struct SchedulerConfig {
    pub max_active_snapshots: usize,
    pub dirty_page_size: usize,
    pub background_interval: Duration,
    pub transient_retry_delay: Duration,
}
impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_active_snapshots: DEFAULT_MAX_ACTIVE_SNAPSHOTS,
            dirty_page_size: DEFAULT_DIRTY_PAGE_SIZE,
            background_interval: Duration::from_secs(15),
            transient_retry_delay: Duration::from_millis(25),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotRequest {
    pub world: WorldId,
    pub coordinate: ChunkCoordinate,
    pub revision: u64,
}
#[derive(Debug)]
pub enum SnapshotReply {
    Snapshot(Arc<Snapshot>),
    Missing,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BridgeError {
    Transient(String),
    Permanent(String),
}
impl fmt::Display for BridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transient(message) => write!(formatter, "transient snapshot bridge error: {message}"),
            Self::Permanent(message) => write!(formatter, "snapshot bridge error: {message}"),
        }
    }
}
impl std::error::Error for BridgeError {}

#[async_trait]
pub trait SnapshotBridge: Send + Sync {
    async fn request(&self, request: SnapshotRequest) -> Result<SnapshotReply, BridgeError>;
}

#[derive(Debug)]
pub struct SnapshotBundle {
    pub north: Option<Arc<Snapshot>>,
    pub center: Arc<Snapshot>,
    pub south: Option<Arc<Snapshot>>,
    pub biome_sources: Vec<Arc<Snapshot>>,
    pub grass_resolutions: Vec<(i32, i32, i32, u32, u32)>,
}
#[derive(Debug)]
pub struct InstallRequest {
    pub world: WorldId,
    pub coordinate: ChunkCoordinate,
    pub revision: u64,
    pub snapshots: SnapshotBundle,
}
#[async_trait]
pub trait TileInstaller: Send + Sync {
    async fn install(&self, request: InstallRequest) -> Result<(), String>;
}

#[derive(Debug)]
pub enum SchedulerError {
    InvalidConfig(&'static str),
    Repository(RepositoryError),
    Bridge(BridgeError),
    Install(String),
    MissingJob,
    InvalidJob(&'static str),
    Serialization(serde_json::Error),
    Join(tokio::task::JoinError),
}
impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) | Self::InvalidJob(message) => formatter.write_str(message),
            Self::Repository(error) => error.fmt(formatter),
            Self::Bridge(error) => error.fmt(formatter),
            Self::Install(message) => write!(formatter, "tile installation failed: {message}"),
            Self::MissingJob => formatter.write_str("render job does not exist"),
            Self::Serialization(error) => error.fmt(formatter),
            Self::Join(error) => error.fmt(formatter),
        }
    }
}
impl std::error::Error for SchedulerError {}
impl From<RepositoryError> for SchedulerError { fn from(value: RepositoryError) -> Self { Self::Repository(value) } }
impl From<serde_json::Error> for SchedulerError { fn from(value: serde_json::Error) -> Self { Self::Serialization(value) } }
impl From<tokio::task::JoinError> for SchedulerError { fn from(value: tokio::task::JoinError) -> Self { Self::Join(value) } }

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RunReport {
    pub selected: usize,
    pub completed: usize,
    pub stale: usize,
    pub cancelled: bool,
}

#[derive(Default)]
struct WorldPause {
    paused: std::sync::atomic::AtomicBool,
    notify: Notify,
}
pub struct Scheduler {
    repository: Arc<Repository>,
    bridge: Arc<dyn SnapshotBridge>,
    installer: Arc<dyn TileInstaller>,
    snapshot_credits: Arc<Semaphore>,
    config: SchedulerConfig,
    pause_states: Mutex<HashMap<WorldId, Arc<WorldPause>>>,
    cancelled_jobs: Mutex<HashSet<Vec<u8>>>,
    next_job_nonce: AtomicU64,
}
impl Scheduler {
    pub fn new(
        repository: Arc<Repository>,
        bridge: Arc<dyn SnapshotBridge>,
        installer: Arc<dyn TileInstaller>,
        config: SchedulerConfig,
    ) -> Result<Self, SchedulerError> {
        if config.max_active_snapshots == 0 {
            return Err(SchedulerError::InvalidConfig("snapshot credit count must be positive"));
        }
        if config.dirty_page_size == 0 {
            return Err(SchedulerError::InvalidConfig("dirty page size must be positive"));
        }
        if config.background_interval.is_zero() {
            return Err(SchedulerError::InvalidConfig("background interval must be positive"));
        }
        Ok(Self {
            repository,
            bridge,
            installer,
            snapshot_credits: Arc::new(Semaphore::new(config.max_active_snapshots)),
            config,
            pause_states: Mutex::new(HashMap::new()),
            cancelled_jobs: Mutex::new(HashSet::new()),
            next_job_nonce: AtomicU64::new(1),
        })
    }
    pub fn config(&self) -> &SchedulerConfig { &self.config }

    fn pause_state(&self, world: &WorldId) -> Arc<WorldPause> {
        let mut states = self.pause_states.lock().expect("pause state lock poisoned");
        states.entry(world.clone()).or_insert_with(|| Arc::new(WorldPause::default())).clone()
    }
    pub fn pause(&self, world: &WorldId) {
        self.pause_state(world).paused.store(true, Ordering::Release);
    }
    pub fn resume(&self, world: &WorldId) {
        let state = self.pause_state(world);
        state.paused.store(false, Ordering::Release);
        state.notify.notify_waiters();
    }
    pub fn is_paused(&self, world: &WorldId) -> bool {
        self.pause_state(world).paused.load(Ordering::Acquire)
    }
    pub(crate) async fn wait_unpaused(&self, world: &WorldId) {
        let state = self.pause_state(world);
        while state.paused.load(Ordering::Acquire) {
            let notified = state.notify.notified();
            if !state.paused.load(Ordering::Acquire) { break; }
            notified.await;
        }
    }
    fn is_cancelled(&self, id: &[u8]) -> bool {
        self.cancelled_jobs.lock().map_or(true, |jobs| jobs.contains(id))
    }

    async fn request_snapshot(&self, request: SnapshotRequest) -> Result<SnapshotReply, SchedulerError> {
        loop {
            let permit = self.snapshot_credits.clone().acquire_owned().await
                .map_err(|_| SchedulerError::InvalidConfig("snapshot credit semaphore closed"))?;
            let result = self.bridge.request(request.clone()).await;
            drop(permit);
            match result {
                Ok(reply) => return Ok(reply),
                Err(BridgeError::Transient(_)) => tokio::time::sleep(self.config.transient_retry_delay).await,
                Err(error) => return Err(SchedulerError::Bridge(error)),
            }
        }
    }

    async fn snapshot_bundle(
        &self,
        world: &WorldId,
        coordinate: ChunkCoordinate,
        revision: u64,
        job_id: Option<&[u8]>,
    ) -> Result<Option<SnapshotBundle>, SchedulerError> {
        if job_id.is_some_and(|id| self.is_cancelled(id)) { return Ok(None); }
        let center = self.request_snapshot(SnapshotRequest { world: world.clone(), coordinate, revision }).await?;
        let center = match center {
            SnapshotReply::Snapshot(snapshot) => snapshot,
            SnapshotReply::Missing => return Ok(None),
        };
        let (north, south) = tokio::join!(
            self.request_snapshot(SnapshotRequest { world: world.clone(), coordinate: ChunkCoordinate { x: coordinate.x, z: coordinate.z - 1 }, revision }),
            self.request_snapshot(SnapshotRequest { world: world.clone(), coordinate: ChunkCoordinate { x: coordinate.x, z: coordinate.z + 1 }, revision }),
        );
        let optional = |reply: Result<SnapshotReply, SchedulerError>| -> Result<Option<Arc<Snapshot>>, SchedulerError> {
            match reply? {
                SnapshotReply::Snapshot(snapshot) => Ok(Some(snapshot)),
                SnapshotReply::Missing => Ok(None),
            }
        };
        Ok(Some(SnapshotBundle { north: optional(north)?, center, south: optional(south)?, biome_sources: Vec::new(), grass_resolutions: Vec::new() }))
    }

    async fn render_one(
        &self,
        world: &WorldId,
        coordinate: ChunkCoordinate,
        revision: u64,
        job_id: Option<&[u8]>,
    ) -> Result<RenderDisposition, SchedulerError> {
        self.wait_unpaused(world).await;
        if job_id.is_some_and(|id| self.is_cancelled(id)) {
            return Ok(RenderDisposition::Cancelled);
        }
        let Some(snapshots) = self.snapshot_bundle(world, coordinate, revision, job_id).await? else {
            if job_id.is_some_and(|id| self.is_cancelled(id)) {
                return Ok(RenderDisposition::Cancelled);
            }
            return Ok(if self.repository.world_is_current(world).await? {
                RenderDisposition::Missing
            } else {
                RenderDisposition::Stale
            });
        };
        if job_id.is_some_and(|id| self.is_cancelled(id)) {
            return Ok(RenderDisposition::Cancelled);
        }
        if !self.repository.world_is_current(world).await? {
            return Ok(RenderDisposition::Stale);
        }
        self.installer.install(InstallRequest {
            world: world.clone(),
            coordinate,
            revision,
            snapshots,
        }).await.map_err(SchedulerError::Install)?;
        Ok(RenderDisposition::Installed)
    }

    pub async fn start_job(
        &self,
        world: WorldId,
        kind: JobKind,
        coordinates: Vec<ChunkCoordinate>,
    ) -> Result<RenderJob, SchedulerError> {
        self.start_job_at_revision(world, kind, coordinates, 0).await
    }
    pub async fn start_job_at_revision(
        &self,
        world: WorldId,
        kind: JobKind,
        coordinates: Vec<ChunkCoordinate>,
        revision: u64,
    ) -> Result<RenderJob, SchedulerError> {
        jobs::start(self, world, kind, coordinates, revision).await
    }
    pub async fn run_job(&self, id: &[u8]) -> Result<RunReport, SchedulerError> {
        self.run_job_steps(id, usize::MAX).await
    }
    pub async fn run_job_steps(&self, id: &[u8], steps: usize) -> Result<RunReport, SchedulerError> {
        jobs::run(self, id, steps).await
    }
    pub async fn cancel_job(&self, id: &[u8]) -> Result<(), SchedulerError> {
        jobs::cancel(self, id).await
    }
    pub async fn resume_jobs(&self) -> Result<RunReport, SchedulerError> {
        jobs::resume_all(self).await
    }
    pub async fn reset_world(&self, world: &WorldId) -> Result<(), SchedulerError> {
        self.repository.reset_world(world).await?;
        Ok(())
    }
    pub async fn run_dirty_page(&self) -> Result<RunReport, SchedulerError> {
        dirty::run_page(self).await
    }
    pub async fn run_background_until<F>(&self, mut shutdown: F) -> Result<(), SchedulerError>
    where F: FnMut() -> bool {
        while !shutdown() {
            self.run_dirty_page().await?;
            tokio::time::sleep(self.config.background_interval).await;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RenderDisposition { Installed, Missing, Stale, Cancelled }
