use async_trait::async_trait;
use squaremap_render::{Registry, Snapshot};
use squaremap_server::scheduler::{
    BridgeError, InstallRequest, Scheduler, SchedulerConfig, SnapshotBridge, SnapshotReply,
    SnapshotRequest, TileInstaller,
};
use squaremap_state::{ChunkCoordinate, JobKind, JobState, Repository, World, WorldId};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::tempdir;
use tokio::sync::Notify;

fn world(value: &str, epoch: u64) -> World {
    World::new("minecraft", value, epoch, Vec::new())
}
fn coordinate(x: i32, z: i32) -> ChunkCoordinate {
    ChunkCoordinate { x, z }
}
fn empty_snapshot(request: &SnapshotRequest) -> Arc<Snapshot> {
    let registry = Registry::default();
    let generation = registry.generation();
    Arc::new(Snapshot {
        world: squaremap_render::snapshot::World {
            namespace: request.world.namespace.clone(),
            value: request.world.value.clone(),
            epoch: request.world.epoch,
        },
        coordinate: squaremap_render::snapshot::Coordinate {
            x: request.coordinate.x,
            z: request.coordinate.z,
        },
        min_y: 0,
        max_y: 15,
        ceiling: false,
        revision: request.revision,
        sections: Vec::new(),
        surface: squaremap_render::SurfaceHeightmap { heightmap: vec![0; 256] },
        registry_generation: generation,
    })
}

