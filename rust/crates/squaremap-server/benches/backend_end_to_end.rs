use async_trait::async_trait;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use reqwest::StatusCode;
use serde::Serialize;
use squaremap_render::{PngOptions, RenderSettings, Snapshot, fixture::FixtureCorpus};
use squaremap_server::http::{HttpConfig, HttpServer};
use squaremap_server::output::OutputRoot;
use squaremap_server::scheduler::{
    BridgeError, RenderTileInstaller, Scheduler, SchedulerConfig, SnapshotBridge, SnapshotReply,
    SnapshotRequest, WorldRenderConfig,
};
use squaremap_state::{ChunkCoordinate, JobKind, Repository, World, WorldId};
use std::{path::Path, sync::Arc, time::Instant};
use tempfile::tempdir;
use tokio::runtime::Runtime;

const CORPUS_ROOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../testdata/bridge/v2/render"
);
const WORKLOAD_THRESHOLD: f64 = 100.0;

#[derive(Serialize)]
struct WorkloadReport {
    corpus_cases: usize,
    iterations: u64,
    elapsed_nanos: u128,
    items_per_second: f64,
    threshold: f64,
    published_paths: Vec<String>,
    passed: bool,
    scope: &'static str,
}

struct CorpusBridge {
    cases: Vec<(
        ChunkCoordinate,
        Arc<Snapshot>,
        Option<Arc<Snapshot>>,
        Option<Arc<Snapshot>>,
    )>,
}
#[async_trait]
impl SnapshotBridge for CorpusBridge {
    async fn request(&self, request: SnapshotRequest) -> Result<SnapshotReply, BridgeError> {
        let coordinate = request.coordinate;
        for (expected, center, north, south) in &self.cases {
            if *expected == coordinate {
                if coordinate.z < 0 {
                    return Ok(north.clone().map(SnapshotReply::Snapshot).unwrap_or(
                        SnapshotReply::Missing(
                            squaremap_protocol::wire::ChunkMissingReason::Unloaded,
                        ),
                    ));
                }
                if coordinate.z > 0 {
                    return Ok(south.clone().map(SnapshotReply::Snapshot).unwrap_or(
                        SnapshotReply::Missing(
                            squaremap_protocol::wire::ChunkMissingReason::Unloaded,
                        ),
                    ));
                }
                return Ok(SnapshotReply::Snapshot(center.clone()));
            }
        }
        Ok(SnapshotReply::Missing(
            squaremap_protocol::wire::ChunkMissingReason::Unloaded,
        ))
    }
    async fn enumerate_world(
        &self,
        _world: &squaremap_state::WorldId,
    ) -> Result<Vec<ChunkCoordinate>, BridgeError> {
        Ok(self
            .cases
            .iter()
            .map(|(coordinate, _, _, _)| *coordinate)
            .collect())
    }
}

