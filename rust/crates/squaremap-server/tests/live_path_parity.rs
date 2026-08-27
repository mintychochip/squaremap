use async_trait::async_trait;
use serde::Deserialize;
use squaremap_protocol::wire::{
    ChunkMissingReason, Point, VisibilityLimit, VisibilityLimitKind,
};
use squaremap_render::{Registry, Snapshot};
use squaremap_server::radius;
use squaremap_server::scheduler::{
    BridgeError, InstallRequest, Scheduler, SchedulerConfig, SnapshotBridge, SnapshotReply,
    SnapshotRequest, StagedInstall, TileInstaller,
};
use squaremap_state::{ChunkCoordinate, JobKind, Repository, World, WorldId};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::tempdir;

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
        surface: squaremap_render::SurfaceHeightmap {
            heightmap: vec![0; 256],
        },
        registry_generation: generation,
    })
}

#[derive(Default)]
struct FakeBridge {
    requests: Mutex<Vec<SnapshotRequest>>,
}

#[async_trait]
impl SnapshotBridge for FakeBridge {
    async fn request(&self, request: SnapshotRequest) -> Result<SnapshotReply, BridgeError> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(SnapshotReply::Snapshot(empty_snapshot(&request)))
    }
    async fn enumerate_world(&self, _world: &WorldId) -> Result<Vec<ChunkCoordinate>, BridgeError> {
        Ok(vec![coordinate(0, 0), coordinate(8, 8)])
    }
}

#[derive(Default)]
struct CapturingInstaller {
    bundles: Mutex<Vec<(ChunkCoordinate, Vec<(i32, i32)>)>>,
}

struct NoopStaged;

#[async_trait]
impl StagedInstall for NoopStaged {
    async fn publish(self: Box<Self>) -> Result<(), String> {
        Ok(())
    }
}

#[async_trait]
impl TileInstaller for CapturingInstaller {
    async fn stage(&self, request: InstallRequest) -> Result<Box<dyn StagedInstall>, String> {
        let biome = request
            .snapshots
            .biome_sources
            .iter()
            .map(|snapshot| (snapshot.coordinate.x, snapshot.coordinate.z))
            .collect();
        self.bundles
            .lock()
            .unwrap()
            .push((request.coordinate, biome));
        Ok(Box::new(NoopStaged))
    }
}

async fn repository() -> (tempfile::TempDir, Arc<Repository>) {
    let dir = tempdir().unwrap();
    let repository = Arc::new(
        Repository::open(dir.path().join("state.sqlite"))
            .await
            .unwrap(),
    );
    (dir, repository)
}

fn scheduler(
    repository: Arc<Repository>,
    bridge: Arc<dyn SnapshotBridge>,
    installer: Arc<CapturingInstaller>,
) -> Scheduler {
    Scheduler::new(
        repository,
        bridge,
        installer,
        SchedulerConfig {
            max_active_snapshots: 96,
            dirty_page_size: 16,
            background_interval: Duration::from_millis(10),
            transient_retry_delay: Duration::from_millis(1),
        },
    )
    .unwrap()
}

/// Paper `/radiusrender` takes block center/radius. Java converts with `n >> 4`.
#[test]
fn radius_block_center_matches_java_block_to_chunk() {
    let chunks = radius::chunks_from_blocks(800, -17, 256, None).expect("valid paper radius");
    let set: HashSet<_> = chunks.iter().copied().collect();
    // block 800 >> 4 = 50; block -17 >> 4 = -2; block 256 >> 4 = 16
    assert!(set.contains(&coordinate(50, -2)));
    assert!(set.contains(&coordinate(50 - 16, -2 - 16)));
    assert!(set.contains(&coordinate(50 + 16, -2 + 16)));
    assert!(!set.contains(&coordinate(50 - 17, -2)));
    assert!(!set.contains(&coordinate(50, -2 + 17)));
    assert_eq!(chunks.len(), 33 * 33);
}

