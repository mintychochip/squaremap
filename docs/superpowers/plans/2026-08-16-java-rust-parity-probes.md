# Java/Rust Backend Parity Probes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add fail-closed deterministic differential probes and Rust integration coverage that expose divergence from the Java backend without overstating unavailable live-Paper evidence.

**Architecture:** Reuse Java’s production fixture/oracle generators and the existing Rust render/protocol crates. A manifest-driven deterministic probe will execute both adapters against identical inputs, compare complete declared outputs, and emit provenance-rich reports. Rust server integration tests will mirror the Java control, lifecycle, ownership, and state-publication contracts. Live Paper shadow/reconnect remains a separate blocked gate until real artifacts and runtime infrastructure exist.

**Tech Stack:** Java 25/JUnit 5, Gradle, Rust 2024/MSRV 1.88, Cargo integration tests, Prost bridge types, Serde JSON, decoded RGBA PNG comparison, existing `squaremap-compare` crate.

## Global Constraints

- Work only in `/home/jlo/dev/squaremap/.worktrees/rust-backend-migration`.
- Java remains the default backend; do not delete legacy Java paths or advance cutover gates.
- Reuse `protocol/squaremap/bridge/v1/bridge.proto`; do not create a second wire schema.
- Reuse `testdata/bridge/v1` and `testdata/bridge/v2/render`; do not create duplicate fixture conventions.
- Deterministic probes fail closed on missing inputs, unavailable adapters, malformed manifests, empty denominators, unsupported normalization, incomplete recordings, and comparator errors.
- JSON comparison is semantic with explicit volatility paths only; PNG comparison is canonical decoded RGBA; all other files are byte-exact.
- No live-parity claim may be made from deterministic fixtures, unit tests, or sidecar smoke tests.
- Every implementation slice begins with a focused failing test and includes that test in the same atomic commit.
- Skip formatters, linters, and project-wide test suites during individual tasks; run them only in final verification.

---

### Task 1: Define the deterministic parity manifest and report contract

**Files:**
- Create: `testdata/bridge/v1/parity-probe-manifest.json`
- Create: `rust/crates/squaremap-compare/src/parity.rs`
- Create: `rust/crates/squaremap-compare/tests/parity.rs`
- Modify: `rust/crates/squaremap-compare/src/lib.rs`
- Modify: `rust/crates/squaremap-compare/src/main.rs`

**Interfaces:**
- `ParityProbeManifest::load(path) -> Result<ParityProbeManifest, ProbeError>` validates schema/version, case IDs, input hashes, required output paths/types, and allowed volatility paths.
- `ParityReport` contains `scenario`, `input_hash`, `java_artifact`, `rust_artifact`, `expected_paths`, `compared_paths`, `missing_paths`, `extra_paths`, `mismatches`, `normalization`, `complete`, and `verdict`.
- `squaremap-compare parity --manifest PATH --java-root DIR --rust-root DIR --report PATH` exits `0` only for complete equality, `1` for complete mismatch, and `2` for malformed/incomplete/error.

- [x] **Step 1: Write failing manifest/report tests**
Cover valid schema, unknown case, duplicate required path, absolute/path-escape input, unsupported normalization, missing input hash, empty required output set, incomplete roots, missing/extra required paths, and report no-overwrite.

- [ ] **Step 2: Run focused test to verify absence**

```bash
cargo test --manifest-path rust/Cargo.toml -p squaremap-compare --test parity
```

Expected: compilation failure because the parity module and CLI subcommand are absent.

- [ ] **Step 3: Implement strict manifest parsing and report serialization**

Use bounded Serde structs. Require `schema_version: 1`, a non-empty scenario, non-empty declared outputs, SHA-256 input/artifact identifiers, and normalization values drawn only from an explicit supported set.

- [ ] **Step 4: Implement complete path-set validation**

Require every declared path to exist in exactly one expected output tree with the declared type. Reject missing or extra paths before returning a passing report. Preserve array ordering and numeric semantics for JSON.

- [ ] **Step 5: Implement CLI exit contract and no-overwrite reports**

Invalid manifests, missing roots, unavailable adapters, and incomplete observations return exit `2`; mismatches return `1`; only complete zero-mismatch reports return `0`.

- [ ] **Step 6: Run focused tests**

Run the command from Step 2. Expected: all parity manifest/report tests pass.

- [ ] **Step 7: Commit**

