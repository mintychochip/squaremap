use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use serde::Serialize;
use squaremap_render::{fixture::FixtureCorpus, MemoryTileStore, PngOptions, RenderSettings};
use squaremap_server::scheduler::{InstallRequest, RenderTileInstaller, TileInstaller, WorldRenderConfig};
use squaremap_state::{ChunkCoordinate, WorldId};
use std::{path::Path, sync::Arc, time::Instant};
use tokio::runtime::Runtime;

const CORPUS_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../testdata/bridge/v2/render");
const WORKLOAD_THRESHOLD: f64 = 100.0;

#[derive(Serialize)]
struct WorkloadReport {
    corpus_cases: usize,
    iterations: u64,
    elapsed_nanos: u128,
    items_per_second: f64,
    threshold_items_per_second: f64,
    published_paths: usize,
    passed: bool,
    scope: &'static str,
}

fn benchmark_backend_workload(c: &mut Criterion) {
    let corpus = FixtureCorpus::load(Path::new(CORPUS_ROOT)).expect("valid v2 render corpus");
    assert_eq!(corpus.cases.len(), 26, "fixed backend workload denominator");
    let store = Arc::new(MemoryTileStore::new());
    let installer = Arc::new(RenderTileInstaller::new(store.clone()));
    let world = WorldId { namespace: "minecraft".into(), value: "overworld".into(), epoch: 1 };
    let runtime = Runtime::new().expect("runtime");
    let mut group = c.benchmark_group("backend_workload/v2");
    group.throughput(Throughput::Elements(corpus.cases.len() as u64));
    let mut measured = false;
    group.bench_function("render_tile_installer", |b| {
        b.iter_custom(|iterations| {
            let start = Instant::now();
            runtime.block_on(async {
                for _ in 0..iterations {
                    for (index, case) in corpus.cases.iter().enumerate() {
                        let row = &case.row;
                        let settings = RenderSettings {
                            iterate_up: row.iterate_up,
                            map_max_height: row.max_height,
                            biomes_enabled: row.biome_enabled,
                            biome_blend: row.biome_blend,
                            glass_clear: row.glass_clear,
                            water_clear: row.water_clear,
                            water_checkerboard: row.water_checkerboard,
                            lava_checkerboard: row.lava_checkerboard,
                        };
                        let invisible_ids: Vec<_> = [row.invisible_id].into_iter().filter(|id| *id != 0).collect();
                        let iterate_ids: Vec<_> = [row.iterate_up_base_id].into_iter().filter(|id| *id != 0).collect();
                        installer.configure_world(world.clone(), WorldRenderConfig {
                            settings,
                            invisible_ids: invisible_ids.into(),
                            iterate_up_base_ids: iterate_ids.into(),
                            biome_zoom_seed: corpus.manifest.biome_zoom_seed,
                            max_zoom: 3,
                            png_options: PngOptions::default(),
                            tile_prefix: "world".into(),
                        }).expect("configure production installer");
                        let biome_sources = case.biome_source.as_ref().map(|source| source.snapshots()).unwrap_or_default();
                        installer.install(InstallRequest {
                            world: world.clone(),
                            coordinate: ChunkCoordinate { x: index as i32, z: 0 },
                            revision: 1,
                            snapshots: squaremap_server::scheduler::SnapshotBundle {
                                north: case.north.clone(),
                                center: case.center.clone(),
                                south: case.south.clone(),
                                biome_sources,
                                grass_resolutions: case.row.grass_samples.iter().map(|sample| (
                                    sample.block_x,
                                    sample.block_y,
                                    sample.block_z,
                                    sample.biome_id,
                                    sample.resolved_grass_argb as u32,
                                )).collect(),
                            },
                        }).await.unwrap_or_else(|error| panic!("production tile installation case {}: {}", row.id, error));
                    }
                }
            });
            let elapsed = start.elapsed();
            let published_paths = store.file_count();
            if !measured {
                measured = true;
                let nanos = elapsed.as_nanos().max(1);
                let items = iterations.saturating_mul(corpus.cases.len() as u64);
                let rate = items as f64 * 1_000_000_000.0 / nanos as f64;
                println!("{}", serde_json::to_string_pretty(&WorkloadReport {
                    corpus_cases: corpus.cases.len(),
                    iterations,
                    elapsed_nanos: nanos,
                    items_per_second: rate,
                    threshold_items_per_second: WORKLOAD_THRESHOLD,
                    published_paths,
                    passed: rate >= WORKLOAD_THRESHOLD,
                    scope: "RenderTileInstaller with MemoryTileStore; excludes Scheduler, bridge, repository, and HTTP",
                }).expect("benchmark report"));
            }
            elapsed
        });
    });
    group.finish();
}

criterion_group!(benches, benchmark_backend_workload);
criterion_main!(benches);
