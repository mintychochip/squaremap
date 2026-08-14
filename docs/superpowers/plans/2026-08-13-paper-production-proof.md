# Paper Production Proof Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and execute a fail-closed local Paper lifecycle gate proving Rust HTTP/output ownership, tile parity under real block changes, lifecycle/recovery convergence, paired performance/RSS behavior, and zero output mismatches.

**Architecture:** Add a transparent recorder to the existing Java bridge, drive a pinned Paper process from a black-box verification harness, and feed the resulting recordings and output trees into the existing `squaremap-compare` crate. Run isolated Java and Rust twins, require strict quiescence at lifecycle barriers, and accept only a complete hash-addressed evidence bundle.

**Tech Stack:** Java 25, Gradle Kotlin DSL, Paper/run-paper, JUnit 5, Rust 2024/MSRV 1.88, Tokio, Prost, Serde, PNG, SHA-256, Linux `/proc` process metrics.

## Global Constraints

- Work only in `/home/jlo/dev/squaremap/.worktrees/rust-backend-migration`.
- Treat `docs/superpowers/specs/2026-08-13-paper-production-proof-design.md` as authoritative.
- Reuse the existing Java bridge under `common/src/main/java/xyz/jpenilla/squaremap/common/bridge`, Rust server under `rust/crates/squaremap-server`, and comparison crate under `rust/crates/squaremap-compare`.
- The local gate uses a pinned Paper build, deterministic world, verified plugin/sidecar hashes, fresh data roots, isolated ports, and isolated Java/Rust output roots.
- Recording is opt-in, bounded, transparent, and secret-free. Dropped frames and writer failures fail the gate.
- No normalization may hide paths, status codes, headers, pixels, block state, lifecycle outcomes, ownership, queue loss, or mismatches.
- Every production change begins with a failing focused test and lands with that test in one atomic commit.
- Component tests are prerequisites, not substitutes for the final real Paper execution.
- The remote production-like gate remains `blocked_infrastructure` until its host, access, safe mutation scope, pinned artifacts, isolated roots, and metrics permissions are supplied.

---

### Task 1: Pin and validate the Paper proof fixture

**Files:**
- Create: `testdata/bridge/v1/paper-fixture.json`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/verification/ProofFixture.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/verification/ProofFixtureTest.java`
- Modify: `testdata/bridge/v1/manifest.json`

**Interfaces:**
- Produces: `ProofFixture.load(Path manifest) -> ProofFixture`
- Produces: validated Paper/plugin/sidecar artifacts, world seed/settings, roots, ports, scenario steps, timeouts, quiet interval, RSS interval, thresholds, and normalization rules.
- Invariant: missing artifacts, SHA-256 mismatch, root overlap/symlink alias, undeclared normalization, or missing required scenario step fails before Paper launch.

- [ ] **Step 1: Inspect existing fixture manifest schema**

Read `testdata/bridge/v1/manifest.json` and current comparison tests. Preserve existing fields and add a versioned `paper_fixture` object rather than introducing a second manifest convention.

- [ ] **Step 2: Write failing fixture validation tests**

Add full test cases for valid metadata, missing artifact, hash mismatch, output-root overlap, symlink alias, invalid port collision, undeclared normalization, missing lifecycle step, and invalid timeout/interval values.

- [ ] **Step 3: Run the focused test and confirm failure**

Run:

```bash
./gradlew :squaremap-common:test --tests '*ProofFixtureTest'
```

Expected: compilation failure because `ProofFixture` is absent.

- [ ] **Step 4: Implement the minimal validated fixture model**

Use immutable Java records and existing JSON facilities. Confine every resolved path beneath its declared root, compute SHA-256 before launch, reject overlapping canonical paths, and require scenario steps for readiness, baseline, mutation, churn, reload, cancel, resume, quiescence, kill, restart, replay, second mutation, and shutdown.

- [ ] **Step 5: Add pinned local fixture metadata**

Record the Paper/Minecraft version compatible with the current Paper module, exact artifact URLs and SHA-256 values, deterministic seed/settings, Linux process collector, ports/roots, timing intervals, performance thresholds, and the complete ordered scenario.

- [ ] **Step 6: Run the focused test**

Run the command from Step 3. Expected: every `ProofFixtureTest` case passes.

- [ ] **Step 7: Commit the fixture contract**

```bash
git add testdata/bridge/v1/paper-fixture.json testdata/bridge/v1/manifest.json common/src/main/java/xyz/jpenilla/squaremap/common/bridge/verification/ProofFixture.java common/src/test/java/xyz/jpenilla/squaremap/common/bridge/verification/ProofFixtureTest.java
git commit -m "Pin the Paper production proof fixture"
```

---

### Task 2: Record production bridge traffic transparently

**Files:**
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/recording/BridgeRecorder.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/recording/BridgeRecordingSink.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/recording/BridgeRecorderTest.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeConnection.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeBackendController.java`

