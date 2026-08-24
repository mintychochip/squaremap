# Java/Rust Equivalence Probes

**Status:** design; not a live-Paper or cutover claim  
**Date:** 2026-08-23  
**Default:** Rust is the only production backend. Java HTTP/renderer production types are deleted.  
**Oracle:** leftover Java algorithms (`Image`, chunk-render fixtures, JSON exporters) plus frozen fixtures and decoded-RGBA hashes.

## Goal

CI must prove, with named fail-closed tests, that the Rust backend is observably the same as the Java backend on three surfaces a user or map client can see:

1. Production tile installation (chunk → region-local pixels → pyramid)
2. HTTP request/response contract the old Undertow stack exposed
3. JSON/icon bytes produced by production view writers and fetched over HTTP

Each test name states the Java behavior. Each module states what it proves and what it does not. Silent skips are forbidden. Live Paper remains Future.

## What already exists (do not duplicate)

| Surface | Existing evidence | Gap |
|---|---|---|
| `TilePyramid` vs `Image.save()` | `ImagePyramidOracleTest`, `tile_pyramid_parity.rs` (10 catalog cases, zero mismatches) | Rust comparison is gated on `SQUAREMAP_JAVA_TILE_ORACLE` and no-ops when unset |
| Chunk pixels | v2 render corpus, independent Rust probe | Does not go through `RenderTileInstaller` |
| Installer self-check | `tile_install_parity.rs` sequential chunk merge | Compares Rust installer to Rust `render_case`, not Java `Image` |
| HTTP | `http_contract.rs` | Encodes Java semantics in Rust assertions; not a named Java-contract table; no frozen denominator |
| JSON fixtures | `testdata/bridge/v1/fixtures/{java,rust}` + `squaremap-compare parity` | Compares pre-written files; does not run production writers or HTTP GET |

## Evidence rules

- Comparison for tiles is **decoded RGBA** (SHA-256 of 512×512×4). Compressed PNG bytes may differ.
- Comparison for JSON is **semantic** (`serde_json::Value` / Gson equality). Array order of players/markers is not required to match if the frontend keys by uuid/id; the fixtures already use a stable order and tests keep it.
- Comparison for icons is decoded RGBA or documented `image/rgba` payload equality.
- Missing oracle files, empty denominators, unknown case IDs, and path-set mismatches fail the test.
- Tests must not `return` when an env var is unset.
- A mismatch is either a failure or a **named deliberate divergence** in the contract table. Undocumented differences fail.
- No test may claim live Paper, loader lifecycle, bridge handshake, dirty replay, or performance.

## Architecture

Three Cargo/JUnit layers, plus committed oracles under `testdata/bridge/`. Gradle proves Java algorithms still produce the oracles. Cargo proves Rust production paths still match them.

```
Java Image / exporters          frozen testdata oracles           Rust production path
----------------------          ----------------------           --------------------
Image.setPixel+save  ----hash--> v2/tiles + v2/installer   <---- RenderTileInstaller
StateExporter JSON   ----file--> v1/fixtures/java          <---- views::apply_replacement
Undertow semantics   ----table-> v1/http-java-contract.json <---- HttpServer GET/HEAD
```

Layer 3 starts an in-process `HttpServer` (same type as `serve-fixture`), not a Paper process.

## Layer 1 — Production tile path

**Proves:** `RenderTileInstaller` maps `chunk (x,z)` to region-local pixels with `floorMod(chunk, 32) * 16`, stages that region, and publishes a zoom pyramid whose decoded RGBA and relative paths match Java `Image.setPixel` + `Image.save()` for the same ARGB inputs.

**Does not prove:** Minecraft snapshots, biome/grass rendering (already a separate corpus), HTTP, scheduler jobs, Paper.

### Mapping contract (both sides)

```
regionX = floorDiv(chunkX, 32)
regionZ = floorDiv(chunkZ, 32)
localX  = floorMod(chunkX, 32) * 16
localZ  = floorMod(chunkZ, 32) * 16
for px,pz in 0..16: set pixel (localX+px, localZ+pz) to chunk pixel (px,pz)
then save pyramid at max_zoom = 3
```

