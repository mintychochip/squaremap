mod dirty;
mod jobs;
pub mod live;
mod renderer;

pub use renderer::{PrefixedTileStore, RenderTileInstaller, WorldRenderConfig};

use async_trait::async_trait;
use squaremap_protocol::wire::{ChunkMissingReason, ConfigReplace, VisibilityLimitKind};
use squaremap_render::Snapshot;
use squaremap_render::visibility::VisibilityLimit;
use squaremap_state::{ChunkCoordinate, JobKind, RenderJob, Repository, RepositoryError, WorldId};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};

pub const DEFAULT_MAX_ACTIVE_SNAPSHOTS: usize = 96;
pub const DEFAULT_DIRTY_PAGE_SIZE: usize = 256;
/// Live ticks pick a small newest page so fly-in chunks are not stuck behind a
/// thousand-chunk generation backlog.
pub const LIVE_DIRTY_PAGE_SIZE: usize = 32;
pub const LIVE_DIRTY_CONCURRENCY: usize = 2;
/// Live dirty ticks have no job cancel token. Bound Transient retries so one
/// newest chunk cannot stall the rest of the page.
const LIVE_SNAPSHOT_TRANSIENT_ATTEMPTS: u32 = 3;
const LIVE_SNAPSHOT_REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

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
    pub loaded_only: bool,
}
#[derive(Debug)]
pub enum SnapshotReply {
    Snapshot(Arc<Snapshot>),
    Missing(ChunkMissingReason),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BridgeError {
    Transient(String),
    Permanent(String),
}
impl fmt::Display for BridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transient(message) => {
                write!(formatter, "transient snapshot bridge error: {message}")
            }
            Self::Permanent(message) => write!(formatter, "snapshot bridge error: {message}"),
        }
    }
}
impl std::error::Error for BridgeError {}