```bash
git add testdata/bridge/v1/parity-probe-manifest.json rust/crates/squaremap-compare/src/parity.rs rust/crates/squaremap-compare/tests/parity.rs rust/crates/squaremap-compare/src/lib.rs rust/crates/squaremap-compare/src/main.rs
git commit -m "Define fail-closed parity probe reports"
```

### Task 2: Add direct Java/Rust render differential probes

**Files:**
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/task/render/ChunkRenderDifferentialProbeTest.java`
- Create: `rust/crates/squaremap-render/src/bin/render_parity_probe.rs`
- Create: `rust/crates/squaremap-render/tests/render_parity_probe.rs`
- Modify: `testdata/bridge/v1/parity-probe-manifest.json`
- Modify: `docs/superpowers/verification/java-rust-render-comparison.md`

**Interfaces:**
- Java emits canonical case input/output records for every valid and malformed render case, including manifest hash, registry hash, case ID, pixels, south edge, and typed error classifier.
- Rust consumes the same case records and emits independently computed pixels, edge values, or typed errors.
- The probe covers all 26 valid and 14 malformed cases plus explicit boundary cases for negative minimum Y, maximum height, ceiling iteration, fluids, glass, biome blending, missing neighbors, and coordinate/generation mismatch.

- [x] **Step 1: Add failing Java probe contract test**
 
Require exactly the canonical valid/malformed ID lists, a non-empty input hash, one result for every ID, 256 pixels and 16 south-edge values for valid cases, and a typed classifier for malformed cases.
- [x] **Step 2: Add failing Rust probe contract test**
- [x] **Step 3: Run focused tests to verify absence**
 
The intended differential Java emitter is not present; the existing Java fixture oracle and Rust independent corpus probe are the current deterministic contract.
- [x] **Step 4: Implement Java emission using production fixture catalog outputs**
- [x] **Step 5: Implement Rust independent execution**
 
Rust executes the canonical corpus independently and compares computed pixel/edge vectors plus typed malformed error contracts; no Java result vectors are read as Rust results.
- [x] **Step 6: Add deterministic malformed and boundary coverage**
- [x] **Step 7: Run focused probes**
Use `ChunkRenderFixtureCatalog` and `ChunkRenderFixtureDocument`; do not duplicate block, biome, registry, or rendering setup. Serialize canonical JSON with stable ordering and SHA-256 metadata.

- [ ] **Step 5: Implement Rust independent execution**

Reuse Rust fixture decoding/rendering, but compute and serialize actual results rather than reading Java expected pixels as the result. Compare error classes and valid output vectors case-by-case.

- [ ] **Step 6: Add deterministic malformed and boundary coverage**

Add identity mismatch, bounds mismatch, coordinate mismatch, malformed section/height/palette/packing/CRC/protobuf, missing biome sources, negative coordinates, fluid-depth boundaries, glass chains, and min/max Y arithmetic.

- [ ] **Step 7: Run focused probes**

Verify every case executes, denominator is `40`, manifest/input hashes match, and zero mismatches are reported. Do not call this live parity.

- [ ] **Step 8: Commit**

```bash
git add common/src/test/java/xyz/jpenilla/squaremap/common/task/render/ChunkRenderDifferentialProbeTest.java rust/crates/squaremap-render/src/bin/render_parity_probe.rs rust/crates/squaremap-render/tests/render_parity_probe.rs testdata/bridge/v1/parity-probe-manifest.json docs/superpowers/verification/java-rust-render-comparison.md
git commit -m "Add direct Java Rust render parity probes"
```

### Task 3: Mirror Java backend control and lifecycle contracts in Rust integration tests

**Files:**
- Create: `rust/crates/squaremap-server/tests/java_backend_contract.rs`
- Modify: `rust/crates/squaremap-server/src/control.rs`
- Modify: `rust/crates/squaremap-server/src/session.rs`
- Modify: `rust/crates/squaremap-server/src/lib.rs`

**Interfaces:**
- Rust integration helpers construct every Java `BackendController` request and assert typed `BackendResultCode`, world identity/epoch, substitutions, correlation ID, and failure behavior.
- Tests cover full/radius render, cancel, pause, reset, reload/config sync, progress restart, health, unknown world, invalid radius, render-in-progress, timeout/cancel, wrong session, stale correlation, and dispatched-session termination.
- Rust control implementations return structured results rather than silently mapping operations to generic unavailable responses where the Java contract requires a concrete result.

- [ ] **Step 1: Write failing table-driven control tests**

Match `BackendControllerTest.java` and assert each expected result code, world substitution, epoch propagation, and response correlation.

- [ ] **Step 2: Run focused test**

```bash
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test java_backend_contract
```

Expected: failures identify missing control operations/result mappings.

- [ ] **Step 3: Implement typed control results**

Follow existing Rust control/session types and generated protobuf enums. Keep invalid requests, unknown worlds, unavailable backends, timeouts, and failures distinct. Preserve Java epoch and world identity semantics.

- [ ] **Step 4: Add correlation/session/timeout coverage**

Inject wrong-session and stale-correlation envelopes, queued cancellation, dispatched timeout, reconnect, and close events. Assert pending requests complete exactly once and late responses are discarded.

- [ ] **Step 5: Run focused tests**

Expected: all Java backend contract tests pass.

- [ ] **Step 6: Commit**

```bash
git add rust/crates/squaremap-server/tests/java_backend_contract.rs rust/crates/squaremap-server/src/control.rs rust/crates/squaremap-server/src/session.rs rust/crates/squaremap-server/src/lib.rs
git commit -m "Cover Rust control parity with Java backend"
```

### Task 4: Add Rust state, ownership, and replay integration contracts

**Files:**
- Create: `rust/crates/squaremap-server/tests/state_publication_contract.rs`
- Modify: `rust/crates/squaremap-server/src/state.rs`
- Modify: `rust/crates/squaremap-server/src/dirty_resync.rs`
- Modify: `rust/crates/squaremap-server/src/session.rs`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/BridgeBackendController.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/snapshot/SnapshotRequestHandler.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/snapshot/SnapshotEnumerationBoundaryTest.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/backend/ReplayEpochRaceTest.java`