Unset pixels stay unset (`Integer.MIN_VALUE` / missing present-bit). Transparent `0x00000000` is a real write. Rust stays fail-closed on corrupt existing tiles; that is a documented hardening, not part of this equality set.

### Installer catalog

Create `testdata/bridge/v2/installer/manifest.json`. Cases are ordered lists of `(chunk_x, chunk_z, render_fixture_id)`. Each fixture id is a valid case in `testdata/bridge/v2/render/manifest.json`. The 256 ARGB pixels come from that fixture’s Java oracle (Java) or from independently rendering the same snapshots (Rust). This is the production path: same chunk pixels, then installer mapping, then pyramid.

Do not serve or compare pre-written Java PNGs as the Rust result.

Required cases:

| id | installs | why |
|---|---|---|
| `fixture-flat-solid-at-0-0` | `flat-solid` at `(0,0)` | origin mapping |
| `fixture-flat-solid-at-1-0` | `flat-solid` at `(1,0)` | local origin `16,0` |
| `fixture-flat-solid-then-iterate-down` | `flat-solid` at `(0,0)` then `iterate-down` at `(1,0)` | merge two different chunk patterns into one native tile and parent downsample |
| `fixture-flat-solid-at-31-31` | `flat-solid` at `(31,31)` | last chunk of region `0,0` (`496,496`) |
| `fixture-flat-solid-at-negative` | `flat-solid` at `(-1,-1)` | region `-1,-1`, local `496,496`, negative zoom paths |
| `fixture-flat-solid-at-32-0` | `flat-solid` at `(32,0)` | region `1,0` (same native-zoom split as `solid-east`) |

Each case records expected relative PNG paths (including the world tile prefix used by the installer, e.g. `world/3/0_0.png`). Hashes live in `testdata/bridge/v2/installer/java-oracle-hashes.json` keyed by `case_id` + path.

### Java oracle

Add `common/src/test/java/.../data/ProductionInstallerOracleTest.java`:

- For each install, load the fixture’s 256 Java-oracle ARGB pixels from the existing chunk-render catalog.
- `Image.setPixel` using the mapping above, `save()`, decode RGBA, SHA-256.
- Assert hashes and path sets equal the committed oracle file.
- Fail if the oracle file is missing or a case is absent.

Do not write the oracle from the test in CI. Generate once during implementation and commit.

### Rust probe

Add `rust/crates/squaremap-server/tests/java_installer_equivalence.rs`:

- Module header comments: proves / does not prove.
- For each catalog case, configure `RenderTileInstaller` with `max_zoom = 3` and relative prefix `world`.
- `stage` + `publish` the v2 render fixture snapshots at the case’s chunk coordinates (same path as `tile_install_parity.rs`; do not read Java PNG bytes as the Rust result).
- Hash decoded RGBA of every published path and compare to the same committed oracle.
- Test names: `installer_places_flat_solid_at_0_0_matching_java_image_rgba`, etc.
- First pixel mismatch is printed on failure (`(x,z) rust=… java=…`).

### Close the silent skip

Commit `testdata/bridge/v2/tiles/java-oracle-hashes.json` from the existing Java `build/tile-oracle/java/report.json` (paths keyed by name, not zip-by-index; Java currently lists zoom `0..3`, Rust probes have used `3..0`).

Change `java_oracle_decoded_rgba_hashes_and_paths_match` to always load that file. Keep the env-var path as an optional extra dump for pixel diffs; absence of the env var must not skip the hash comparison.

`ImagePyramidOracleTest` also asserts it still produces those committed hashes so `Image.java` cannot drift unnoticed.

## Layer 2 — HTTP Java contract table

**Proves:** `HttpServer` matches the documented Java/Undertow URL, status, header, and body-class contract that the web frontend relies on.

**Does not prove:** a live Undertow process, HTTP/2, Paper plugin serving, or that Java still exists on the classpath (`ExclusiveRustOwnershipTest` already asserts it does not).