#[derive(Default)]
struct FakeBridge {
    requests: Mutex<Vec<SnapshotRequest>>,
    attempts: Mutex<HashMap<(String, i32, i32), usize>>,
    transient_once: Mutex<HashSet<(String, i32, i32)>>,
    missing: Mutex<HashSet<(String, i32, i32)>>,
    active: AtomicUsize,
    high_water: AtomicUsize,
    hold: Mutex<bool>,
    released: AtomicBool,
    entered: Notify,
    release: Notify,
}
impl FakeBridge {
    fn transient_once(&self, world: &str, x: i32, z: i32) {
        self.transient_once.lock().unwrap().insert((world.into(), x, z));
    }
    fn missing(&self, world: &str, x: i32, z: i32) {
        self.missing.lock().unwrap().insert((world.into(), x, z));
    }
    fn hold(&self) { *self.hold.lock().unwrap() = true; }
    async fn wait_entered(&self) {
        loop {
            let entered = !self.requests.lock().unwrap().is_empty();
            if entered { return; }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    fn release(&self) {
        self.released.store(true, Ordering::Release);
        self.release.notify_waiters();
    }
}
#[async_trait]
impl SnapshotBridge for FakeBridge {
    async fn request(&self, request: SnapshotRequest) -> Result<SnapshotReply, BridgeError> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.high_water.fetch_max(active, Ordering::SeqCst);
        self.requests.lock().unwrap().push(request.clone());
        self.entered.notify_waiters();
        if *self.hold.lock().unwrap() {
            while !self.released.load(Ordering::Acquire) {
                let notified = self.release.notified();
                if self.released.load(Ordering::Acquire) { break; }
                notified.await;
            }
        }
        tokio::task::yield_now().await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        let key = (request.world.value.clone(), request.coordinate.x, request.coordinate.z);
        let attempt = {
            let mut attempts = self.attempts.lock().unwrap();
            let entry = attempts.entry(key.clone()).or_default();
            *entry += 1;
            *entry
        };
        if attempt == 1 && self.transient_once.lock().unwrap().contains(&key) {
            return Err(BridgeError::Transient("disconnect".into()));
        }
        if self.missing.lock().unwrap().contains(&key) {
            return Ok(SnapshotReply::Missing);
        }
        Ok(SnapshotReply::Snapshot(empty_snapshot(&request)))
    }
}

#[derive(Default)]
struct FakeInstaller { installed: Mutex<Vec<(WorldId, ChunkCoordinate)>> }
#[async_trait]
impl TileInstaller for FakeInstaller {
    async fn install(&self, request: InstallRequest) -> Result<(), String> {
        self.installed.lock().unwrap().push((request.world, request.coordinate));
        Ok(())
    }
}

async fn repository() -> (tempfile::TempDir, Arc<Repository>) {
    let dir = tempdir().unwrap();
    let repository = Arc::new(Repository::open(dir.path().join("state.sqlite")).await.unwrap());
    (dir, repository)
}
fn scheduler(
    repository: Arc<Repository>,
    bridge: Arc<FakeBridge>,
    installer: Arc<FakeInstaller>,
    page_size: usize,
) -> Scheduler {
    Scheduler::new(
        repository,
        bridge,
        installer,
        SchedulerConfig {
            max_active_snapshots: 96,
            dirty_page_size: page_size,
            background_interval: Duration::from_millis(10),
            transient_retry_delay: Duration::from_millis(1),
        },
    ).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dirty_coalesces_retries_missing_and_requests_both_shading_neighbors() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    repository.mark_dirty(&overworld.id(), coordinate(4, 8), 1, &[1; 16], 1).await.unwrap();
    repository.mark_dirty(&overworld.id(), coordinate(4, 8), 2, &[1; 16], 2).await.unwrap();
    let bridge = Arc::new(FakeBridge::default());
    bridge.transient_once("overworld", 4, 8);
    let installer = Arc::new(FakeInstaller::default());
    let scheduler = scheduler(repository.clone(), bridge.clone(), installer.clone(), 16);

    let report = scheduler.run_dirty_page().await.unwrap();

    assert_eq!(report.completed, 1);
    assert!(repository.recover().await.unwrap().dirty.is_empty());
    assert_eq!(installer.installed.lock().unwrap().len(), 1);
    let requested: HashSet<_> = bridge.requests.lock().unwrap().iter().map(|r| (r.coordinate.x, r.coordinate.z)).collect();
    assert!(requested.contains(&(4, 7)));
    assert!(requested.contains(&(4, 8)));
    assert!(requested.contains(&(4, 9)));
    assert!(bridge.attempts.lock().unwrap().get(&("overworld".into(), 4, 8)).copied().unwrap() >= 2);

    repository.mark_dirty(&overworld.id(), coordinate(9, 9), 3, &[1; 16], 3).await.unwrap();
    bridge.missing("overworld", 9, 9);
    let report = scheduler.run_dirty_page().await.unwrap();
    assert_eq!(report.completed, 0);
    assert_eq!(report.stale, 0);
    assert!(!report.cancelled);
    assert_eq!(repository.recover().await.unwrap().dirty.len(), 1);
    assert_eq!(installer.installed.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_credit_ceiling_is_96() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    for x in 0..130 {
        repository.mark_dirty(&overworld.id(), coordinate(x, 0), x as u64 + 1, &[2; 16], x as u64 + 1).await.unwrap();
    }
    let bridge = Arc::new(FakeBridge::default());
    let scheduler = scheduler(repository, bridge.clone(), Arc::new(FakeInstaller::default()), 130);
    scheduler.run_dirty_page().await.unwrap();
    assert!(bridge.high_water.load(Ordering::SeqCst) <= 96);
    assert!(bridge.high_water.load(Ordering::SeqCst) > 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bounded_page_is_fair_between_worlds() {
    let (_dir, repository) = repository().await;
    let first = world("a", 1);
    let second = world("b", 1);
    repository.apply_world(first.clone()).await.unwrap();
    repository.apply_world(second.clone()).await.unwrap();
    for x in 0..8 {
        repository.mark_dirty(&first.id(), coordinate(x, 0), x as u64 + 1, &[3; 16], x as u64 + 1).await.unwrap();
    }
    repository.mark_dirty(&second.id(), coordinate(99, 0), 1, &[4; 16], 1).await.unwrap();
    let installer = Arc::new(FakeInstaller::default());
    scheduler(repository, Arc::new(FakeBridge::default()), installer.clone(), 2).run_dirty_page().await.unwrap();
    let worlds: HashSet<_> = installer.installed.lock().unwrap().iter().map(|(world, _)| world.value.clone()).collect();
    assert_eq!(worlds, HashSet::from(["a".into(), "b".into()]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_blocks_progress_and_cancel_fences_late_snapshot() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let bridge = Arc::new(FakeBridge::default());
    let installer = Arc::new(FakeInstaller::default());
    let first_scheduler = Arc::new(scheduler(repository.clone(), bridge.clone(), installer.clone(), 8));
    let job = first_scheduler.start_job(overworld.id(), JobKind::Full, vec![coordinate(1, 1)]).await.unwrap();
    first_scheduler.pause(&overworld.id);
    let running = { let scheduler = first_scheduler.clone(); let id = job.id.clone(); tokio::spawn(async move { scheduler.run_job(&id).await }) };
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(installer.installed.lock().unwrap().is_empty());
    first_scheduler.resume(&overworld.id);
    running.await.unwrap().unwrap();
    assert_eq!(installer.installed.lock().unwrap().len(), 1);

    let bridge = Arc::new(FakeBridge::default());
    bridge.hold();
    let installer = Arc::new(FakeInstaller::default());
    let scheduler = Arc::new(scheduler(repository.clone(), bridge.clone(), installer.clone(), 8));
    let job = scheduler.start_job(overworld.id(), JobKind::Radius, vec![coordinate(2, 2)]).await.unwrap();
    let running = { let scheduler = scheduler.clone(); let id = job.id.clone(); tokio::spawn(async move { scheduler.run_job(&id).await }) };
    bridge.wait_entered().await;
    scheduler.cancel_job(&job.id).await.unwrap();
    bridge.release();
    let report = running.await.unwrap().unwrap();
    assert!(report.cancelled);
    assert!(installer.installed.lock().unwrap().is_empty());
    assert_eq!(repository.load_render_job(&job.id).await.unwrap().unwrap().state, JobState::Cancelled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn persisted_cursor_resumes_after_reopen_and_epoch_invalidates_late_work() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("state.sqlite");
    let repository = Arc::new(Repository::open(&db).await.unwrap());
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let bridge = Arc::new(FakeBridge::default());
    let installer = Arc::new(FakeInstaller::default());
    let initial_scheduler = scheduler(repository.clone(), bridge, installer.clone(), 8);
    let job = initial_scheduler.start_job(overworld.id(), JobKind::Full, vec![coordinate(1, 1), coordinate(2, 2)]).await.unwrap();
    initial_scheduler.run_job_steps(&job.id, 1).await.unwrap();
    drop(initial_scheduler);
    drop(repository);

    let reopened = Arc::new(Repository::open(&db).await.unwrap());
    let reopened_scheduler = scheduler(reopened.clone(), Arc::new(FakeBridge::default()), installer.clone(), 8);
    reopened_scheduler.resume_jobs().await.unwrap();
    let completed = reopened.load_render_job(&job.id).await.unwrap().unwrap();
    assert_eq!(completed.state, JobState::Completed);
    assert_eq!(completed.completed_chunks, 2);

    reopened.mark_dirty(&overworld.id(), coordinate(7, 7), 9, &[5; 16], 1).await.unwrap();
    let bridge = Arc::new(FakeBridge::default());
    bridge.hold();
    let installer = Arc::new(FakeInstaller::default());
    let scheduler = Arc::new(scheduler(reopened.clone(), bridge.clone(), installer.clone(), 8));
    let running = { let scheduler = scheduler.clone(); tokio::spawn(async move { scheduler.run_dirty_page().await }) };
    bridge.wait_entered().await;
    reopened.apply_world(world("overworld", 2)).await.unwrap();
    bridge.release();
    running.await.unwrap().unwrap();
    assert!(installer.installed.lock().unwrap().is_empty());
}