#[async_trait]
pub trait SnapshotBridge: Send + Sync {
    async fn request(&self, request: SnapshotRequest) -> Result<SnapshotReply, BridgeError>;
    /// Enumerates the authoritative full-render catalog supplied by the bridge.
    ///
    /// Implementations must not derive this from dirty state or on-disk region files.
    async fn enumerate_world(&self, world: &WorldId) -> Result<Vec<ChunkCoordinate>, BridgeError>;
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
enum SnapshotBundleResult {
    Ready(SnapshotBundle),
    Missing,
    Stale,
    Cancelled,
}
#[derive(Debug)]
pub struct InstallRequest {
    pub world: WorldId,
    pub coordinate: ChunkCoordinate,
    pub revision: u64,
    pub snapshots: SnapshotBundle,
}
#[async_trait]
pub trait StagedInstall: Send {
    async fn publish(self: Box<Self>) -> Result<(), String>;
}

#[async_trait]
pub trait TileInstaller: Send + Sync {
    async fn stage(&self, request: InstallRequest) -> Result<Box<dyn StagedInstall>, String>;
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
            Self::InvalidConfig(message) | Self::InvalidJob(message) => {
                formatter.write_str(message)
            }
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
impl From<RepositoryError> for SchedulerError {
    fn from(value: RepositoryError) -> Self {
        Self::Repository(value)
    }
}
impl From<serde_json::Error> for SchedulerError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value)
    }
}
impl From<tokio::task::JoinError> for SchedulerError {
    fn from(value: tokio::task::JoinError) -> Self {
        Self::Join(value)
    }
}

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
    install_fences: Mutex<HashMap<WorldId, Arc<tokio::sync::Mutex<u64>>>>,
    visibility: Mutex<HashMap<WorldId, VisibilityLimit>>,
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
            return Err(SchedulerError::InvalidConfig(
                "snapshot credit count must be positive",
            ));
        }
        if config.dirty_page_size == 0 {
            return Err(SchedulerError::InvalidConfig(
                "dirty page size must be positive",
            ));
        }
        if config.background_interval.is_zero() {
            return Err(SchedulerError::InvalidConfig(
                "background interval must be positive",
            ));
        }
        Ok(Self {
            repository,
            bridge,
            installer,
            snapshot_credits: Arc::new(Semaphore::new(config.max_active_snapshots)),
            config,
            pause_states: Mutex::new(HashMap::new()),
            cancelled_jobs: Mutex::new(HashSet::new()),
            install_fences: Mutex::new(HashMap::new()),
            visibility: Mutex::new(HashMap::new()),
            next_job_nonce: AtomicU64::new(1),
        })
    }
    fn install_fence(&self, world: &WorldId) -> Arc<tokio::sync::Mutex<u64>> {
        let mut fences = self
            .install_fences
            .lock()
            .expect("install fence lock poisoned");
        fences
            .entry(world.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(0)))
            .clone()
    }
    pub(crate) async fn invalidate_install_fence(&self, world: &WorldId) {
        let fence = self.install_fence(world);
        let mut generation = fence.lock().await;
        *generation = generation.wrapping_add(1);
    }
    pub fn config(&self) -> &SchedulerConfig {
        &self.config
    }

    fn pause_state(&self, world: &WorldId) -> Arc<WorldPause> {
        let mut states = self.pause_states.lock().expect("pause state lock poisoned");
        states
            .entry(world.clone())
            .or_insert_with(|| Arc::new(WorldPause::default()))
            .clone()
    }
    pub fn pause(&self, world: &WorldId) {
        self.pause_state(world)
            .paused
            .store(true, Ordering::Release);
    }
    pub fn resume(&self, world: &WorldId) {
        let state = self.pause_state(world);
        state.paused.store(false, Ordering::Release);
        state.notify.notify_waiters();
    }
    pub fn is_paused(&self, world: &WorldId) -> bool {
        self.pause_state(world).paused.load(Ordering::Acquire)
    }
    pub fn set_visibility(&self, world: WorldId, limit: VisibilityLimit) {
        self.visibility
            .lock()
            .expect("visibility lock poisoned")
            .insert(world, limit);
    }
    pub fn visibility_for(&self, world: &WorldId) -> Option<VisibilityLimit> {
        self.visibility
            .lock()
            .expect("visibility lock poisoned")
            .get(world)
            .cloned()
    }
    pub fn filter_visible(
        &self,
        world: &WorldId,
        coordinates: Vec<ChunkCoordinate>,
    ) -> Vec<ChunkCoordinate> {
        match self.visibility_for(world) {
            None => coordinates,
            Some(limit) => coordinates
                .into_iter()
                .filter(|coordinate| limit.contains_chunk(coordinate.x, coordinate.z))
                .collect(),
        }
    }
    pub fn apply_visibility_from_config(
        &self,
        config: &ConfigReplace,
    ) -> Result<(), SchedulerError> {
        for configured in &config.worlds {
            let identity = configured
                .identity
                .as_ref()
                .ok_or(SchedulerError::InvalidConfig("world identity is missing"))?;
            let settings = configured
                .settings
                .as_ref()
                .ok_or(SchedulerError::InvalidConfig("world settings are missing"))?;
            let geometric: Vec<_> = settings
                .visibility_limits
                .iter()
                .filter(|limit| {
                    !matches!(
                        VisibilityLimitKind::try_from(limit.kind),
                        Ok(VisibilityLimitKind::WorldBorder)
                    )
                })
                .cloned()
                .collect();
            let limit = VisibilityLimit::from_wire(&geometric, None)
                .map_err(|_| SchedulerError::InvalidConfig("visibility limit is invalid"))?;
            self.set_visibility(
                WorldId::new(
                    identity.namespace.clone(),
                    identity.value.clone(),
                    identity.epoch,
                ),
                limit,
            );
        }
        Ok(())
    }
    fn allows_chunk(&self, world: &WorldId, coordinate: ChunkCoordinate) -> bool {
        self.visibility_for(world)
            .is_none_or(|limit| limit.contains_chunk(coordinate.x, coordinate.z))
    }
    pub(crate) async fn wait_unpaused(&self, world: &WorldId, job_id: Option<&[u8]>) -> bool {
        let state = self.pause_state(world);
        while state.paused.load(Ordering::Acquire) {
            if job_id.is_some_and(|id| self.is_cancelled(id)) {
                return false;
            }
            let notified = state.notify.notified();
            if !state.paused.load(Ordering::Acquire) {
                break;
            }
            if job_id.is_some() {
                tokio::select! {
                    _ = notified => {}
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {}
                }
            } else {
                notified.await;
            }
        }
        !job_id.is_some_and(|id| self.is_cancelled(id))
    }
    fn is_cancelled(&self, id: &[u8]) -> bool {
        self.cancelled_jobs
            .lock()
            .map_or(true, |jobs| jobs.contains(id))
    }

    async fn request_snapshot(
        &self,
        request: SnapshotRequest,
        job_id: Option<&[u8]>,
    ) -> Result<SnapshotReply, SchedulerError> {
        let mut transient_attempts = 0u32;
        loop {
            if job_id.is_some_and(|id| self.is_cancelled(id)) {
                return Err(SchedulerError::InvalidJob("render job cancelled"));
            }
            let permit = if let Some(id) = job_id {
                tokio::select! {
                    permit = self.snapshot_credits.clone().acquire_owned() => permit
                        .map_err(|_| SchedulerError::InvalidConfig("snapshot credit semaphore closed"))?,
                    _ = self.wait_for_cancellation(id) => {
                        return Err(SchedulerError::InvalidJob("render job cancelled"));
                    }
                }
            } else {
                self.snapshot_credits
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| {
                        SchedulerError::InvalidConfig("snapshot credit semaphore closed")
                    })?
            };
            let result = if let Some(id) = job_id {
                tokio::select! {
                    result = self.bridge.request(request.clone()) => result,
                    _ = self.wait_for_cancellation(id) => {
                        return Err(SchedulerError::InvalidJob("render job cancelled"));
                    }
                }
            } else {
                match tokio::time::timeout(
                    LIVE_SNAPSHOT_REQUEST_TIMEOUT,
                    self.bridge.request(request.clone()),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(BridgeError::Transient("snapshot request timed out".into())),
                }
            };
            drop(permit);
            match result {
                Ok(reply) => return Ok(reply),
                Err(BridgeError::Transient(_)) => {
                    transient_attempts += 1;
                    if job_id.is_none() && transient_attempts >= LIVE_SNAPSHOT_TRANSIENT_ATTEMPTS {
                        return Ok(SnapshotReply::Missing(ChunkMissingReason::Unloaded));
                    }
                    if let Some(id) = job_id {
                        tokio::select! {
                            _ = tokio::time::sleep(self.config.transient_retry_delay) => {}
                            _ = self.wait_for_cancellation(id) => {
                                return Err(SchedulerError::InvalidJob("render job cancelled"));
                            }
                        }
                    } else {
                        tokio::time::sleep(self.config.transient_retry_delay).await;
                    }
                }
                Err(error) => return Err(SchedulerError::Bridge(error)),
            }
        }
    }

    async fn wait_for_cancellation(&self, id: &[u8]) {
        while !self.is_cancelled(id) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn snapshot_bundle(
        &self,
        world: &WorldId,
        coordinate: ChunkCoordinate,
        revision: u64,
        job_id: Option<&[u8]>,
        loaded_only: bool,
        fetch_neighbors: bool,
    ) -> Result<SnapshotBundleResult, SchedulerError> {
        if job_id.is_some_and(|id| self.is_cancelled(id)) {
            return Ok(SnapshotBundleResult::Cancelled);
        }
        let center_reply = match self
            .request_snapshot(
                SnapshotRequest {
                    world: world.clone(),
                    coordinate,
                    revision,
                    loaded_only,
                },
                job_id,
            )
            .await
        {
            Ok(reply) => reply,
            Err(SchedulerError::InvalidJob("render job cancelled")) => {
                return Ok(SnapshotBundleResult::Cancelled);
            }
            Err(error) => return Err(error),
        };
        if job_id.is_some_and(|id| self.is_cancelled(id)) {
            return Ok(SnapshotBundleResult::Cancelled);
        }
        let center = match center_reply {
            SnapshotReply::Snapshot(snapshot) => snapshot,
            SnapshotReply::Missing(ChunkMissingReason::Unloaded) => {
                if !self.repository.world_is_current(world).await? {
                    return Ok(SnapshotBundleResult::Stale);
                }
                return Ok(SnapshotBundleResult::Missing);
            }
            SnapshotReply::Missing(_) => return Ok(SnapshotBundleResult::Stale),
        };
        if !fetch_neighbors {
            return Ok(SnapshotBundleResult::Ready(SnapshotBundle {
                north: None,
                center,
                south: None,
                biome_sources: Vec::new(),
                grass_resolutions: Vec::new(),
            }));
        }
        let neighbor_offsets = [
            (0, -1),
            (0, 1),
            (-1, 0),
            (1, 0),
            (-1, -1),
            (1, -1),
            (-1, 1),
            (1, 1),
        ];
        let mut neighbor_replies = Vec::with_capacity(neighbor_offsets.len());
        for (dx, dz) in neighbor_offsets {
            neighbor_replies.push(
                self.request_snapshot(
                    SnapshotRequest {
                        world: world.clone(),
                        coordinate: ChunkCoordinate {
                            x: coordinate.x + dx,
                            z: coordinate.z + dz,
                        },
                        revision,
                        loaded_only,
                    },
                    job_id,
                )
                .await,
            );
        }
        if neighbor_replies.iter().any(|reply| {
            reply.as_ref().is_err_and(|error| {
                matches!(error, SchedulerError::InvalidJob("render job cancelled"))
            })
        }) {
            return Ok(SnapshotBundleResult::Cancelled);
        }
        let optional = |reply: Result<SnapshotReply, SchedulerError>| -> Result<Option<Arc<Snapshot>>, SchedulerError> {
            match reply? {
                SnapshotReply::Snapshot(snapshot) => Ok(Some(snapshot)),
                SnapshotReply::Missing(_) => Ok(None),
            }
        };
        let north = optional(neighbor_replies.remove(0))?;
        let south = optional(neighbor_replies.remove(0))?;
        let mut biome_sources = Vec::new();
        for reply in neighbor_replies {
            if let Some(snapshot) = optional(reply)? {
                biome_sources.push(snapshot);
            }
        }
        Ok(SnapshotBundleResult::Ready(SnapshotBundle {
            north,
            center,
            south,
            biome_sources,
            grass_resolutions: Vec::new(),
        }))
    }
    async fn render_one(
        &self,
        world: &WorldId,
        coordinate: ChunkCoordinate,
        revision: u64,
        job_id: Option<&[u8]>,
        loaded_only: bool,
        fetch_neighbors: bool,
    ) -> Result<RenderDisposition, SchedulerError> {
        if !self.wait_unpaused(world, job_id).await
            || job_id.is_some_and(|id| self.is_cancelled(id))
        {
            return Ok(RenderDisposition::Cancelled);
        }
        if !self.allows_chunk(world, coordinate) {
            return Ok(RenderDisposition::Installed);
        }
        let snapshots = match self
            .snapshot_bundle(world, coordinate, revision, job_id, loaded_only, fetch_neighbors)
            .await?
        {
            SnapshotBundleResult::Ready(snapshots) => snapshots,
            SnapshotBundleResult::Missing => return Ok(RenderDisposition::Missing),
            SnapshotBundleResult::Stale => return Ok(RenderDisposition::Stale),
            SnapshotBundleResult::Cancelled => return Ok(RenderDisposition::Cancelled),
        };
        let fence = self.install_fence(world);
        let expected_generation = *fence.lock().await;
        if job_id.is_some_and(|id| self.is_cancelled(id))
            || !self.repository.world_is_current(world).await?
        {
            return Ok(if job_id.is_some_and(|id| self.is_cancelled(id)) {
                RenderDisposition::Cancelled
            } else {
                RenderDisposition::Stale
            });
        }
        let staged = self
            .installer
            .stage(InstallRequest {
                world: world.clone(),
                coordinate,
                revision,
                snapshots,
            })
            .await
            .map_err(SchedulerError::Install)?;
        let generation = fence.lock().await;
        if *generation != expected_generation || job_id.is_some_and(|id| self.is_cancelled(id)) {
            return Ok(RenderDisposition::Cancelled);
        }
        if !self.repository.world_is_current(world).await? {
            return Ok(RenderDisposition::Stale);
        }
        staged.publish().await.map_err(SchedulerError::Install)?;
        drop(generation);
        Ok(RenderDisposition::Installed)
    }

    pub async fn start_job(
        &self,
        world: WorldId,
        kind: JobKind,
        coordinates: Vec<ChunkCoordinate>,
    ) -> Result<RenderJob, SchedulerError> {
        self.start_job_at_revision(world, kind, coordinates, 0)
            .await
    }
    pub async fn start_job_at_revision(
        &self,
        world: WorldId,
        kind: JobKind,
        coordinates: Vec<ChunkCoordinate>,
        revision: u64,
    ) -> Result<RenderJob, SchedulerError> {
        let coordinates = self.filter_visible(&world, coordinates);
        jobs::start(self, world, kind, coordinates, revision).await
    }
    pub async fn run_job(&self, id: &[u8]) -> Result<RunReport, SchedulerError> {
        self.run_job_steps(id, usize::MAX).await
    }
    pub async fn run_job_steps(
        &self,
        id: &[u8],
        steps: usize,
    ) -> Result<RunReport, SchedulerError> {
        jobs::run(self, id, steps).await
    }
    pub async fn cancel_job(&self, id: &[u8]) -> Result<(), SchedulerError> {
        jobs::cancel(self, id).await
    }
    pub async fn resume_jobs(&self) -> Result<RunReport, SchedulerError> {
        jobs::resume_all(self).await
    }
    pub async fn reset_world(&self, world: &WorldId) -> Result<(), SchedulerError> {
        self.invalidate_install_fence(world).await;
        self.repository.reset_world(world).await?;
        Ok(())
    }
    pub async fn run_dirty_page(&self) -> Result<RunReport, SchedulerError> {
        dirty::run_page(self).await
    }
    pub async fn run_background_until<F>(&self, mut shutdown: F) -> Result<(), SchedulerError>
    where
        F: FnMut() -> bool,
    {
        while !shutdown() {
            self.run_dirty_page().await?;
            tokio::time::sleep(self.config.background_interval).await;
        }
        Ok(())
    }
    /// Runs one owner-scoped dirty page for `bridge_id` with lease assignment,
    /// rendering, and owner-scoped completion/deferral.
    pub async fn run_owner_dirty_page(
        &self,
        bridge_id: &[u8],
    ) -> Result<RunReport, SchedulerError> {
        dirty::run_owner_page(self, bridge_id, None).await
    }
    /// Production dirty tick: only rows leased to `bridge_id`.
    pub async fn run_live_dirty_page(&self, bridge_id: &[u8]) -> Result<RunReport, SchedulerError> {
        self.run_live_dirty_page_near(bridge_id, None).await
    }
    pub async fn run_live_dirty_page_near(
        &self,
        bridge_id: &[u8],
        focus: Option<(i32, i32)>,
    ) -> Result<RunReport, SchedulerError> {
        dirty::run_owner_page(self, bridge_id, focus).await
    }
    pub async fn discover_full_render_coordinates(
        &self,
        world: &WorldId,
    ) -> Result<Vec<ChunkCoordinate>, SchedulerError> {
        let coordinates = self
            .bridge
            .enumerate_world(world)
            .await
            .map_err(SchedulerError::Bridge)?;
        Ok(self.filter_visible(world, coordinates))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RenderDisposition {
    Installed,
    Missing,
    Stale,
    Cancelled,
}