### Contract file

Create `testdata/bridge/v1/http-java-contract.json`:

```json
{
  "schema_version": 1,
  "source": "Java IntegratedServer / Undertow semantics captured after Java HTTP deletion",
  "cases": [ { "id": "...", "proves": "...", "verdict": "match" | "deliberate", "...": "..." } ]
}
```

Each `match` case has `request` (`method`, `path`, optional headers) and `expect` (`status`, `body`: `empty` | `exact` | `ignored`, optional `content_type` of a string or `absent`, optional `cache_control`, optional `etag`: `quoted` | `absent`).

The Rust test `java_http_contract.rs` loads the file, builds a fixture web root + output root, binds `HttpServer`, and executes every `match` row. Denominator is `cases.length`. A missing id, unused row, or extra undocumented assertion fails.

### Required match rows

| id | Java behavior |
|---|---|
| `index-from-web-root` | `GET /` is 200 HTML from the extracted web directory, not the output root |
| `asset-from-web-root` | `GET /assets/*` from web root with JS/CSS content types |
| `favicon-ico-content-type` | `GET /favicon.ico` is `image/x-icon` |
| `settings-json-cache-and-etag` | `GET /tiles/settings.json` is `application/json`, `Cache-Control: max-age=0, must-revalidate, no-cache`, quoted ETag |
| `settings-json-head` | `HEAD` same headers, empty body |
| `etag-if-none-match-304` | matching `If-None-Match` → 304 empty |
| `etag-star-and-weak-304` | `*` and `W/"etag"` → 304 |
| `existing-tile-200` | existing `tiles/*.png` is 200 |
| `missing-tile-empty-200` | missing `tiles/*.png` is 200 empty, **no** `Content-Type` (Leaflet skip) |
| `registered-icon-from-output` | `/images/icon/registered/*` from output root |
| `static-icon-from-web` | `/images/icon/player.png` from web root |
| `unknown-path-404` | non-tile missing path is 404 |
| `post-not-allowed` | `POST /` is 405 |
| `path-traversal-rejected` | `../`, `%2e%2e`, `%252e%252e`, encoded slash, NUL, backslash are 4xx |
| `disabled-http-does-not-bind` | `http_enabled=false` writes files but binds no listener |
| `hostname-bind-localhost` | bind host `localhost` resolves like Undertow `addHttpListener` |

Existing `http_contract.rs` cases stay as implementation tests. The new file is the **Java-equivalence denominator**. If both would assert the same thing, the new test reads the table rather than copying literals.

### Required deliberate rows (counted, not silent)

| id | Java | Rust | reason |
|---|---|---|---|
| `players-json-cache-control` | `JsonCache` served `players.json` / `markers.json` with no `Cache-Control` | always `max-age=0, must-revalidate, no-cache` | prevent stale player positions |
| `json-post-405` | `JsonCache` answered non-GET/HEAD | 405 | no mutation API |
| `encoded-separator-rejected` | Undertow decoded and could serve | 400/4xx | path confinement |
| `no-h2c` | Undertow could enable h2c | HTTP/1.1 only | not required by the frontend |

A deliberate row must include `java`, `rust`, and `reason`. The test asserts the **Rust** side of each divergence still holds (so a “fix” that accidentally matches Java on a hardening path fails until the table is updated).

## Layer 3 — Production views + HTTP vs frozen Java JSON

**Proves:** production `views::apply_replacement` / `write_players` / marker and icon writers, served by `HttpServer`, return bodies that semantically equal `testdata/bridge/v1/fixtures/java/*`.

**Does not prove:** bridge framing, tokens, Paper events, dirty/render, or that Java exporters still emit those fixtures (Java `StateExporterTest` remains the exporter oracle).

### Inputs

Reuse the existing Java fixtures as the expected HTTP bodies:

- `settings.json`
- `world-settings.json`
- `players.json`
- `markers.json`
- `icons.json`

Build the Rust side from the same logical records the Java exporters tested (`StateExporterTest` public player, two worlds, marker geometry set, spawn icon). Do not copy Java fixture bytes into the output root and serve them; that would prove the static file server, not the writers.

