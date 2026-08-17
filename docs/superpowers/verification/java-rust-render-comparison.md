# Java/Rust renderer comparison

## Fair workload contract

The primary comparison must render the same 26 valid cases from `testdata/bridge/v2/render/manifest.json`, in manifest order, using the same snapshot inputs, settings, biome sources, and neighbor relationships. One item is one 256-pixel render plus its 16-value south edge.

The primary timing excludes fixture decoding, registry construction, engine/context construction, JUnit assertions, PNG encoding, filesystem output, scheduler, bridge, repository, HTTP, Gradle startup, and Cargo compilation. Each implementation performs an untimed validation pass, then identical warmup and measured corpus passes. Every measured pass contributes a deterministic checksum; output validation occurs outside the timed region. Both reports must include the manifest hash, case count, warmup/measured passes, elapsed time, throughput, checksum, and mismatch count.

Production-path benchmarks remain separate: they include different layers and must not be used as the renderer-only Java/Rust speedup.

## Current Java driver

`common/src/test/java/xyz/jpenilla/squaremap/common/task/render/ChunkRenderBenchmarkTest.java` now:

- Reuses the existing Java fixture catalog and canonical `ChunkRenderEngine`.
- Prepares engines and expected results before timing.
- Runs 10 warmup passes and 30 measured passes over 26 cases.
- Times only the render/checksum loop.
- Validates measured output after timing.
- Emits stable JSON-like benchmark output.

Run with:

```bash
./gradlew :squaremap-common:test \
  --tests '*ChunkRenderBenchmarkTest' \
  -Dsquaremap.renderBenchmark=true \
  --no-daemon --console=plain --rerun-tasks
```

Fresh result:

```json
{"backend":"java","case_count":26,"warmup_passes":10,"measured_passes":30,"elapsed_nanos":174362601,"items_per_second":4473.436365,"checksum":-5578976407213116928}
```

The targeted Gradle test passed.

## Remaining work before a speedup claim

The Rust renderer-only benchmark still measures `RenderTileInstaller` plus configuration and `MemoryTileStore`, not the same direct renderer loop. It must gain a direct fixture loop with the same 10/30 pass protocol and checksum semantics. The Java checksum must then be compared with a Rust checksum generated from the same canonical fixture output; no speedup may be reported until parity and matched timing are both confirmed.
