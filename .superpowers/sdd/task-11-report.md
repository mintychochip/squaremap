# Task 11 report

## Delivered design

- Extracted Java's production chunk scan into `ChunkRenderEngine`; `AbstractRender` delegates single-chunk and column rendering through the same adapter, preserving visibility, pause, cancellation, image output, and scheduling behavior.
- Added strict, source-backed biome color data and deterministic descriptor export. Registry descriptors now carry air/glass/fluid/tint semantics needed by Rust, with immutable per-replacement generations.
- Added Rust `RenderContext`, biome selector/blending, exact x-major 16x16 chunk rendering, north/south validation, post-center `south_edge`, increasing-Z region carry, and a nonmutating south-row helper. Rendering rejects foreign generations and malformed public snapshots before producing pixels.
- Pinned each snapshot request to the exact registry generation observed for that request. A delayed response retains generation A even after an equal-revision generation B replacement.

## Canonical corpus

- Root: `testdata/bridge/v2/render`
- Schema/generator: `2` / `chunk-render-java-v2`
- Valid cases: 26, covering flat/empty/missing-neighbor, traversal, ceilings, max-height clipping, fluids, glass, invisible blocks, all biome categories/blends, and negative minimum Y.
- Malformed cases: 14, covering descriptors, heightmaps, palettes, bounds, CRC/length, section structure, packing/trailing bits, protobuf body, generation, and neighbor coordinates.
- The explicit overwrite generator builds from a fresh in-memory Java catalog, writes sidecars atomically, and moves `manifest.json` last. Compare-only tests require byte-for-byte equality and never write.
- The Rust loader requires the exact case order, settings, topology, case-local paths, generation, coordinates, metadata, 256 pixels, 16 edge heights, biome-source order, selector tuples, malformed metadata, complete file inventory, and no symlinks or path escape.

## Verification evidence

- Explicit generator: `./gradlew --no-daemon -Dsquaremap.regenerate=--overwrite :squaremap-common:test --tests '*ChunkRenderFixtureGenerator'` — BUILD SUCCESSFUL (`artifact://4556`).
- Regenerated inventory: 86 files; manifest SHA-256 `f71c9a0733d75a81a7a886d6ad6ffb2b019ba6c19723a8823134b253bdefc4f1`; no temporary files.
- Java compare-only renderer/fixture/selector/catalog/malformed/color/descriptor gates — BUILD SUCCESSFUL (`artifact://4559`). SHA-256 for all 86 files was identical before and after; no temporary files.
- Full Java common suite — BUILD SUCCESSFUL, 118 tests, 0 failures/errors (`artifact://4564`; current-state rerun `artifact://4610`).
- Rust exact chunk/region/decode/hardening gates — 22 passed (`artifact://4613`). This includes all 26 exact 256-pixel/16-edge oracles, all 14 typed malformed cases, strict manifest/filesystem mutations, real adjacent region sequencing, gap carry, and cancellation at entry/between columns.
- Full `squaremap-render` crate — 77 passed (`artifact://4613`).
- SnapshotClient generation/request gates — 10 passed (`artifact://4613`).
- Protocol schema/framing crate — 23 passed (`artifact://4613`).
- Criterion smoke: 26 predecoded cases per iteration, render-only timing, 10.298–11.364 K chunks/s, 0.000 allocation events/chunk, 0.0 requested bytes/chunk.
- `git diff --check` passed.

## Deferred scope

Task 16 owns comparative release thresholds. Task 11 establishes deterministic parity and measurement plumbing without setting a release performance threshold.