**Interfaces:**
- Produces: `BridgeRecordingSink.record(Direction direction, ByteBuffer frame)` and `flush()/close()` with explicit failure propagation.
- Produces: SMRC-compatible records containing monotonic time, direction, exact frame bytes, and non-secret session/sequence/correlation metadata.
- Invariant: disabled recording is allocation-free on the bridge hot path; enabled recording preserves frame bytes and ordering and never silently drops.

- [ ] **Step 1: Locate all bridge frame write/read callsites**

Use LSP references for `BridgeConnection` frame send/read methods. Select the single post-coalescing outbound boundary and single decoded inbound frame boundary; do not record duplicate higher-level events.

- [ ] **Step 2: Write failing recorder tests**

Cover outbound/inbound byte identity, ordering, monotonic timestamps, disabled no-op behavior, bounded-capacity failure, token exclusion, path containment, flush/close, truncated destination, injected I/O failure, and preservation of the original transport exception.

- [ ] **Step 3: Run the focused test and confirm failure**

```bash
./gradlew :squaremap-common:test --tests '*BridgeRecorderTest'
```

Expected: compilation failure because recorder types are absent.

- [ ] **Step 4: Implement recorder and opt-in wiring**

Use a bounded queue and one diagnostics writer. Copy bytes only when recording is enabled. Make sink failure observable by the bridge supervisor and proof harness. Do not serialize bootstrap tokens or authentication material. Configure the diagnostics path only through the validated proof fixture/property.

- [ ] **Step 5: Run focused bridge tests**

```bash
./gradlew :squaremap-common:test --tests '*BridgeRecorderTest' --tests '*BridgeConnectionTest' --tests '*ProductionBridgeWiringTest'
```

Expected: all selected tests pass.

- [ ] **Step 6: Commit recorder support**

```bash
git add common/src/main/java/xyz/jpenilla/squaremap/common/bridge/recording common/src/test/java/xyz/jpenilla/squaremap/common/bridge/recording common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeConnection.java common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeBackendController.java
git commit -m "Record bridge frames transparently"
```

---

### Task 3: Validate quiescence and evidence fail closed

