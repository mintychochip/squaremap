# Java/Rust Equivalence Probes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add three fail-closed CI probe layers that prove the Rust backend matches leftover Java oracles on production tile installation, HTTP request semantics, and JSON/icon HTTP bodies.

**Architecture:** Frozen testdata oracles (RGBA hashes, JSON fixtures, an HTTP contract table) are the comparison target. Gradle verifies `Image.java` still produces the hashes. Cargo verifies `RenderTileInstaller`, `HttpServer`, and `views::apply_replacement` still match them. No Paper, no env-var skips, no restored Java HTTP.

**Tech Stack:** Java 25/JUnit 5, Gradle, Rust 2024, Tokio, Axum, Prost envelopes, SHA-256 of decoded RGBA, `squaremap-compare` semantic JSON.

**Spec:** `docs/superpowers/specs/2026-08-23-java-rust-equivalence-probes.md`

---

## File map

| Path | Responsibility |
|---|---|
| `testdata/bridge/v2/tiles/java-oracle-hashes.json` | Frozen Java `Image.save()` RGBA hashes for the 10 tile catalog cases, keyed by case id + relative path |
| `testdata/bridge/v2/installer/manifest.json` | Chunk-placement catalog (fixture id + chunk coords) |
| `testdata/bridge/v2/installer/java-oracle-hashes.json` | Frozen Java installer-path RGBA hashes |
| `testdata/bridge/v1/http-java-contract.json` | HTTP match + deliberate rows |
| `common/src/test/java/xyz/jpenilla/squaremap/common/data/ImagePyramidOracleTest.java` | Also pin committed pyramid hashes |
| `common/src/test/java/xyz/jpenilla/squaremap/common/data/ProductionInstallerOracleTest.java` | Java installer oracle |
| `rust/crates/squaremap-render/tests/tile_pyramid_parity.rs` | Always compare committed hashes |
| `rust/crates/squaremap-server/tests/java_installer_equivalence.rs` | Layer 1 |
| `rust/crates/squaremap-server/tests/java_http_contract.rs` | Layer 2 |
| `rust/crates/squaremap-server/tests/java_view_http_equivalence.rs` | Layer 3 |
| `rust/crates/squaremap-server/Cargo.toml` | Dev-dep on `squaremap-compare` |
| `docs/living-specs/java-rust-equivalence.md` | Checkbox progress |
| `docs/living-specs/java-rust-tiling-parity.md` | Move installer item when Layer 1 passes |

Do not restore `IntegratedServer`. Do not spawn Paper. Do not compare compressed PNG bytes.

Pixel index on both sides: `pixels[x * 16 + z]`. Mapping: `region = floorDiv(chunk, 32)`, `local = floorMod(chunk, 32) * 16`. Hash file paths are zoom-relative (`3/0_0.png`), not prefixed with `world/`.

---

### Task 1: Commit pyramid hashes and fail-close the silent skip

**Files:**
- Create: `testdata/bridge/v2/tiles/java-oracle-hashes.json`
- Modify: `rust/crates/squaremap-render/tests/tile_pyramid_parity.rs`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/data/ImagePyramidOracleTest.java`

- [x] **Step 1: Write the committed hash file** from `build/tile-oracle/java/report.json`, keyed by case id then relative path (do not zip by index). Schema:

```json
{
  "schema_version": 1,
  "manifest_hash": "<sha256 of testdata/bridge/v2/tiles/manifest.json>",
  "max_zoom": 3,
  "cases": {
    "solid-origin": {
      "3/0_0.png": "<sha256>",
      "2/0_0.png": "<sha256>",
      "1/0_0.png": "<sha256>",
      "0/0_0.png": "<sha256>"
    }
  }
}
```

Copy hashes from the existing Java `report.json` so each path name maps to its hash (Java lists `0/..` then `3/..`; that order must not be used as an array zip).

- [ ] **Step 2: Write the failing Rust change** — replace the env-var early return in `java_oracle_decoded_rgba_hashes_and_paths_match` with a load of `testdata/bridge/v2/tiles/java-oracle-hashes.json`. Keep `SQUAREMAP_JAVA_TILE_ORACLE` only as an optional extra dump for first-pixel diffs. If the committed file is missing, panic with the path.

Helper:

```rust
const JAVA_HASHES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../testdata/bridge/v2/tiles/java-oracle-hashes.json"
);

