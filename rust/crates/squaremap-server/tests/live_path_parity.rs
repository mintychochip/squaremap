use async_trait::async_trait;
use serde::Deserialize;
use squaremap_protocol::wire::{Point, VisibilityLimit, VisibilityLimitKind};
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

#[tokio::test]
async fn live_snapshot_bundle_includes_east_west_biome_neighbors() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let installer = Arc::new(CapturingInstaller::default());
    let scheduler = scheduler(
        repository.clone(),
        Arc::new(FakeBridge::default()),
        installer.clone(),
    );
    repository
        .mark_dirty(&overworld.id(), coordinate(4, 8), 1, &[1; 16], &[1; 16], 1)
        .await
        .unwrap();
    scheduler.run_live_dirty_page(&[1; 16]).await.unwrap();
    let bundles = installer.bundles.lock().unwrap();
    assert_eq!(bundles.len(), 1);
    let biome: HashSet<_> = bundles[0].1.iter().copied().collect();
    assert!(
        biome.contains(&(5, 8)) && biome.contains(&(3, 8)),
        "live bundle must include E/W biome neighbors, got {biome:?}"
    );
    assert!(
        !biome.is_empty(),
        "live biome neighbor list must not be hardcoded empty"
    );
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
