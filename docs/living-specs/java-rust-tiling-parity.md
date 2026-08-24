# Java/Rust tiling parity

Last updated: 2026-08-23
Status: active

## Intent

The Rust tile pyramid must match upstream Java `Image` on decoded RGBA, paths, and downsample sample points. Keep fail-closed existing-tile validation. A/B graphs are the performance evidence surface; they may not publish a speedup if checksums differ.

## Boundaries

In scope:

- Direct Java `Image.save()` versus Rust `TilePyramid` comparison of decoded RGBA, tile paths, and downsample sample points
- Shared catalog cases at `max_zoom` 3, including solid fills, sparse rectangles, checkers, gradients, negative regions, transparency, and merge-into-existing tiles
- Fail-closed Rust validation of corrupt, wrong-type, and wrong-size existing tiles
- Paired renderer and pyramid A/B evidence using per-pass samples and graphs

Out of scope:

- Compressed PNG byte equality
- Matching Java overwrite of corrupt existing tiles
- Mixing `backend_workload`, HTTP, or scheduler numbers into renderer or pyramid speedup
- Production-path installer comparison until Next
- Live Paper tile-tree comparison until Future

## Invariants

- Comparison is decoded RGBA.
- PNG bytes may differ.
- Corrupt existing tiles are not in the equality contract (Rust fail-closed; Java overwrites — intentional hardening).

## Implementation guidance

- Java `Image.java` is the oracle (unused in production after Java backend removal, still the upstream algorithm).
- Shared case catalog at `testdata/bridge/v2/tiles/manifest.json`.
- Do not compare compressed PNG bytes.
- Do not mix backend_workload/HTTP/scheduler numbers into renderer or pyramid speedup.

Catalog pixel colors are ARGB (`0xAARRGGBB`). Region coordinates are `[x, z]`. Region-local pixel coordinates are `0..512`. Unset pixels are absent (`Integer.MIN_VALUE` on Java, missing present-bit on Rust) and must not be written; they are distinct from transparent `0x00000000`.

Pixel kinds:

- `solid` sets every region pixel to `argb`.
- `rect` sets only `[x, x + w) × [z, z + h)` to `argb`; every other region pixel stays unset.
- `checker` paints the full region: `even` where `((x / step) + (z / step)) % 2 == 0`, otherwise `odd` (integer division).
- `gradient` means `argb = 0xFF000000 | ((x & 255) << 16) | ((z & 255) << 8)` at every pixel.

Pre-existing tile content:

- `existing` is applied by writing that pattern to destination tiles before the case runs, on both backends.
- `existing_from` means first apply that prior case into the same output tree, then apply this case (shared parent at zoom ≥ 1).

## Current

- [x] Direct Java `Image.save()` vs Rust `TilePyramid` decoded-RGBA probe, zero mismatches
- [x] Sequential sparse chunk-sized updates to one region match
- [x] Negative-region + zoom 0..=3 path/origin/pixel probe
- [x] Paired renderer A/B with per-pass samples and graphs
- [x] Paired pyramid A/B with per-pass samples and graphs
- [x] Production-path installer (chunk → region local coords → pyramid) vs Java Image fed the same chunk pixels. Owned by `docs/living-specs/java-rust-equivalence.md` Layer 1.

## Next

None. Live Paper tile-tree comparison remains Future.

## Future

Live Paper tile-tree comparison.

## Decisions log

- 2026-08-21: Keep Rust fail-closed on corrupt/wrong-type/wrong-size existing tiles; do not match Java overwrite.
- 2026-08-21: A/B graphs use per-pass samples; no speedup published unless checksums match.
- 2026-08-21: Java `Image.save()` vs Rust `TilePyramid` decoded-RGBA probe was zero-mismatch on first paired run (10 catalog cases). No pyramid compositing fix required.
- 2026-08-21: Paired A/B checksums matched on all three repeats plus the 90-sample summaries. Renderer `chunk-render-v2` median 5619.63 Java vs 12540.46 Rust items/s (2.23×). Pyramid `pyramid-png-v2` median 59.054 Java vs 101.713 Rust items/s (1.72×). Not a universal backend speedup.

## Open questions

None recorded.