#[test]
fn one_block_radius_schedules_only_the_center_chunk() {
    let chunks =
        radius::chunks_from_blocks(16, 16, 1, None).expect("1-block radius is valid in Paper");
    assert_eq!(chunks, vec![coordinate(1, 1)]);
}

#[tokio::test]
async fn visibility_limit_excludes_chunks_from_started_jobs() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let scheduler = scheduler(
        repository,
        Arc::new(FakeBridge::default()),
        Arc::new(CapturingInstaller::default()),
    );
    let limit = squaremap_render::visibility::VisibilityLimit::from_wire(
        &[VisibilityLimit {
            kind: VisibilityLimitKind::Rectangle as i32,
            points: vec![Point { x: 0, z: 0 }, Point { x: 15, z: 15 }],
            ..Default::default()
        }],
        None,
    )
    .unwrap();
    scheduler.set_visibility(overworld.id(), limit);
    let job = scheduler
        .start_job(
            overworld.id(),
            JobKind::Radius,
            vec![coordinate(0, 0), coordinate(8, 8)],
        )
        .await
        .unwrap();
    #[derive(Deserialize)]
    struct Cursor {
        coordinates: Vec<ChunkCoordinate>,
    }
    let cursor: Cursor = serde_json::from_slice(&job.payload).unwrap();
    assert_eq!(cursor.coordinates, vec![coordinate(0, 0)]);
}

#[tokio::test]
async fn full_render_enumeration_drops_visibility_excluded_chunks() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let scheduler = scheduler(
        repository,
        Arc::new(FakeBridge::default()),
        Arc::new(CapturingInstaller::default()),
    );
    let limit = squaremap_render::visibility::VisibilityLimit::from_wire(
        &[VisibilityLimit {
            kind: VisibilityLimitKind::Rectangle as i32,
            points: vec![Point { x: 0, z: 0 }, Point { x: 15, z: 15 }],
            ..Default::default()
        }],
        None,
    )
    .unwrap();
    scheduler.set_visibility(overworld.id(), limit);
    let discovered = scheduler
        .discover_full_render_coordinates(&overworld.id())
        .await
        .unwrap();
    assert_eq!(discovered, vec![coordinate(0, 0)]);
}

/// Live fly-in shares one Java snapshot worker with a 2s timeout. Requesting
/// eight biome neighbors per chunk makes the player's own cell time out.
/// Jobs still fetch neighbors; live ticks only snapshot the dirty chunk.
#[tokio::test]
async fn live_snapshot_bundle_only_requests_the_dirty_chunk() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let bridge = Arc::new(FakeBridge::default());
    let installer = Arc::new(CapturingInstaller::default());
    let scheduler = scheduler(repository.clone(), bridge.clone(), installer.clone());
    repository
        .mark_dirty(&overworld.id(), coordinate(4, 8), 1, &[1; 16], &[1; 16], 1)
        .await
        .unwrap();
    scheduler.run_live_dirty_page(&[1; 16]).await.unwrap();
    let requested: HashSet<_> = bridge
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| (request.coordinate.x, request.coordinate.z))
        .collect();
    assert_eq!(
        requested,
        HashSet::from([(4, 8)]),
        "live ticks must not fan out neighbor snapshots, got {requested:?}"
    );
    assert_eq!(installer.bundles.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn live_dirty_page_only_completes_rows_leased_to_the_caller() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let owner_a = [1u8; 16];
    let owner_b = [2u8; 16];
    repository
        .mark_dirty(&overworld.id(), coordinate(1, 1), 1, &owner_a, &[1; 16], 1)
        .await
        .unwrap();
    repository
        .mark_dirty(&overworld.id(), coordinate(2, 2), 1, &owner_b, &[2; 16], 1)
        .await
        .unwrap();
    let scheduler = scheduler(
        repository.clone(),
        Arc::new(FakeBridge::default()),
        Arc::new(CapturingInstaller::default()),
    );
    let report = scheduler.run_live_dirty_page(&owner_a).await.unwrap();
    assert_eq!(report.completed, 1);
    let remaining = repository.recover().await.unwrap().dirty;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].coordinate, coordinate(2, 2));
    assert_eq!(remaining[0].owner_bridge_id, owner_b.to_vec());
}