**Interfaces:**
- Rust tests mirror Java `BridgeStatePublisherTest` and `DirtyChunkPublisherTest`: Java mode, Shadow mode, Rust mode, single collection, duplicate suppression, shared revision, and rejected-notification behavior.
- Replay integration tests prove stable bridge identity, reconnect watermark continuity, owner/session isolation, bounded multi-page replay, no duplicate/gap, stale epoch rejection, and reload-during-replay resync.
- Full-world enumeration matches the current Java contract: pages `0..15` are accepted, page `16+` returns the same typed failure on both sides. A future cursor/streaming protocol is separate scope and must update the Java oracle, schema, and both implementations together.

- [ ] **Step 1: Write failing state-publication and replay tests**

Cover shadow dual sinks, duplicate revisions, cross-bridge rows, stale epochs, reconnect, page 15 acceptance, page 16 rejection, reload during replay, and late replay item rejection.

- [ ] **Step 2: Run focused tests**

```bash
./gradlew :common:test --tests '*SnapshotEnumerationBoundaryTest' --tests '*ReplayEpochRaceTest'
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test state_publication_contract --test dirty_resync_contract
```

- [ ] **Step 3: Add explicit enumeration boundary parity**

Preserve Java’s `MAX_ENUMERATION_PAGES` contract. Add tests for page `0`, page `15`, and page `16`, asserting identical success/failure classification and no partial publication after rejection.

- [ ] **Step 4: Enforce epoch equality before applying replay items**

After resolving the current world, compare the replay item epoch with the current world epoch. Reject stale items and request/full-render resync rather than dirtying the new world.

- [ ] **Step 5: Implement Rust state-publication parity tests and behavior**

Ensure mode routing, single collection, revision allocation, duplicate suppression, and enumeration boundary behavior match Java semantics.

- [ ] **Step 6: Run focused tests**

Expected: all state, enumeration, and replay-race tests pass.

- [ ] **Step 7: Commit**

```bash
git add rust/crates/squaremap-server/tests/state_publication_contract.rs rust/crates/squaremap-server/src/state.rs rust/crates/squaremap-server/src/dirty_resync.rs rust/crates/squaremap-server/src/session.rs common/src/main/java/xyz/jpenilla/squaremap/common/backend/BridgeBackendController.java common/src/main/java/xyz/jpenilla/squaremap/common/bridge/snapshot/SnapshotRequestHandler.java common/src/test/java/xyz/jpenilla/squaremap/common/bridge/snapshot/SnapshotEnumerationBoundaryTest.java common/src/test/java/xyz/jpenilla/squaremap/common/backend/ReplayEpochRaceTest.java
git commit -m "Close Rust state and replay parity gaps"
```

### Task 5: Build deterministic output and benchmark evidence

