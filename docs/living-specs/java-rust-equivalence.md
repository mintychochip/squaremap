# Java/Rust equivalence probes — Living Spec
> Status: active
> Last updated: 2026-08-23

## Intent

CI must prove the Rust backend is observably the same as the deleted Java backend on tiles, HTTP, and JSON the map client sees. Claims are named and fail-closed. Live Paper is not this domain.

## Boundaries

### In scope

- Production installer (chunk → region-local coords → pyramid) vs Java `Image`
- Frozen Java RGBA hashes and JSON fixtures as oracles
- HTTP GET/HEAD contract table encoding Undertow/IntegratedServer semantics
- Production view writers served through in-process `HttpServer`
- Named deliberate divergences (cache headers, method rejection, path confinement, no h2c)

### Out of scope / non-goals

- Live Paper worlds, plugin lifecycle, or sidecar stdin handshake
- Restoring Java HTTP/renderer production types
- Compressed PNG byte equality
- Matching Java overwrite of corrupt tiles
- Performance, packaging, canary, cutover

## Invariants

- Tile equality is decoded RGBA; PNG bytes may differ.
- JSON equality is semantic.
- Missing oracles, empty denominators, and undocumented mismatches fail closed.
- Tests do not skip when an env var is unset.
- Live Paper evidence cannot be inferred from these probes.

## Implementation guidance

- Java `Image.java` and `StateExporterTest` remain the algorithm/exporter oracles.
- Rust probes use `RenderTileInstaller`, `HttpServer`, and `views::apply_replacement` — not reimplemented mapping or static fixture copies served as if they were written.
- Reuse `squaremap-compare` JSON/PNG comparators; do not add a third equality helper.
- HTTP match vs deliberate rows live in one table (`testdata/bridge/v1/http-java-contract.json`).
- Design: `docs/superpowers/specs/2026-08-23-java-rust-equivalence-probes.md`.

## Current

- [x] Approach chosen: three explicit CI layers, no Paper in default suite
- [x] Layer 1: installer catalog of render-fixture+chunk placements + committed Java hashes + `ProductionInstallerOracleTest` + `java_installer_equivalence.rs`
- [x] Close `tile_pyramid_parity` silent skip with committed `v2/tiles/java-oracle-hashes.json`
- [x] Layer 2: `http-java-contract.json` + `java_http_contract.rs` (match + deliberate rows)
- [x] Layer 3: `java_view_http_equivalence.rs` production writers + HTTP vs `fixtures/java`
- [x] Pin `StateExporterTest` outputs to the same frozen Java JSON fixtures Layer 3 fetches (exporter drift detection)

## Next

None. Live Paper remains Future.

## Future

- [ ] Live Paper tile-tree comparison
- [ ] Optional `SQUAREMAP_PAPER_PROBE=1` gate (fail closed without artifacts)

## Decisions log

| Date | Decision | Why |
|---|---|---|
| 2026-08-23 | Three named CI layers; no live Paper in default suite | Strongest honest proof without blocked infrastructure |
| 2026-08-23 | Frozen hashes/fixtures, not env-gated Java dumps | Cargo test must not silently skip |
| 2026-08-23 | Deliberate HTTP divergences are counted rows | Explicitness: hardening must not look like an accidental match |
| 2026-08-23 | Worlds JSON is ordered by `order` then identity | Matches Java exporter / frontend `order` sort; identity-only sort put nether first |
| 2026-08-23 | MultiPolygon serializes as `"type":"polygon"` | Matches Java `UpdateMarkers` and the web frontend dispatcher |

## Open questions

- [x] Which surfaces? All three, explicit names.
- [x] CI vs Paper? CI-only deterministic.
}