#[derive(Deserialize)]
struct JavaOracleHashes {
    schema_version: u32,
    manifest_hash: String,
    max_zoom: u8,
    cases: BTreeMap<String, BTreeMap<String, String>>,
}
```

Compare rust probe hashes to `cases[id][path]`. Fail on missing case, missing path, extra path, or hash mismatch. Print first pixel mismatch when optional RGBA dumps exist.

- [ ] **Step 3: Run the focused Rust test**

```bash
cargo test --manifest-path rust/Cargo.toml -p squaremap-render --test tile_pyramid_parity java_oracle_decoded_rgba_hashes_and_paths_match -- --nocapture
```

Expected: fail until the hash file exists and path-keyed comparison is wired; then pass with zero mismatches.

- [ ] **Step 4: Pin the same file from Java** in `ImagePyramidOracleTest` after generating `report`: for every case/path, `assertEquals(committedHash, sha256Hex(rgba), id + " " + relative)`. Fail if the committed file is missing.

- [ ] **Step 5: Run Java oracle test**

```bash
./gradlew :squaremap-common:test --tests '*ImagePyramidOracleTest' --no-daemon --console=plain
```

Expected: BUILD SUCCESSFUL.

---

### Task 2: Installer catalog + Java `Image` oracle + committed installer hashes

**Files:**
- Create: `testdata/bridge/v2/installer/manifest.json`
- Create: `testdata/bridge/v2/installer/java-oracle-hashes.json`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/data/ProductionInstallerOracleTest.java`

- [ ] **Step 1: Write the installer catalog**

```json
{
  "schema_version": 1,
  "max_zoom": 3,
  "render_manifest": "testdata/bridge/v2/render/manifest.json",
  "cases": [
    {"id": "fixture-flat-solid-at-0-0", "installs": [{"fixture": "flat-solid", "chunk": [0, 0]}]},
    {"id": "fixture-flat-solid-at-1-0", "installs": [{"fixture": "flat-solid", "chunk": [1, 0]}]},
    {"id": "fixture-flat-solid-then-iterate-down", "installs": [
      {"fixture": "flat-solid", "chunk": [0, 0]},
      {"fixture": "iterate-down", "chunk": [1, 0]}
    ]},
    {"id": "fixture-flat-solid-at-31-31", "installs": [{"fixture": "flat-solid", "chunk": [31, 31]}]},
    {"id": "fixture-flat-solid-at-negative", "installs": [{"fixture": "flat-solid", "chunk": [-1, -1]}]},
    {"id": "fixture-flat-solid-at-32-0", "installs": [{"fixture": "flat-solid", "chunk": [32, 0]}]}
  ]
}
```

- [ ] **Step 2: Write the failing Java oracle test** that loads the catalog, paints each install via `Image.setPixel` using pixels from the render manifest (`pixels[x * 16 + z]` at `local = Math.floorMod(chunk, 32) * 16`), `save()`, hashes decoded RGBA, and compares to `java-oracle-hashes.json`. Missing hash file fails closed. Pixel index and mapping as in the spec. One `Image` per region; sequential installs in the same region must `save()` in order so later chunks merge via `getOrCreate`.

Test names:

- `oraclePinsCommittedHashesForEveryInstallerCase`
- `chunk00NativeOriginMatchesFlatSolidPixel00`
- `chunk10SitsAtRegionLocal16_0`
- `chunk3131SitsAtRegionLocal496_496`
- `negativeChunkUsesNegativeZoomPaths`
- `chunk320UsesEastNativeTile`

- [ ] **Step 3: Run the Java test and watch it fail** (missing hashes or missing class).

```bash
./gradlew :squaremap-common:test --tests '*ProductionInstallerOracleTest' --no-daemon --console=plain
```