### Probe

Add `rust/crates/squaremap-server/tests/java_view_http_equivalence.rs`:

1. Construct replacement envelopes matching the fixture records (worlds `minecraft_overworld` / `minecraft_nether`, public player, marker set, 2×1 rgba icon).
2. `apply_replacement` into a fresh `OutputRoot`.
3. Bind `HttpServer` with that root.
4. `GET /tiles/settings.json`, `/tiles/players.json`, `/tiles/minecraft_overworld/settings.json`, `/tiles/minecraft_overworld/markers.json`, and `GET /images/icon/registered/spawn.png` (icons are files under the output root, not `tiles/icons.json`).
5. Semantic JSON compare to `testdata/bridge/v1/fixtures/java/{settings,players,world-settings,markers}.json`. Icon: decoded PNG RGBA vs the spawn payload the Java fixture describes.
6. Fail on missing path, extra required path, or JSON inequality.

Use `squaremap-compare`’s JSON comparator (`run_manifest` / `OutputType::Json`) rather than a third equality helper.

`squaremap-compare parity` against the pre-written `fixtures/java` vs `fixtures/rust` directories remains a fixture-sanity check; Layer 3 is the production-path proof.

## File map

| Path | Role |
|---|---|
| `testdata/bridge/v2/installer/manifest.json` | chunk-placement catalog |
| `testdata/bridge/v2/installer/java-oracle-hashes.json` | committed Java RGBA hashes |
| `testdata/bridge/v2/tiles/java-oracle-hashes.json` | committed pyramid hashes (closes silent skip) |
| `testdata/bridge/v1/http-java-contract.json` | HTTP match + deliberate rows |
| `common/.../data/ProductionInstallerOracleTest.java` | Java installer oracle |
| `common/.../data/ImagePyramidOracleTest.java` | also pin committed pyramid hashes |
| `rust/.../tests/java_installer_equivalence.rs` | Layer 1 |
| `rust/.../tests/java_http_contract.rs` | Layer 2 |
| `rust/.../tests/java_view_http_equivalence.rs` | Layer 3 |
| `rust/.../tests/tile_pyramid_parity.rs` | always compare committed hashes |
| `docs/living-specs/java-rust-equivalence.md` | domain catalog for this suite |
| `docs/living-specs/java-rust-tiling-parity.md` | installer item moves Current when Layer 1 passes |

Do not add a fourth catch-all `parity` test. Do not spawn Paper. Do not restore `IntegratedServer`.

## Error handling

- Missing testdata file: fail with the path.
- Hash mismatch: fail with case id, relative path, both hashes, first RGBA pixel coordinate.
- HTTP table: fail with case id, expected vs actual status/header.
- JSON: fail with the field path from the existing comparator.
- Deliberate row whose Rust assertion no longer holds: fail (table is stale or behavior regressed).

## Test execution (CI)

```bash
./gradlew :squaremap-common:test --tests '*ProductionInstallerOracleTest' --tests '*ImagePyramidOracleTest' --tests '*StateExporterTest'
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test java_installer_equivalence --test java_http_contract --test java_view_http_equivalence
cargo test --manifest-path rust/Cargo.toml -p squaremap-render --test tile_pyramid_parity
```

All three layers run in default CI. No env vars required.

## Non-goals

- Live Paper tile-tree comparison (living-spec Future)
- Reintroducing Java HTTP or `LegacyBackendController`
- Compressed PNG byte equality
- Matching Java overwrite of corrupt tiles
- Performance / RSS / canary / cutover
- Full bridge handshake against a real sidecar stdin token (Layer 3 is in-process `HttpServer` + view writers)

## Success

The suite proves “fundamentally the same as Java” for CI as:

- installer pyramids match frozen Java `Image` RGBA hashes for every installer catalog case
- every HTTP contract row is executed; match rows equal Java semantics; deliberate rows stay named
- production JSON/icon HTTP bodies equal frozen Java fixtures

Anything else stays explicitly unproven.
}
