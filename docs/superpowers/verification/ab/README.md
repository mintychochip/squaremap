# Java/Rust A/B reports

Paired renderer and tile-pyramid measurements. Comparison is checksum-gated: graphs may still render on mismatch, but a speedup must not be published unless every pair (`run-1`/`run-2`/`run-3` and the concatenated summary) has `passed: true`.

## How to run

From the repository root:

```bash
scripts/ab-java-rust.sh
scripts/ab-java-rust.sh --workload chunk-render-v2
scripts/ab-java-rust.sh --workload pyramid-png-v2
```

The script prebuilds release binaries, runs each selected workload three times on Java then Rust, writes per-run JSON under `build/ab/<workload>/`, concatenates 90 per-pass samples into a summary, and copies `index.html`, `throughput.svg`, `pass-times.svg`, and `ab-report.json` into this directory.

## Workloads

- `chunk-render-v2` — 26 render-fixture cases, direct pixel + south-edge loop. Timed region excludes Gradle/Cargo startup, fixture load, and PNG/IO.
- `pyramid-png-v2` — 10 tile-catalog cases through Java `Image.save()` / Rust `TilePyramid`. PNG encode and tile writes are inside the timed region; decoded RGBA is the checksum.

## Graphs

`throughput.svg` is median items/s from per-pass times. `pass-times.svg` is the raw per-pass wall times. `index.html` embeds both and shows min/median/p95/max, checksums, and speedup.

Do not treat Criterion HTML (`backend_workload` or other Cargo benches) as this A/B. Those include installer, store, and scheduler layers that this harness excludes.