- [ ] **Step 4: Generate and commit installer hashes** by running the same paint/save/hash logic once (test can write the generated map to stdout on mismatch). Save `testdata/bridge/v2/installer/java-oracle-hashes.json` with the same schema as the pyramid hash file.

- [ ] **Step 5: Re-run Java tests until BUILD SUCCESSFUL.**

---

### Task 3: Rust production installer vs committed Java hashes

**Files:**
- Create: `rust/crates/squaremap-server/tests/java_installer_equivalence.rs`
- Reuse helpers from `rust/crates/squaremap-server/tests/tile_install_parity.rs` (corpus load, snapshot bundle, `RenderTileInstaller`).

Module header:

```rust
//! Proves: RenderTileInstaller maps chunk coordinates with floorMod(chunk, 32)*16
//! and publishes a pyramid whose decoded RGBA matches Java Image.setPixel+save
//! for the same v2 render-fixture pixels.
//! Does not prove: live Minecraft snapshots, HTTP, scheduler jobs, Paper.
```

- [ ] **Step 1: Write failing tests** named:

- `installer_places_flat_solid_at_0_0_matching_java_image_rgba`
- `installer_places_flat_solid_at_1_0_matching_java_image_rgba`
- `installer_places_flat_solid_then_iterate_down_matching_java_image_rgba`
- `installer_places_flat_solid_at_31_31_matching_java_image_rgba`
- `installer_places_flat_solid_at_negative_matching_java_image_rgba`
- `installer_places_flat_solid_at_32_0_matching_java_image_rgba`
- `installer_catalog_denominator_is_six_cases`

Each install test: `RenderTileInstaller` with `max_zoom = 3`, prefix `world`, `stage`+`publish` fixture snapshots at the catalog chunk coords, hash decoded RGBA of `world/{zoom}/{x}_{z}.png` after stripping the `world/` prefix, compare to committed hashes. Fail with first pixel `(x,z)` on mismatch. Do not read Java PNG bytes as the Rust result.

- [ ] **Step 2: Run**

```bash
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test java_installer_equivalence -- --nocapture
```

Expected: compile or assertion failure until the installer path matches.

- [ ] **Step 3: Minimal implementation** — only test code plus existing `RenderTileInstaller`. No production mapping change unless a real mismatch is found; if hashes mismatch, print the first pixel and stop (do not “fix” Java).

- [ ] **Step 4: Re-run until all installer tests pass.**

---

### Task 4: HTTP Java contract table + runner

**Files:**
- Create: `testdata/bridge/v1/http-java-contract.json`
- Create: `rust/crates/squaremap-server/tests/java_http_contract.rs`

- [ ] **Step 1: Write the contract JSON** with `schema_version: 1` and every required id from the spec (16 match + 4 deliberate). Each `match` row has `id`, `proves`, `verdict: "match"`, `request` (`method`, `path`, optional `headers`), `expect` (`status`, `body`: `empty` | `exact` | `ignored`, optional `exact_body`, optional `content_type` string or `"absent"`, optional `cache_control`, optional `etag`: `quoted` | `absent`).

`path-traversal-rejected` may use `request.paths: [...]` instead of a single path. `etag-star-and-weak-304` may use `request.if_none_match_variants`.

Each deliberate row has `verdict: "deliberate"`, `java`, `rust`, `reason`, plus `request`/`expect` for the **Rust** side that must still hold.

- [ ] **Step 2: Write the failing runner** `java_http_contract.rs`:

- Header comments: proves / does not prove.
- Load the JSON. Fail if schema ≠ 1 or `cases` is empty.
- Assert the set of ids equals the spec’s required set (hard-code the expected ids in the test as the denominator).
- Build web root (`index.html` = `web-index`, `assets/app.js`, `favicon.ico`, `images/icon/player.png`) and output root (`index.html` = `output-index`, `tiles/settings.json`, `tiles/existing.png`, `tiles/players.json`, `images/icon/registered/spawn.png`).
- Bind `HttpServer` with `web_root`.
- Execute every `match` and `deliberate` row. Fail with case id on status/header/body mismatch.
- `disabled-http-does-not-bind`: `enabled: false`, `local_addr()` is None, `atomic_write` still works.
- `hostname-bind-localhost`: `lookup_host("localhost:0")`, bind that `SocketAddr`, GET `/` is 200.
- `no-h2c`: GET `/` version is HTTP/1.1 and no `upgrade: h2c`.