**Files:**
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/verification/QuiescenceSnapshot.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/verification/QuiescenceProbe.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/verification/QuiescenceProbeTest.java`
- Create: `rust/crates/squaremap-compare/src/evidence.rs`
- Create: `rust/crates/squaremap-compare/tests/evidence.rs`
- Modify: `rust/crates/squaremap-compare/src/lib.rs`
- Modify: `rust/crates/squaremap-compare/src/main.rs`

**Interfaces:**
- Produces: two-sample `QuiescenceProbe.await(Duration timeout, Duration quietInterval) -> QuiescenceSnapshot`.
- Consumes: process readiness, bridge queue/ack state, render/cancel state, dirty/durable work, output inventory/hash/mtime, temporary files, recorder flush, and lifecycle state.
- Produces: `squaremap-compare gate --evidence <manifest> --parity <report> --performance <report>` with exit 0 pass, 1 unmet gate/mismatch, 2 malformed/incomplete evidence.

- [ ] **Step 1: Write failing quiescence tests**

Cover stable two-sample success and failures for dead/unready process, non-empty queue, missing ACK, active render, pending cancellation, dirty job, changed hash/mtime, temporary file, unflushed recorder, outstanding lifecycle step, and missing metric.

- [ ] **Step 2: Write failing evidence tests**

Cover complete bundle, missing artifact, SHA mismatch, skipped scenario step, failed quiescence, wrong HTTP owner, nonzero mismatch count, incomplete replay, missing RSS samples, duplicate path, report overwrite, and remote `blocked_infrastructure` remaining distinct from pass.

- [ ] **Step 3: Confirm both test files fail**

```bash
./gradlew :squaremap-common:test --tests '*QuiescenceProbeTest'
cargo test --manifest-path rust/Cargo.toml -p squaremap-compare --test evidence
```

Expected: absent symbols/modules cause failure.

- [ ] **Step 4: Implement strict two-sample quiescence**

Capture all fields in immutable snapshots. Require equality/stability across the quiet interval and return named failing predicates. Treat missing observations as failures.

- [ ] **Step 5: Implement hash-addressed evidence validation**

Validate manifest schema/version, expected path inventory, SHA-256, lifecycle completion, ownership, replay, comparisons, performance samples, and aggregate verdict. Refuse an existing report destination.

- [ ] **Step 6: Run focused tests**

Run both Step 3 commands. Expected: all cases pass.

- [ ] **Step 7: Commit quiescence and evidence validation**

```bash
git add common/src/main/java/xyz/jpenilla/squaremap/common/bridge/verification common/src/test/java/xyz/jpenilla/squaremap/common/bridge/verification rust/crates/squaremap-compare
git commit -m "Fail closed on incomplete Paper evidence"
```

---

### Task 4: Build the black-box Paper lifecycle driver

**Files:**
- Create: `paper/src/test/java/xyz/jpenilla/squaremap/paper/verification/PaperProofDriver.java`
- Create: `paper/src/test/java/xyz/jpenilla/squaremap/paper/verification/PaperProofDriverTest.java`
- Create: `paper/src/test/java/xyz/jpenilla/squaremap/paper/verification/PaperControlPlugin.java`
- Create: `paper/src/test/resources/paper-proof/plugin.yml`
- Modify: `paper/build.gradle.kts`
- Modify: `testdata/bridge/v1/paper-fixture.json`

**Interfaces:**
- Produces Gradle task `:squaremap-paper:paperProductionProof` with fixture, sidecar, mode, diagnostics, and optional retained-run properties.
- Driver interacts through Paper console/RCON or the fixture control plugin command surface, HTTP, process state, and files; it does not call Squaremap internals.
- Lifecycle transcript is ordered JSONL with step IDs, start/end, acknowledgements, epochs/revisions, process generations, hashes, barriers, and failure reasons.

- [ ] **Step 1: Write failing driver contract tests**

Cover exact scenario order, readiness timeout, command timeout, unexpected process exit, duplicate/skipped step, root/port overlap, mutation command acknowledgement, world epoch advancement, player state transitions, cancellation terminal state, resume completion, quiescence invocation, and deterministic transcript schema.

- [ ] **Step 2: Confirm driver tests fail**

```bash
./gradlew :squaremap-paper:test --tests '*PaperProofDriverTest'
```

Expected: absent driver/control types cause failure.

- [ ] **Step 3: Implement fixture control commands**

Expose commands only in the proof fixture for deterministic block set/break at declared coordinates, fixture-player lifecycle/visibility state, world unload/reload, full-render start/cancel/resume, metrics snapshot, and barrier acknowledgement. Commands must execute through Paper-safe scheduling APIs and produce explicit completion/failure acknowledgements.

- [ ] **Step 4: Implement process driver and Gradle task**

Launch fresh Paper data, install the production plugin and fixture control plugin, stream logs, allocate declared ports, await explicit Paper/plugin/sidecar/HTTP readiness, execute the scenario, and always perform bounded process-tree cleanup. Emit a unique diagnostics root and never overwrite a prior run.

- [ ] **Step 5: Implement HTTP and filesystem probes**

Capture `GET`/`HEAD`, status, required cache headers/ETags, body hashes, missing-tile behavior, listener PID/executable, output inventory/hash/mtime, temporary files, and root ownership after each barrier.

- [ ] **Step 6: Run driver contract tests**

Run the Step 2 command. Expected: all driver tests pass without starting a full production scenario.

- [ ] **Step 7: Commit the lifecycle harness**

```bash
git add paper/build.gradle.kts paper/src/test testdata/bridge/v1/paper-fixture.json
git commit -m "Drive the Paper production proof lifecycle"
```

---

### Task 5: Prove sidecar kill, restart, and replay inside Paper

**Files:**
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/SidecarSupervisor.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeBackendController.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/process/SidecarReplayRecoveryTest.java`
- Modify: `paper/src/test/java/xyz/jpenilla/squaremap/paper/verification/PaperProofDriver.java`
- Create: `paper/src/test/java/xyz/jpenilla/squaremap/paper/verification/PaperRecoveryScenarioTest.java`
- Modify: `rust/crates/squaremap-compare/tests/replay.rs`

