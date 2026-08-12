# Task 12 report

## TDD evidence

- RED: `cargo test -p squaremap-render --test tile_pyramid` failed before production work (`artifact://4672`) because the tile-pyramid API did not exist.
- GREEN: the focused suite now exercises exact zoom paths and origins, all four quadrants, negative regions, sparse 16x16 overwrite, explicit transparency, neighboring-pixel preservation, compression modes, malformed/wrong-format PNGs, maximum zoom, concurrent writers sharing a parent, cancellation while waiting for a destination lock, pre-publication failure, and post-rename directory-sync warnings (`artifact://4706`): 7/7 pass.

## Design decisions

- `RegionPixels` stores RGBA separately from a presence bitset, preserving Java's distinction between an unset pixel and numeric ARGB zero (transparent).
- Parent levels use Java's top-left point-sampling order (`x` outer, `z` inner; `2^zoom` stride), Java-compatible floor division/remainders, and paths `{level}/{x}_{z}.png`.
- `TilePyramid` registers one async mutex per destination and keeps an explicit user count. A registration guard decrements the count on normal completion, error, or future cancellation, removing the idle map entry without an ABA race.
- PNG input is strictly 512x512 RGBA8 and decoded under an 8 MiB internal allocation limit; output comparison is decoded RGBA rather than encoded bytes.
- `OutputRoot` implements the render crate's async `TileStore` using bounded `spawn_blocking` reads/writes, existing descriptor-relative confinement, a same-directory temporary file, file flush plus `sync_all` before rename, and a path-bearing warning when the post-rename directory fsync fails.

## Verification
- `cargo test -p squaremap-render --test tile_pyramid`: 7 passed (`artifact://4712`).
- `cargo test -p squaremap-server --test http_contract`: 11 passed, including confined and bounded `TileStore` publication/read (`artifact://4712`).
- `SQUAREMAP_OUTPUT_ROOT=/tmp/squaremap-task12-final cargo test -p squaremap-render`: 84 passed (`artifact://4712`).
- `SQUAREMAP_OUTPUT_ROOT=/tmp/squaremap-task12-final cargo test -p squaremap-server`: 75 passed (`artifact://4712`).
- `cargo check --workspace --all-targets`: passed (`artifact://4712`). The Windows target itself was installed; a separate cross-check remains blocked by the workstation's missing `x86_64-w64-mingw32-gcc`, not by Rust diagnostics (`artifact://4702`).
- Two requested specialist reviews and one fallback reviewer invocation were attempted; all were unavailable because the subagent service returned `usage_limit_reached`. The parent performed a line-by-line requirements, concurrency, security, and failure-mode audit instead.
