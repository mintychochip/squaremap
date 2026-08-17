//! Benchmark for the v2 strict chunk-render corpus (`testdata/bridge/v2/render`).
//!
//! Measures all 26 valid cases with a counting global allocator. Load and
//! predecoding happen once, outside Criterion; the timed section contains
//! render calls only. Denominators are exact: each Criterion iteration renders
//! all 26 cases, so the sample denominator is `iters * 26`.

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[path = "../tests/support/fixtures.rs"]
mod fixtures;

use fixtures::{FixtureCorpus, PreparedCase};

/// Root of the v2 strict render corpus, resolved from the crate manifest at
/// compile time so the benchmark is independent of the invocation cwd.
const CORPUS_ROOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../testdata/bridge/v2/render"
);

// ---------------------------------------------------------------------------
// Counting global allocator
// ---------------------------------------------------------------------------

/// Snapshot of allocation events and requested bytes since the last reset.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct AllocCounters {
    alloc_events: u64,
    alloc_bytes: u64,
    realloc_events: u64,
    realloc_bytes: u64,
    alloc_zeroed_events: u64,
    alloc_zeroed_bytes: u64,
}

/// Global allocator that counts allocation events and requested bytes.
///
/// Deallocations are intentionally ignored. `realloc` counts its event and the
/// requested new size; `alloc_zeroed` counts its event and requested size.
/// Counters are `AtomicU64` and updated with relaxed ordering (single-threaded
/// measurement with delta snapshots, so ordering is irrelevant to correctness).
struct CountingAllocator;

static ALLOC_EVENTS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static REALLOC_EVENTS: AtomicU64 = AtomicU64::new(0);
static REALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static ALLOC_ZEROED_EVENTS: AtomicU64 = AtomicU64::new(0);
static ALLOC_ZEROED_BYTES: AtomicU64 = AtomicU64::new(0);

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

impl CountingAllocator {
    /// Zero every counter. Call immediately before the timed section so
    /// ambient allocations (warm-up, sample bookkeeping) are excluded.
    fn reset() {
        ALLOC_EVENTS.store(0, Ordering::Relaxed);
        ALLOC_BYTES.store(0, Ordering::Relaxed);
        REALLOC_EVENTS.store(0, Ordering::Relaxed);
        REALLOC_BYTES.store(0, Ordering::Relaxed);
        ALLOC_ZEROED_EVENTS.store(0, Ordering::Relaxed);
        ALLOC_ZEROED_BYTES.store(0, Ordering::Relaxed);
    }

    /// Total events/bytes accumulated since the last `reset`.
    fn snapshot() -> AllocCounters {
        AllocCounters {
            alloc_events: ALLOC_EVENTS.load(Ordering::Relaxed),
            alloc_bytes: ALLOC_BYTES.load(Ordering::Relaxed),
            realloc_events: REALLOC_EVENTS.load(Ordering::Relaxed),
            realloc_bytes: REALLOC_BYTES.load(Ordering::Relaxed),
            alloc_zeroed_events: ALLOC_ZEROED_EVENTS.load(Ordering::Relaxed),
            alloc_zeroed_bytes: ALLOC_ZEROED_BYTES.load(Ordering::Relaxed),
        }
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_EVENTS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: forwarded verbatim to the system allocator with the same layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        REALLOC_EVENTS.fetch_add(1, Ordering::Relaxed);
        REALLOC_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOC_ZEROED_EVENTS.fetch_add(1, Ordering::Relaxed);
        ALLOC_ZEROED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.alloc_zeroed(layout) }
    }
}

// ---------------------------------------------------------------------------
// Corpus: loaded and predecoded exactly once, outside Criterion
// ---------------------------------------------------------------------------

/// The 26 valid v2 cases, each fully predecoded (registry, contexts, chunk
/// snapshots, biome sources). Built lazily on first use, never inside a
/// measured sample.
static CORPUS: LazyLock<Vec<PreparedCase>> = LazyLock::new(|| {
    let corpus = FixtureCorpus::load(Path::new(CORPUS_ROOT))
        .unwrap_or_else(|e| panic!("v2 render corpus load failed: {e}"));
    assert_eq!(
        corpus.cases.len(),
        26,
        "v2 render corpus must contain exactly 26 valid cases"
    );
    corpus.cases
});

// ---------------------------------------------------------------------------
// Benchmark
// ---------------------------------------------------------------------------

fn bench_render_v2_corpus(c: &mut Criterion) {
    let cases: &[PreparedCase] = &*CORPUS;
    assert_eq!(
        cases.len(),
        26,
        "benchmark denominator is exactly 26 valid cases"
    );
    let mut group = c.benchmark_group("chunk_render/v2");
    group.throughput(Throughput::Elements(26));
    group.bench_function("strict-corpus", |b| {
        b.iter_custom(|iters| {
            CountingAllocator::reset();
            let start = Instant::now();
            for _ in 0..iters {
                for case in cases {
                    let output = fixtures::render_case(case).unwrap_or_else(|error| panic!("{}: {error:?}", case.row.id));
                    black_box(output);
                }
            }
            let elapsed = start.elapsed();
            let counters = CountingAllocator::snapshot();
            let rendered = iters * 26;
            let secs = elapsed.as_secs_f64();
            let chunks_per_sec = rendered as f64 / secs;
            let allocation_events = counters.alloc_events + counters.realloc_events + counters.alloc_zeroed_events;
            let requested_bytes = counters.alloc_bytes + counters.realloc_bytes + counters.alloc_zeroed_bytes;
            eprintln!("chunk_render/v2: render_denominator=26 samples={rendered} chunks/s={chunks_per_sec:.1} allocation_events/chunk={:.3} requested_bytes/chunk={:.1}", allocation_events as f64 / rendered as f64, requested_bytes as f64 / rendered as f64);
            elapsed
        })
    });
    group.finish();
}

criterion_group!(benches, bench_render_v2_corpus);
criterion_main!(benches);