**Interfaces:**
- Existing supervisor policy remains authoritative for retry count/backoff; the driver records rather than bypasses it.
- Produces process-generation and authenticated-session evidence for kill, restart, baseline replay, dirty/render recovery, and final convergence.
- Invariant: Paper remains alive; Java never assumes Rust HTTP/output ownership after failure.

- [ ] **Step 1: Audit supervisor/reconnect state transitions and references**

Use LSP references for supervisor start/stop/restart and bridge reconnect callbacks. Identify any missing production replay trigger before editing.

- [ ] **Step 2: Write failing recovery tests**

Cover kill after readiness, kill during block mutation, kill during full render, reconnect after world reload, new authenticated session, stale sequence rejection, complete baseline resend, dirty/render convergence, restart budget exhaustion, and explicit shutdown cancelling pending restart.

- [ ] **Step 3: Confirm focused recovery tests fail**

```bash
./gradlew :squaremap-common:test --tests '*SidecarReplayRecoveryTest'
./gradlew :squaremap-paper:test --tests '*PaperRecoveryScenarioTest'
cargo test --manifest-path rust/Cargo.toml -p squaremap-compare --test replay
```

Expected: newly asserted production recovery/replay behavior fails until wired.

- [ ] **Step 4: Implement the missing production recovery path**

Preserve existing supervisor policy. On authenticated reconnect, replay the current replacement state and recover durable render/dirty work exactly once. Reject stale session/sequence input. Explicit shutdown must suppress restart.

- [ ] **Step 5: Add real child-kill lifecycle step**

Have the external driver obtain the supervised child PID, send SIGKILL to only that generation, prove Paper HTTP/control responsiveness, observe restart/backoff/authentication/replay, repeat a real mutation, and await final quiescence.

- [ ] **Step 6: Run focused recovery tests**

Run all Step 3 commands. Expected: all selected tests pass.

- [ ] **Step 7: Commit recovery behavior and proof**

```bash
git add common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process common/src/test/java/xyz/jpenilla/squaremap/common/bridge/process paper/src/test/java/xyz/jpenilla/squaremap/paper/verification rust/crates/squaremap-compare/tests/replay.rs
git commit -m "Recover the Rust sidecar inside Paper"
```

---

### Task 6: Collect paired performance and RSS evidence

**Files:**
- Create: `paper/src/test/java/xyz/jpenilla/squaremap/paper/verification/LinuxProcessMetrics.java`
- Create: `paper/src/test/java/xyz/jpenilla/squaremap/paper/verification/LinuxProcessMetricsTest.java`
- Modify: `paper/src/test/java/xyz/jpenilla/squaremap/paper/verification/PaperProofDriver.java`
- Modify: `rust/crates/squaremap-compare/src/benchmark.rs`
- Modify: `rust/crates/squaremap-compare/tests/benchmark.rs`
- Modify: `testdata/bridge/v1/paper-fixture.json`

