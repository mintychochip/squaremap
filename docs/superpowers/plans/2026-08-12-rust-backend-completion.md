# Rust Backend Migration Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the remaining Rust sidecar migration gates and cleanly transfer backend ownership from Java without breaking loaders, the public Java API, or the existing web contract.

**Architecture:** Finish the missing comparison/replay crate, then wire mode-exclusive HTTP/output lifecycle ownership in Java. Add operational evidence and compatibility/distribution gates before making Rust the observation-release default and deleting obsolete Java backend paths.

**Tech Stack:** Rust 2024/MSRV 1.88, Tokio, Prost, Serde, PNG, SHA-256; Java 25/Gradle Kotlin DSL/JUnit; existing Bun/Vite web UI.

## Global Constraints

- Use `/home/jlo/dev/squaremap/.worktrees/rust-backend-migration`; do not touch unrelated `master` worktree edits.
- Preserve current JSON paths, PNG paths, HTTP cache headers, missing-PNG behavior, Java addon API, and Paper/Fabric/NeoForge/Sponge integration.
- Keep child-process loopback TCP, length-delimited Protobuf, bounded queues, zstd only for chunk bodies, and no JNI/gRPC.
- `java`, `shadow`, and `rust` use separate output roots; Rust mode must not start Java HTTP/render ownership.
- Java remains the default until replay, live-shadow, fault, and isolated-performance gates all pass; only then may an observation release select Rust.
- Every production behavior gets a failing test first; commit each coherent green slice atomically.
- Do not delete legacy Java backend until replay, shadow, fault, and isolated-performance evidence exists.

---

### Task 1: Implement comparison recording and semantic comparison

**Files:**
- Modify: `rust/crates/squaremap-compare/Cargo.toml`
- Modify: `rust/crates/squaremap-compare/src/lib.rs`
- Create: `rust/crates/squaremap-compare/src/recording.rs`
- Create: `rust/crates/squaremap-compare/src/compare.rs`
- Create: `rust/crates/squaremap-compare/src/report.rs`
- Create: `rust/crates/squaremap-compare/src/main.rs`
- Create: `rust/crates/squaremap-compare/tests/replay.rs`
- Create: deterministic fixture manifest/recording/output files under `testdata/bridge/v1`

**Interfaces:**
- `RecordingWriter`, `RecordingReader`, and deterministic replay over `{u64 nanos,u8 direction,u32 len,bytes}` records.
- `compare_output(java_root, rust_root) -> ComparisonReport` with canonical JSON and decoded-RGBA PNG semantics.
- `write_report(path, report)` refuses overwrite.
- CLI subcommands `replay`, `compare-output`, `report`; exit 0 equal, 1 mismatch, 2 invalid/tool failure.

**Steps:**
- [ ] Write tests for truncation, invalid protocol version, JSON key-order equivalence, array-order mismatch, exact RGBA equality across PNG encodings, first pixel mismatch, missing/extra paths, and report no-overwrite.
- [ ] Run `cargo test --manifest-path rust/Cargo.toml -p squaremap-compare --test replay`; verify expected missing-symbol failures.
- [ ] Implement bounded reader/writer and comparison/report APIs with explicit size limits and deterministic ordering.
- [ ] Implement CLI argument parsing without introducing an unnecessary framework; validate paths and exit codes.
- [ ] Run focused compare tests, then `cargo test --manifest-path rust/Cargo.toml --workspace`.
- [ ] Run fixture comparison and verify `mismatch_count: 0`.
- [ ] Commit: `Implement Rust replay and parity comparison`.

---

### Task 2: Enforce mode-exclusive HTTP/output lifecycle

**Files:**
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/SquaremapCommon.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/BridgeBackendController.java`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/process/ProductionBridgeWiringTest.java`
- Create/modify focused lifecycle test fixture as needed.

**Interfaces:**
- Java mode starts/stops `IntegratedServer` and Java cache.
- Shadow mode keeps Java authoritative and starts Rust mirror without public HTTP ownership.
- Rust mode waits for Rust readiness and never starts/stops `IntegratedServer`/`JsonCache`.

**Steps:**
- [ ] Write failing lifecycle tests for Java, shadow, and Rust ownership transitions.
- [ ] Run focused Gradle tests and confirm Rust-mode failure before implementation.
- [ ] Gate Java HTTP lifecycle and state publication by `BackendMode`; make Rust readiness the Rust startup condition.
- [ ] Run focused tests and `./gradlew :common:test`.
- [ ] Commit: `Make backend HTTP ownership mode-specific`.

