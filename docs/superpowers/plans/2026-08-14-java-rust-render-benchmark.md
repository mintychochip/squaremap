# Fair Java/Rust backend rendering comparison

## Goal

Measure Java and Rust rendering performance on the same workload without mixing backend orchestration, HTTP, build startup, or output-file effects.

## Shared workload contract

- Input corpus: `testdata/bridge/v2/render/manifest.json` and referenced files.
- Exactly 26 valid cases, in manifest order.
- Each case uses identical center, north, south, and biome-source snapshots.
- Each case uses manifest settings: height limit, iterate direction, glass/water/lava behavior, biome enablement/blend, invisible block, and iterate-up base IDs.
- One timed item means one case rendered to the same logical result: 256 ARGB pixels plus the 16-value south edge.
- Both implementations must validate the complete result against the fixture oracle outside the timed region.
- No scheduler, bridge, repository, HTTP, PNG encoding, filesystem writes, or test framework startup in the primary renderer comparison.

## Harness shape

Use one long-lived process per backend. Load/decode fixtures, initialize registries, allocate reusable buffers, and perform one untimed validation pass before timing. Run the 26 cases in fixed order for every iteration. Emit JSON with case count, iterations, elapsed nanoseconds, items/s, per-case totals, checksum, and validation status.

Rust should expose a dedicated renderer-only harness matching the Java loop. Existing `backend_workload` includes `RenderTileInstaller`, async installation, configuration, and `MemoryTileStore`; retain it as a separate production-path benchmark, not the apples-to-apples renderer number.

Java should add an opt-in benchmark entry point beside the fixture tests. It can reuse `ChunkRenderFixtureCatalog` and `ChunkRenderEngine`, but timing must call `renderChunkResult` for each catalog case with fresh or explicitly reusable state equivalent to Rust. Avoid JUnit assertions and Gradle task setup inside the timed loop.

## Timing rules

- Same machine, OS, CPU governor/power mode, JDK/Rust release builds, and idle background load.
- Java: release-compiled test/runtime classpath, fixed heap flags documented in report, one JVM process; do not include Gradle startup.
- Rust: `--release` binary; do not include Cargo compilation or Criterion plotting.
- Warm up both implementations for the same number of full corpus passes, then collect the same number of measured passes.
- Use monotonic wall-clock timing around only the 26-case loop. Report both corpus passes/s and case items/s.
- Use a deterministic checksum over all pixels and south-edge values; fail if Java and Rust checksums differ from their shared oracle or each other.
- Run at least 30 measured samples/passes after warmup; report median, p95, min/max, and raw sample count. Repeat the complete run three times and retain all raw JSON.
- Do not compare RSS unless measuring equivalent process boundaries. If memory is required, report peak RSS separately for each standalone renderer process and do not combine it with Paper, Gradle, or Criterion processes.

## Acceptance gates

1. Both reports say `case_count=26`, same input manifest hash, same warmup and measured pass counts.
2. Both report zero oracle mismatches and identical result checksums.
3. Primary comparison excludes I/O, PNG encoding, scheduler, bridge, repository, HTTP, and build startup.
4. Statistics are based on identical sample protocol; no speedup is published if any gate fails.
5. Production-path benchmarks remain separately labeled because they measure different contracts.
