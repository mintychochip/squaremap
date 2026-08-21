# Java/Rust renderer and tile comparison

Matched-protocol status as of 2026-08-21. Renderer and pyramid are separate workloads. These numbers are not a universal backend speedup: they do not include live Paper, HTTP, the scheduler, the installer, or Criterion `backend_workload`.

## Fair workload contract

The renderer comparison must render the same 26 valid cases from `testdata/bridge/v2/render/manifest.json`, in manifest order, using the same snapshot inputs, settings, biome sources, and neighbor relationships. One item is one 256-pixel render plus its 16-value south edge.

The primary renderer timing excludes fixture decoding, registry construction, engine/context construction, JUnit assertions, PNG encoding, filesystem output, scheduler, bridge, repository, HTTP, Gradle startup, and Cargo compilation. Each implementation performs an untimed validation pass, then identical warmup and measured corpus passes. Every measured pass contributes a deterministic checksum; output validation occurs outside the timed region. Both reports include the manifest hash, case count, warmup/measured passes, elapsed time, throughput, checksum, and per-pass samples.

The pyramid comparison is a different contract: Java `Image.save()` versus Rust `TilePyramid` over the 10 catalog cases at `testdata/bridge/v2/tiles/manifest.json`. PNG encode and tile writes are inside the timed region. Equality is decoded RGBA plus paths, not compressed PNG bytes.

Production-path benchmarks remain separate: they include different layers and must not be used as the renderer-only or pyramid-only speedup.

## Tiling functional equivalence

First paired Java/Rust tile pyramid probe (commit `70f1209`, recorded in `06cafba`): 10 catalog cases, decoded RGBA and paths, zero mismatches. Rust remains fail-closed on corrupt, wrong-type, and wrong-size existing tiles; Java overwrites those tiles. That divergence is intentional hardening, not part of the equality contract.

## Renderer A/B (`chunk-render-v2`)

Drivers:

- Java: `common/src/test/java/xyz/jpenilla/squaremap/common/task/render/ChunkRenderBenchmarkTest.java`
- Rust: `rust/crates/squaremap-render/src/bin/render_protocol.rs`

Protocol: 10 warmup passes, 30 measured passes per process, three process repeats (90 samples). Checksum `-5578976407213116928` on both backends (historical matched value). Graphs: [chunk-render-v2/index.html](ab/chunk-render-v2/index.html).

Summary `ab-report.json` from this run (`passed: true`, 90 samples):

- Java median: 5619.627716954914 items/s
- Rust median: 12540.461074167662 items/s
- Speedup: 2.231546590948006

## Pyramid A/B (`pyramid-png-v2`)

Drivers:

- Java: `common/src/test/java/xyz/jpenilla/squaremap/common/data/ImagePyramidBenchmarkTest.java`
- Rust: `rust/crates/squaremap-render/src/bin/tile_protocol.rs`

Protocol: 10 warmup passes, 30 measured passes per process, three process repeats (90 samples). Checksum `-4734813580563762388` on both backends. Graphs: [pyramid-png-v2/index.html](ab/pyramid-png-v2/index.html).

Summary `ab-report.json` from this run (`passed: true`, 90 samples):

- Java median: 59.054130232286 items/s
- Rust median: 101.71315983678903 items/s
- Speedup: 1.722371651850704

## How to regenerate

```bash
scripts/ab-java-rust.sh
```

Java is launched through Gradle; the timed region is inside the test JVM. Gradle startup, task graph, and compilation are outside that region. Rust uses `--release` protocol binaries; Cargo compilation is outside the timed region.

## Caveats

- Not live Paper.
- Not HTTP or scheduler.
- Java timed inside a Gradle JVM; Gradle startup is excluded from the timed region.
- PNG encode is included only in the pyramid workload.
- One machine, this run only.
- Do not mix renderer items/s with pyramid items/s, and do not treat Criterion HTML as this A/B.