fn benchmark_backend_end_to_end(c: &mut Criterion) {
    let corpus = FixtureCorpus::load(Path::new(CORPUS_ROOT)).expect("valid v2 render corpus");
    assert_eq!(corpus.cases.len(), 26, "fixed backend workload denominator");
    let runtime = Runtime::new().expect("runtime");
    let setup = runtime.block_on(async {
        let directory = tempdir().expect("temporary output root");
        let output = OutputRoot::new(directory.path()).expect("output root");
        let repository = Arc::new(
            Repository::open(directory.path().join("state.sqlite"))
                .await
                .expect("repository"),
        );
        let world = WorldId::new("minecraft", "overworld", 1);
        repository
            .apply_world(World::new(
                world.namespace.clone(),
                world.value.clone(),
                world.epoch,
                Vec::new(),
            ))
            .await
            .expect("world");
        let tile_store: Arc<dyn squaremap_render::TileStore> = Arc::new(output.clone());
        let installer = Arc::new(RenderTileInstaller::new(tile_store));
        let cases = corpus
            .cases
            .iter()
            .enumerate()
            .map(|(index, case)| {
                (
                    ChunkCoordinate {
                        x: index as i32,
                        z: 0,
                    },
                    case.center.clone(),
                    case.north.clone(),
                    case.south.clone(),
                )
            })
            .collect();
        let bridge = Arc::new(CorpusBridge { cases });
        for (index, case) in corpus.cases.iter().enumerate() {
            installer
                .configure_world(
                    world.clone(),
                    WorldRenderConfig {
                        settings: RenderSettings {
                            iterate_up: case.row.iterate_up,
                            map_max_height: case.row.max_height,
                            biomes_enabled: case.row.biome_enabled,
                            biome_blend: case.row.biome_blend,
                            glass_clear: case.row.glass_clear,
                            water_clear: case.row.water_clear,
                            water_checkerboard: case.row.water_checkerboard,
                            lava_checkerboard: case.row.lava_checkerboard,
                        },
                        invisible_ids: [case.row.invisible_id]
                            .into_iter()
                            .filter(|id| *id != 0)
                            .collect::<Vec<_>>()
                            .into(),
                        iterate_up_base_ids: [case.row.iterate_up_base_id]
                            .into_iter()
                            .filter(|id| *id != 0)
                            .collect::<Vec<_>>()
                            .into(),
                        biome_zoom_seed: corpus.manifest.biome_zoom_seed,
                        max_zoom: 3,
                        png_options: PngOptions::default(),
                        tile_prefix: format!("tiles/minecraft_overworld/{index}").into(),
                    },
                )
                .expect("configure installer");
        }
        let scheduler = Arc::new(
            Scheduler::new(
                repository,
                bridge,
                installer,
                SchedulerConfig {
                    max_active_snapshots: 96,
                    dirty_page_size: 64,
                    background_interval: std::time::Duration::from_secs(60),
                    transient_retry_delay: std::time::Duration::from_millis(1),
                },
            )
            .expect("scheduler"),
        );
        let server = HttpServer::bind(HttpConfig::loopback(), output.clone())
            .await
            .expect("http server");
        let address = server.local_addr().expect("http address");
        (directory, output, world, scheduler, server, address)
    });
    let (directory, output, world, scheduler, mut server, address) = setup;
    let client = reqwest::Client::new();
    let http_urls: Vec<String> = (0..corpus.cases.len())
        .map(|index| format!("http://{address}/tiles/minecraft_overworld/{index}/0_0.png"))
        .collect();
    let mut group = c.benchmark_group("backend_end_to_end/v2");
    group.throughput(Throughput::Elements(corpus.cases.len() as u64));
    let mut reported = false;
    group.bench_function("scheduler_bridge_repository_installer_output", |b| {
        b.iter_custom(|iterations| {
            let start = Instant::now();
            runtime.block_on(async {
                for _ in 0..iterations {
                    let coordinates: Vec<_> = (0..corpus.cases.len())
                        .map(|index| ChunkCoordinate { x: index as i32, z: 0 })
                        .collect();
                    let job = scheduler
                        .start_job_at_revision(world.clone(), JobKind::Full, coordinates, 1)
                        .await
                        .expect("start render job");
                    scheduler.run_job(&job.id).await.expect("end-to-end render");
                    for (index, url) in http_urls.iter().enumerate() {
                        let response = client.get(url).send().await.expect("representative HTTP GET");
                        assert!(
                            matches!(response.status(), StatusCode::NOT_FOUND | StatusCode::OK),
                            "unexpected HTTP status for item {index}: {}",
                            response.status()
                        );
                    }
                }
            });
            let elapsed = start.elapsed();
            if !reported {
                reported = true;
                let nanos = elapsed.as_nanos().max(1);
                let items = iterations.saturating_mul(corpus.cases.len() as u64);
                let rate = items as f64 * 1_000_000_000.0 / nanos as f64;
                let published = output.existing_files("tiles/minecraft_overworld").expect("published files");
                println!("{}", serde_json::to_string_pretty(&WorkloadReport {
                    corpus_cases: corpus.cases.len(),
                    iterations,
                    elapsed_nanos: nanos,
                    items_per_second: rate,
                    threshold: WORKLOAD_THRESHOLD,
                    published_paths: published.iter().map(|path| path.display().to_string()).collect(),
                    passed: rate >= WORKLOAD_THRESHOLD && !published.is_empty(),
                    scope: "Scheduler -> SnapshotBridge -> Repository -> RenderTileInstaller -> OutputRoot -> HTTP GET per item",
                }).expect("benchmark report"));
                assert!(rate >= WORKLOAD_THRESHOLD, "end-to-end throughput below threshold: {rate}");
            }
            elapsed
        });
    });
    group.finish();
    runtime.block_on(async {
        server.shutdown().await.expect("http shutdown");
    });
    drop(directory);
}

criterion_group!(benches, benchmark_backend_end_to_end);
criterion_main!(benches);