- [ ] **Step 3: Run**

```bash
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test java_http_contract -- --nocapture
```

Expected: fail until the table and runner exist; then pass against current `HttpServer`.

- [ ] **Step 4: Do not weaken production HTTP to make a row pass.** If a required match fails, fix `HttpServer` only when it contradicts documented Java semantics in the spec. Deliberate rows assert the Rust hardening.

---

### Task 5: Production views + HTTP vs frozen Java JSON

**Files:**
- Modify: `rust/crates/squaremap-server/Cargo.toml` (dev-dependency `squaremap-compare`)
- Create: `rust/crates/squaremap-server/tests/java_view_http_equivalence.rs`

- [ ] **Step 1: Write the failing test** with header proves / does not prove.

Construct envelopes matching `StateExporterTest.checkedInGoldenDocumentsUseProductionExporterAndSinkCores` / `view_contract.rs` (two worlds, public player, `all` marker layer timestamp 42, spawn 2×1 rgba `[255,0,0,255,0,0,255,255]`). Call `apply_replacement` (and `write_players` if players go through that path). Bind `HttpServer`. GET:

- `/tiles/settings.json` vs `testdata/bridge/v1/fixtures/java/settings.json`
- `/tiles/players.json` vs `.../players.json`
- `/tiles/minecraft_overworld/settings.json` vs `.../world-settings.json`
- `/tiles/minecraft_overworld/markers.json` vs `.../markers.json`
- `/images/icon/registered/spawn.png` decoded RGBA vs `[255,0,0,255, 0,0,255,255]`

Compare JSON with `squaremap_compare::parity::run_manifest` (write expected/actual into temp roots, manifest type `json`). Do not copy Java fixture bytes into the output root.

Test names:

- `production_settings_http_matches_java_fixture`
- `production_players_http_matches_java_fixture`
- `production_world_settings_http_matches_java_fixture`
- `production_markers_http_matches_java_fixture`
- `production_spawn_icon_http_matches_java_rgba`

- [ ] **Step 2: Run and watch fail** (missing test).

```bash
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test java_view_http_equivalence -- --nocapture
```

- [ ] **Step 3: Implement using existing `views::apply_replacement` and `HttpServer` only.** If JSON field order or optional-field omission differs, fix the writer only when it is a real semantic mismatch with the Java fixture. Do not rewrite fixtures to match Rust.

- [ ] **Step 4: Re-run until all five tests pass.**

---

### Task 6: Living spec checkboxes and CI command evidence

**Files:**
- Modify: `docs/living-specs/java-rust-equivalence.md`
- Modify: `docs/living-specs/java-rust-tiling-parity.md`

- [ ] **Step 1: Re-run the spec’s CI commands** (fresh, full):

```bash
./gradlew :squaremap-common:test --tests '*ProductionInstallerOracleTest' --tests '*ImagePyramidOracleTest' --tests '*StateExporterTest' --no-daemon --console=plain
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test java_installer_equivalence --test java_http_contract --test java_view_http_equivalence
cargo test --manifest-path rust/Cargo.toml -p squaremap-render --test tile_pyramid_parity
```

- [ ] **Step 2: Only after those pass, check off Layer 1–3 Current items** and move the tiling-parity installer item to checked Current. Update `Last updated` to 2026-08-23.

---

## Self-review vs spec

| Spec requirement | Task |
|---|---|
| Installer mapping + hashes | 2, 3 |
| Close pyramid silent skip | 1 |
| HTTP table match + deliberate | 4 |
| Views + HTTP vs java fixtures | 5 |
| Fail closed / no env skip | 1–5 |
| No Paper / no IntegratedServer | all |
| Living spec checkboxes after evidence | 6 |