struct NewestAlwaysTransientBridge {
    fail: ChunkCoordinate,
}

#[async_trait]
impl SnapshotBridge for NewestAlwaysTransientBridge {
    async fn request(&self, request: SnapshotRequest) -> Result<SnapshotReply, BridgeError> {
        if request.coordinate == self.fail {
            return Err(BridgeError::Transient("decode failed".into()));
        }
        Ok(SnapshotReply::Snapshot(empty_snapshot(&request)))
    }
    async fn enumerate_world(&self, _world: &WorldId) -> Result<Vec<ChunkCoordinate>, BridgeError> {
        Ok(Vec::new())
    }
}

/// Fly-in tiles must not stall behind one newest chunk whose snapshot keeps
/// failing. The live dirty tick retries Transient forever today, so the rest
/// of the page — including the player's cell — never paints.
#[tokio::test]
async fn live_dirty_page_paints_older_chunks_when_newest_snapshot_stays_transient() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let owner = [1u8; 16];
    repository
        .mark_dirty(&overworld.id(), coordinate(1, 1), 1, &owner, &[1; 16], 1)
        .await
        .unwrap();
    repository
        .mark_dirty(&overworld.id(), coordinate(9, 9), 2, &owner, &[1; 16], 2)
        .await
        .unwrap();
    let installer = Arc::new(CapturingInstaller::default());
    let scheduler = scheduler(
        repository.clone(),
        Arc::new(NewestAlwaysTransientBridge {
            fail: coordinate(9, 9),
        }),
        installer.clone(),
    );
    let report = tokio::time::timeout(
        Duration::from_millis(500),
        scheduler.run_live_dirty_page(&owner),
    )
    .await
    .expect("live dirty page must not stall on a persistently transient newest chunk")
    .unwrap();
    assert!(
        report.completed >= 1,
        "older dirty chunk must still install, got {report:?}"
    );
    let installed: HashSet<_> = installer
        .bundles
        .lock()
        .unwrap()
        .iter()
        .map(|(coordinate, _)| *coordinate)
        .collect();
    assert!(
        installed.contains(&coordinate(1, 1)),
        "player-visible older chunk must paint, installed {installed:?}"
    );
    assert!(
        !installed.contains(&coordinate(9, 9)),
        "persistently transient newest chunk must not install, installed {installed:?}"
    );
}

/// Sidecar reconnect marks every owned dirty row replay-pending. The live tick
/// must still paint those rows; otherwise fly-in tiles freeze until the next
/// higher-revision dirty event, which may never come.
#[tokio::test]
async fn live_dirty_page_paints_replay_pending_rows_after_reconnect() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let owner = [1u8; 16];
    repository
        .mark_dirty(&overworld.id(), coordinate(4, 8), 1, &owner, &[1; 16], 1)
        .await
        .unwrap();
    repository
        .reassign_bridge_lease(&owner, &[3; 16])
        .await
        .unwrap();
    let installer = Arc::new(CapturingInstaller::default());
    let scheduler = scheduler(
        repository.clone(),
        Arc::new(FakeBridge::default()),
        installer.clone(),
    );
    let report = scheduler.run_live_dirty_page(&owner).await.unwrap();
    assert_eq!(report.completed, 1);
    let installed: HashSet<_> = installer
        .bundles
        .lock()
        .unwrap()
        .iter()
        .map(|(coordinate, _)| *coordinate)
        .collect();
    assert!(installed.contains(&coordinate(4, 8)));
    assert!(repository.recover().await.unwrap().dirty.is_empty());
}