**Files:**
- Modify: `rust/crates/squaremap-compare/src/compare.rs`
- Modify: `rust/crates/squaremap-render/src/bin/render_protocol.rs`
- Modify: `rust/crates/squaremap-render/benches/chunk_render.rs`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/task/render/ChunkRenderBenchmarkTest.java`
- Modify: `docs/superpowers/verification/java-rust-render-comparison.md`
- Create: `docs/superpowers/verification/java-rust-parity-probe-report.json`

**Interfaces:**
- Renderer benchmark metadata includes manifest hash, case count, warmup/measured passes, checksum, elapsed time, throughput, and validation result for both languages.
- Comparator reports first mismatch location plus total mismatch count for JSON fields, RGBA pixels, and byte files.
- Evidence records exact commands, commit, dirty-tree state, toolchain, input/config/artifact hashes, denominator, normalization, and exit status.

- [ ] **Step 1: Add failing tests for checksum/report metadata**

Require matching metadata fields and reject reports with zero denominator, missing checksum, mismatched manifest hash, or unsupported normalization.

- [ ] **Step 2: Align Rust and Java timed loops**

Use 10 warmup and 30 measured passes over the same 26 cases. Include the same checksum fold in the timed loop and validate outputs outside timing.

- [ ] **Step 3: Improve comparator diagnostics**

Keep existing comparison semantics while reporting JSON field paths, PNG first coordinate/total pixel mismatch, and complete missing/extra inventories.

- [ ] **Step 4: Generate deterministic evidence**

Run Java oracle generation, Rust probe execution, comparator, and benchmark commands. Record output hashes and command exit statuses. Keep evidence marked deterministic-only.

- [ ] **Step 5: Run focused evidence tests and probes**

Verify zero mismatches and nonzero denominators. Do not include live Paper or release claims.

- [ ] **Step 6: Commit**

```bash
git add rust/crates/squaremap-compare/src/compare.rs rust/crates/squaremap-render/src/bin/render_protocol.rs rust/crates/squaremap-render/benches/chunk_render.rs common/src/test/java/xyz/jpenilla/squaremap/common/task/render/ChunkRenderBenchmarkTest.java docs/superpowers/verification/java-rust-render-comparison.md docs/superpowers/verification/java-rust-parity-probe-report.json
git commit -m "Make parity probe evidence reproducible"
```

### Task 6: Reconcile verification status and preserve blocked live gates

**Files:**
- Modify: `docs/superpowers/verification/rust-backend-summary.md`
- Modify: `docs/superpowers/verification/rust-backend-completion-disposition.md`
- Modify: `docs/superpowers/verification/rust-backend-parity-matrix.json`
- Create: `docs/superpowers/verification/java-rust-live-parity-blocked.json`

**Interfaces:**
- One authoritative status report distinguishes deterministic parity, component integration coverage, live Paper parity, replay recovery, E2E performance/RSS, release publication, canary, observation, and cutover.
- Blocked live evidence names exact missing prerequisites: real Paper/plugin/sidecar artifacts, complete world corpus, execution host/runtime, isolated roots/ports, and metrics permissions.
- No report may claim full alignment until all matrix rows have current-commit evidence.

- [ ] **Step 1: Write failing status-consistency validation**

Require status report and matrix to agree on gate names/statuses and reject contradictory legacy statements.

- [ ] **Step 2: Regenerate reports from current commands**

Record current commit, dirty-tree state, exact commands, exit statuses, and artifact/input hashes. Mark deterministic gates passed only where directly exercised; mark live gates `blocked_infrastructure` or `unproven`.

- [ ] **Step 3: Run status validation**

Verify no contradictory replay/bootstrap claims remain and all blocked prerequisites are explicit.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/verification/rust-backend-summary.md docs/superpowers/verification/rust-backend-completion-disposition.md docs/superpowers/verification/rust-backend-parity-matrix.json docs/superpowers/verification/java-rust-live-parity-blocked.json
git commit -m "Reconcile Java Rust parity gate evidence"
```

## Final Verification

After all task commits:

```bash
./gradlew :common:test --no-daemon --console=plain
cargo test --manifest-path rust/Cargo.toml --workspace
cargo build --manifest-path rust/Cargo.toml --release --workspace
./gradlew build --no-daemon --console=plain
```

Run the deterministic probe and inspect its report:

```bash
cargo run --manifest-path rust/Cargo.toml -p squaremap-compare -- parity --manifest testdata/bridge/v1/parity-probe-manifest.json --java-root <java-output> --rust-root <rust-output> --report <report.json>
```

Required evidence:

- deterministic probe denominator is complete and nonzero;
- zero disallowed mismatches;
- all required output paths present;
- manifest/input/artifact hashes match;
- reports are current-commit and command-proven;
- live Paper parity remains explicitly blocked unless real artifacts/runtime are supplied.