**Interfaces:**
- Produces fixed-interval Paper and sidecar PID/generation samples: RSS, CPU time, sample timestamp, collector/source, and missing-process reason.
- Produces paired Java/Rust `performance.json`: warm-up/measured repetitions, event-to-frame/output latency, scenario wall time, render throughput, queue high-water, frame bytes, peak/steady RSS, machine metadata, thresholds, and verdict.
- Invariant: performance cannot pass unless parity, lifecycle, recovery, and evidence completeness already pass.

- [ ] **Step 1: Write failing collector and report tests**

Cover `/proc/<pid>/status` parsing, KiB conversion, PID generation change, process exit, missing/permission-denied sample, fixed interval metadata, warm-up exclusion, paired sample mismatch, percentile calculation, threshold boundaries, and correctness-first gating.

- [ ] **Step 2: Confirm focused tests fail**

```bash
./gradlew :squaremap-paper:test --tests '*LinuxProcessMetricsTest'
cargo test --manifest-path rust/Cargo.toml -p squaremap-compare --test benchmark
```

Expected: new collector/report assertions fail.

- [ ] **Step 3: Implement external process sampling**

Read Linux `/proc` without retaining process handles beyond each sample. Attribute sidecar samples to process generation and record every gap explicitly. Sample Paper and Rust simultaneously during the identical scenario.

- [ ] **Step 4: Extend paired benchmark report**

Retain raw samples and compute declared medians, percentiles, maxima, throughput, and threshold verdicts. Include fixture hashes and machine metadata. Reject unequal scenario/action inventories.

- [ ] **Step 5: Run focused tests**

Run both Step 2 commands. Expected: all cases pass.

- [ ] **Step 6: Commit measurement support**

```bash
git add paper/src/test/java/xyz/jpenilla/squaremap/paper/verification rust/crates/squaremap-compare/src/benchmark.rs rust/crates/squaremap-compare/tests/benchmark.rs testdata/bridge/v1/paper-fixture.json
git commit -m "Measure Paper proof performance and RSS"
```

---

### Task 7: Execute and archive the local production proof

**Files:**
- Modify only if the executed gate exposes a production defect: the exact responsible source and its focused regression test.
- Create: `docs/superpowers/verification/paper-production-proof-verdict.json`
- Create: `docs/superpowers/verification/paper-production-proof-summary.md`
- Modify: `testdata/bridge/v1/manifest.json` only for deterministic, repository-approved corpus hashes.

**Interfaces:**
- Consumes `:squaremap-paper:paperProductionProof` and `squaremap-compare gate`.
- Produces immutable diagnostics plus a repository-sized verdict/summary containing commands, commits, environment, thresholds, evidence hashes, each six-claim verdict, aggregate local verdict, and remote `blocked_infrastructure` status.

- [ ] **Step 1: Run focused prerequisite suites**

```bash
./gradlew :squaremap-common:test --tests '*BridgeRecorderTest' --tests '*QuiescenceProbeTest' --tests '*SidecarReplayRecoveryTest'
./gradlew :squaremap-paper:test --tests '*PaperProofDriverTest' --tests '*PaperRecoveryScenarioTest' --tests '*LinuxProcessMetricsTest'
cargo test --manifest-path rust/Cargo.toml -p squaremap-compare
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test bridge_mode --test http_contract --test render_recovery
```

Expected: all selected suites pass. These results remain prerequisite evidence only.

- [ ] **Step 2: Build the exact production artifacts**

```bash
./gradlew :squaremap-paper:shadowJar
cargo build --release --manifest-path rust/Cargo.toml -p squaremap-server -p squaremap-compare
```

Record artifact SHA-256 values into the run manifest and verify they match the fixture inputs.