/// Live ticks must not lock onto a 1024-chunk generation backlog. Only a small
/// newest page should run so a fly-in chunk can enter the next tick quickly.
#[tokio::test]
async fn live_dirty_page_only_takes_a_small_newest_batch() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let owner = [1u8; 16];
    for x in 0..40 {
        repository
            .mark_dirty(
                &overworld.id(),
                coordinate(x, 0),
                x as u64 + 1,
                &owner,
                &[1; 16],
                x as u64 + 1,
            )
            .await
            .unwrap();
    }
    let installer = Arc::new(CapturingInstaller::default());
    let scheduler = Scheduler::new(
        repository.clone(),
        Arc::new(FakeBridge::default()),
        installer.clone(),
        SchedulerConfig {
            max_active_snapshots: 96,
            dirty_page_size: 1024,
            background_interval: Duration::from_millis(10),
            transient_retry_delay: Duration::from_millis(1),
        },
    )
    .unwrap();
    let report = scheduler.run_live_dirty_page(&owner).await.unwrap();
    assert_eq!(report.selected, 32);
    assert_eq!(report.completed, 32);
    let remaining = repository.recover().await.unwrap().dirty;
    assert_eq!(remaining.len(), 8);
    let remaining_x: HashSet<_> = remaining.iter().map(|row| row.coordinate.x).collect();
    assert!(remaining_x.contains(&0));
    assert!(!remaining_x.contains(&39), "newest chunks must be in the live batch");
}

struct NewestUnloadedBridge {
    fail_from_x: i32,
}

#[async_trait]
impl SnapshotBridge for NewestUnloadedBridge {
    async fn request(&self, request: SnapshotRequest) -> Result<SnapshotReply, BridgeError> {
        if request.coordinate.x >= self.fail_from_x {
            return Ok(SnapshotReply::Missing(ChunkMissingReason::Unloaded));
        }
        Ok(SnapshotReply::Snapshot(empty_snapshot(&request)))
    }
    async fn enumerate_world(&self, _world: &WorldId) -> Result<Vec<ChunkCoordinate>, BridgeError> {
        Ok(Vec::new())
    }
}

/// A generation backlog of newest unloaded chunks must not starve the player's
/// already-loaded cell on the next live tick. Deferral has to honor retry
/// backoff so the live page can move on.
#[tokio::test]
async fn live_dirty_page_paints_player_cell_after_newest_unloaded_chunks_defer() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let owner = [1u8; 16];
    for x in 0..40 {
        repository
            .mark_dirty(
                &overworld.id(),
                coordinate(x, 0),
                x as u64 + 1,
                &owner,
                &[1; 16],
                x as u64 + 1,
            )
            .await
            .unwrap();
    }
    let installer = Arc::new(CapturingInstaller::default());
    let scheduler = Scheduler::new(
        repository.clone(),
        Arc::new(NewestUnloadedBridge { fail_from_x: 8 }),
        installer.clone(),
        SchedulerConfig {
            max_active_snapshots: 96,
            dirty_page_size: 1024,
            background_interval: Duration::from_millis(10),
            transient_retry_delay: Duration::from_millis(1),
        },
    )
    .unwrap();
    let first = scheduler.run_live_dirty_page(&owner).await.unwrap();
    assert_eq!(first.selected, 32);
    assert_eq!(first.completed, 0, "newest 32 are unloaded and must not install");
    let second = scheduler.run_live_dirty_page(&owner).await.unwrap();
    assert!(
        second.completed >= 1,
        "player-visible older chunks must paint after newest unload deferral, got {second:?}"
    );
    let installed: HashSet<_> = installer
        .bundles
        .lock()
        .unwrap()
        .iter()
        .map(|(coordinate, _)| coordinate.x)
        .collect();
    assert!(
        installed.contains(&0),
        "player cell at x=0 must paint, installed {installed:?}"
    );
    assert!(
        !installed.iter().any(|x| *x >= 8),
        "unloaded newest chunks must stay uninstalled, installed {installed:?}"
    );
}

struct NewestUnavailableBridge {
    fail_from_x: i32,
}