---

### Task 3: Add fault/recovery, metrics, and performance gates

**Files:**
- Create: `rust/crates/squaremap-server/src/metrics.rs`
- Create: `rust/crates/squaremap-server/tests/fault_recovery.rs`
- Create: `rust/crates/squaremap-compare/src/benchmark.rs`
- Create: `rust/crates/squaremap-compare/tests/benchmark.rs`
- Modify: corresponding crate manifests/lib exports.

**Interfaces:**
- Metrics expose queue depth, render latency, HTTP requests, restarts, and protocol errors.
- Fault tests cover sidecar disconnect, restart, state recovery, bounded retry, and no half-initialized HTTP.
- Benchmark gate emits deterministic JSON with configured thresholds and pass/fail verdict.

**Steps:**
- [ ] Write failing fault and benchmark contract tests.
- [ ] Run focused tests to verify failure.
- [ ] Implement bounded metrics and deterministic benchmark report; do not add unbounded queues or tick-thread I/O.
- [ ] Run focused Rust tests and benchmark command.
- [ ] Commit: `Add Rust backend fault and performance gates`.

---

### Task 4: Add binary manifest and configuration compatibility

**Files:**
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BinaryResolver.java`
- Create: focused Java resolver tests.
- Create/modify: `rust/crates/squaremap-server/src/config/source.rs`, `schema.rs`, `policy.rs` and tests as required by existing module ownership.
- Create: config compatibility fixtures and tests.
- Modify: manifests/build integration only where required.

**Interfaces:**
- Resolver accepts configured absolute path or versioned cache/release path only when SHA-256 matches `BackendManifest`.
- Rust config parser rejects incompatible schema/unknown unsafe values with structured errors.
- Bridge policy/config replacement remains versioned and additive.

**Steps:**
- [ ] Write failing resolver hash/path tests and config compatibility tests.
- [ ] Run focused Java/Rust tests to confirm failure.
- [ ] Implement manifest verification, safe path confinement, config source/schema, and compatibility handling.
- [ ] Run focused tests plus Gradle common tests.
- [ ] Commit: `Verify Rust binaries and configuration compatibility`.

---

### Task 5: Produce parity, shadow, fault, and observation evidence

**Files:**
- Create: `docs/superpowers/verification/rust-backend-ab-report.json`
- Create: `docs/superpowers/verification/rust-backend-summary.md`
- Create: live-shadow/fault/performance fixture outputs under `testdata/bridge/v1` only when deterministic and required.
- Modify: CLI/report code only for missing evidence fields.

**Steps:**
- [ ] Run replay parity and record zero semantic mismatches.
- [ ] Run Java-primary/Rust-shadow and confirm separate roots, no public-port conflict, and bounded bridge behavior.
- [ ] Run sidecar fault/restart/recovery scenarios and record outcomes.
- [ ] Run isolated performance benchmark and compare thresholds.
- [ ] Write reports containing commands, commits, environment, thresholds, results, and verdicts; no fabricated values.
- [ ] Commit: `Record Rust backend migration gate evidence`.

---

### Task 6: Make Rust default for observation release and clean cutover

**Files:**
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeBootstrapConfig.java`
- Modify all backend callers found by LSP/references.
- Delete only after boundary tests: Java `IntegratedServer`, `JsonCache`, Java render executors/implementations, legacy backend controller/mode, Undertow dependency, and obsolete migration aliases.
- Create: clean-cutover boundary test.

**Steps:**
- [ ] Write failing boundary test proving Rust owns HTTP/output with no legacy backend classes reachable.
- [ ] Run focused test and verify failure.
- [ ] Keep Java as the default until every replay, live-shadow, fault/recovery, and isolated-performance gate has passed and been recorded; then select Rust for the observation release without deleting rollback evidence.
- [ ] Remove legacy backend paths and migrate every caller; use LSP references for symbol removal.
- [ ] Run `cargo test --manifest-path rust/Cargo.toml --workspace`, `./gradlew build`, web lint/build, and loader smoke checks.
- [ ] Commit atomic clean-cutover units, splitting unrelated docs/chore changes.

---

## Final acceptance

The migration is complete only when the workspace tests, Gradle build, web build/lint, replay parity, live-shadow, fault/recovery, isolated-performance, observation-release, and clean-cutover boundary checks all pass on the current commit with reports matching the exercised commands.