- [ ] **Step 3: Execute Java-primary twin**

```bash
./gradlew :squaremap-paper:paperProductionProof -PproofMode=java -PproofFixture=testdata/bridge/v1/paper-fixture.json
```

Expected: every mutation/churn/reload/cancel-resume/quiescence step completes and writes the Java evidence root.

- [ ] **Step 4: Execute Rust-primary twin with in-lifecycle kill/restart**

```bash
./gradlew :squaremap-paper:paperProductionProof -PproofMode=rust -PproofFixture=testdata/bridge/v1/paper-fixture.json -PproofSidecar=rust/target/release/squaremap-server
```

Expected: Rust owns the public listener/output root; Paper survives sidecar SIGKILL; authenticated restart/replay occurs; the repeated mutation converges; final quiescence passes.

- [ ] **Step 5: Compare, replay, and validate the bundle**

```bash
cargo run --release --manifest-path rust/Cargo.toml -p squaremap-compare -- compare-output --java <java-output-root> --rust <rust-output-root> --report <diagnostics>/comparisons/parity.json
cargo run --release --manifest-path rust/Cargo.toml -p squaremap-compare -- replay --recording <diagnostics>/recordings/bridge.smrc --output <diagnostics>/replay-output
cargo run --release --manifest-path rust/Cargo.toml -p squaremap-compare -- gate --evidence <diagnostics>/manifest.json --parity <diagnostics>/comparisons/parity.json --performance <diagnostics>/performance.json
```

Expected: all commands exit 0; path/JSON/RGBA/replay mismatch counts are zero; all quiescence, ownership, lifecycle, recovery, measurement, and checksum gates pass.

- [ ] **Step 6: Fix every observed production defect at its source**

For each nonzero gate, first add a focused test reproducing the exact ownership, mutation, lifecycle, recovery, performance, or evidence failure. Implement the minimal source fix, rerun its focused test, then repeat Steps 3–5 from fresh roots. Do not add mismatch exemptions or reuse partial evidence.

- [ ] **Step 7: Record local evidence and remote blocker**

Write the exact commands, commit, environment, fixture/artifact hashes, evidence bundle checksum, thresholds, raw result locations, and six gate verdicts. Set the remote verdict to `blocked_infrastructure` with the missing host/access/safe-mutation/isolation/metrics prerequisites.

- [ ] **Step 8: Run final current-commit verification**

Repeat Step 1 and the complete fresh Steps 3–5. The archived verdict must refer to this current commit and evidence bundle.

- [ ] **Step 9: Commit repository-safe evidence metadata**

```bash
git add docs/superpowers/verification/paper-production-proof-verdict.json docs/superpowers/verification/paper-production-proof-summary.md testdata/bridge/v1/manifest.json
git commit -m "Record the local Paper production proof"
```

Do not commit secrets, private player data, unapproved binaries/worlds, or bulky raw logs. Keep raw evidence at the hash-addressed location named in the summary.

## Final acceptance

- [ ] The Rust child owns the public HTTP socket and Rust output root in the executed Rust-primary Paper run; Java owns neither.
- [ ] Real Paper block set/break/mutation events change tiles and produce zero missing/extra/semantic JSON/decoded RGBA mismatches, including dependent neighbors.
- [ ] World epoch replacement, player churn/privacy, render cancellation/resumption, final completion, shutdown, and all quiescence barriers pass without stale/lost/duplicate/late work.
- [ ] Paper remains alive while the supervised sidecar is killed; bounded authenticated restart, replay, dirty/render recovery, and exact convergence pass.
- [ ] Paired Java/Rust timing, throughput, queue/frame, CPU, Paper RSS, and sidecar RSS data are complete and satisfy predeclared thresholds.
- [ ] The final bundle is complete and hash-valid; comparison and replay report exactly zero mismatches.
- [ ] The remote production-like verdict remains `blocked_infrastructure` until that environment is supplied and executed; local evidence is not presented as remote evidence.