#[async_trait]
impl SnapshotBridge for NewestUnavailableBridge {
    async fn request(&self, request: SnapshotRequest) -> Result<SnapshotReply, BridgeError> {
        if request.coordinate.x >= self.fail_from_x {
            return Ok(SnapshotReply::Missing(ChunkMissingReason::Unavailable));
        }
        Ok(SnapshotReply::Snapshot(empty_snapshot(&request)))
    }
    async fn enumerate_world(&self, _world: &WorldId) -> Result<Vec<ChunkCoordinate>, BridgeError> {
        Ok(Vec::new())
    }
}

/// Java snapshot failures for loaded-only requests are Unavailable, not
/// Unloaded. Those must still defer so a view-ring of false misses cannot
/// monopolize every live tick and leave the player on empty ocean.
#[tokio::test]
async fn live_dirty_page_paints_player_cell_after_newest_unavailable_chunks_defer() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let owner = [1u8; 16];
    for x in 0..40 {
        repository
            .mark_dirty(
                &overworld.id(),
                coordinate(x, 0),
                x as u64 + 1,
                &owner,
                &[1; 16],
                x as u64 + 1,
            )
            .await
            .unwrap();
    }
    let installer = Arc::new(CapturingInstaller::default());
    let scheduler = Scheduler::new(
        repository.clone(),
        Arc::new(NewestUnavailableBridge { fail_from_x: 8 }),
        installer.clone(),
        SchedulerConfig {
            max_active_snapshots: 96,
            dirty_page_size: 1024,
            background_interval: Duration::from_millis(10),
            transient_retry_delay: Duration::from_millis(1),
        },
    )
    .unwrap();
    let first = scheduler.run_live_dirty_page(&owner).await.unwrap();
    assert_eq!(first.selected, 32);
    assert_eq!(first.completed, 0, "newest 32 are unavailable and must not install");
    let second = scheduler.run_live_dirty_page(&owner).await.unwrap();
    assert!(
        second.completed >= 1,
        "player-visible older chunks must paint after newest unavailable deferral, got {second:?}"
    );
    let installed: HashSet<_> = installer
        .bundles
        .lock()
        .unwrap()
        .iter()
        .map(|(coordinate, _)| coordinate.x)
        .collect();
    assert!(
        installed.contains(&0),
        "player cell at x=0 must paint, installed {installed:?}"
    );
}

/// Player-focused live ticks already selected the loaded neighborhood. They
/// must be allowed to snapshot (and load if needed). loaded_only false-negatives
/// on the sidecar thread are what left the player on empty ocean.
#[tokio::test]
async fn player_focused_live_page_does_not_set_loaded_only() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let owner = [1u8; 16];
    repository
        .mark_dirty(
            &overworld.id(),
            coordinate(1, -4),
            1,
            &owner,
            &[1; 16],
            1,
        )
        .await
        .unwrap();
    let bridge = Arc::new(FakeBridge::default());
    let scheduler = scheduler(
        repository,
        bridge.clone(),
        Arc::new(CapturingInstaller::default()),
    );
    scheduler
        .run_live_dirty_page_near(&owner, Some((1, -4)))
        .await
        .unwrap();
    let requests = bridge.requests.lock().unwrap().clone();
    assert!(!requests.is_empty(), "expected snapshot requests");
    assert!(
        requests.iter().all(|request| !request.loaded_only),
        "player-focused live snapshots must load if needed, got {requests:?}"
    );
}

#[tokio::test]
async fn unfocused_live_page_keeps_loaded_only() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let owner = [1u8; 16];
    repository
        .mark_dirty(&overworld.id(), coordinate(9, 9), 1, &owner, &[1; 16], 1)
        .await
        .unwrap();
    let bridge = Arc::new(FakeBridge::default());
    let scheduler = scheduler(
        repository,
        bridge.clone(),
        Arc::new(CapturingInstaller::default()),
    );
    scheduler.run_live_dirty_page(&owner).await.unwrap();
    let requests = bridge.requests.lock().unwrap().clone();
    assert!(!requests.is_empty());
    assert!(
        requests.iter().all(|request| request.loaded_only),
        "unfocused live ticks must not schedule distant chunk loads, got {requests:?}"
    );
}
