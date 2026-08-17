# Migration Contract Boundaries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define and test the missing contracts that must exist before Squaremap can safely implement cache resolution, dirty-event recovery, replay, SHADOW ownership, and Paper proof.

**Architecture:** Keep Java as the default and preserve the current authenticated loopback protocol. Add no behavior that cannot be verified against an explicit contract. First define pure, fail-closed contract models and focused tests; only then wire production paths. Paper artifacts remain blocked until real pinned inputs are supplied.

**Tech Stack:** Java 25, JUnit 5, Protocol Buffers, Rust 2024, Tokio, Gradle, Cargo.

## Global Constraints

- Work only in `/home/jlo/dev/squaremap/.worktrees/rust-backend-migration`.
- Do not switch the default backend from Java.
- Do not delete legacy Java backend paths.
- Do not fabricate Paper, replay, performance, release, or parity evidence.
- Every production behavior starts with a failing focused test.
- Preserve authenticated loopback transport, session IDs, contiguous sequences, bounded queues, separate output roots, JSON/PNG/HTTP contracts, and platform integrations.
- Any missing observation or contract is a fail-closed result, not an inferred pass.

---

### Task 1: Define verified cache candidate contract

**Files:**
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BinaryResolver.java`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/process/BinaryResolverTest.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeBootstrapConfig.java` only after the pure contract passes

**Interfaces:**
- Produce `BinaryResolver.cacheCandidate(Path cacheRoot, String pluginVersion, String targetTriple, BackendManifest.BackendBinary expected) -> Path`.
- The candidate is confined beneath `cacheRoot`, includes version and target components, and is never downloaded by this task.
- Produce `BinaryResolver.verifyCachedPath(...)` only if the candidate contract has an explicit caller; otherwise reuse `verifyConfiguredPath`.
- The cache API is contract-only until a launch handoff can bind the verified bytes/path atomically; a pathname returned after hashing is not sufficient.

- [x] Step 1: Add focused tests for verified cache hit path derivation, missing cache file, wrong length/hash, symlink rejection, traversal rejection, and offline miss. The contract intentionally does not claim a production verification-to-launch handoff; the eventual launcher must still bind verified bytes atomically.
- [x] Step 2: Run `./gradlew :squaremap-common:test --tests '*BinaryResolverTest' --no-daemon`; confirm the new API is absent.
- [x] Step 3: Implement only deterministic confined path derivation and exact existing manifest verification; reject missing cache entries without network access. Do not return a verified `Path` for eventual `ProcessBuilder` use; the eventual launcher must open/execute the same verified file descriptor or perform an ownership/immutability handoff.
- [x] Step 4: Run the focused resolver tests and existing bootstrap target tests.
- [x] Step 5: Have workflowz review the path/permission/hash contract before wiring configuration.

---

### Task 2: Define dirty resync protocol contract

**Files:**
- Modify: `protocol/squaremap/bridge/v1/bridge.proto`
- Create: `rust/crates/squaremap-server/tests/dirty_resync_contract.rs`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/process/InboundSequenceTrackerTest.java` only if protocol sequence tests require shared helpers

**Interfaces:**
- Add explicit protocol messages for resume watermark, dirty replay request, replay item, and resync completion only after their fields are agreed.
- The contract must identify session/revision/world epoch, last durably accepted sequence, replay ordering, and completion/failure.
- No Java or Rust production handler is wired until both sides have focused malformed-order and duplicate tests.

- [ ] Step 1: Write protocol contract tests against the desired message shape and reject the absent schema. Current tests cover malformed fields through the Rust validator and Java generated-message construction; production recovery remains unwired.
- [x] Step 2: Document sequence/session/revision invariants in the protocol comments, including stable `bridge_id` versus reconnect-scoped `session_id`.
- [ ] Step 3: Add protobuf messages with reserved field numbers and bounded repeated data. Existing tags remain unchanged; dirty-resync messages use append-only tags 55–58, and `max_items` is bounded to 1024 in comments, validator, and focused tests. Production negotiated bounds remain unwired, so this gate is intentionally incomplete.
- [x] Step 4: Add Rust decoding/order tests and Java generated-message compatibility tests. Rust validator tests cover malformed ordering, duplicates, identity, revision, replay, completion, and request-page limits; Java `SchemaContractTest` covers generated construction/decoding when the appended `bridge_id` is absent; production recovery remains unwired.
- [ ] Step 5: Have workflowz review whether the contract can recover events accepted-but-unacknowledged across a fresh connection. Workflowz confirms the validators are internally strict, but production recovery remains unavailable: bootstrap rejects identity negotiation and dirty recovery, fresh sessions restart sequence numbering, and no checkpoint continuity handshake exists. This gate remains incomplete.
- [ ] Identity integration follow-up: Workflowz found two P1 risks and both are fixed: all resync validators now reject zero bridge IDs, and bridge checkpoint reconnects refresh `session_id` while retaining the maximum durable sequence. Bootstrap still does not consume or persist `BridgeIdentityReplace`.
- [ ] Replay ownership prerequisite: add an explicit bridge ownership relation for dirty rows before dispatch. The current global `dirty_chunks` table cannot safely page, complete, or defer work for a bridge. The eventual cutover must authenticate and persist `BridgeIdentityReplace`, restore session continuity from a durable watermark, make dirty writes and owner-scoped page/complete/defer operations atomic, and test cross-bridge isolation, reconnect, stale epochs, retry bounds, and pagination before removing bootstrap rejection.

---

### Task 3: Define SHADOW HTTP ownership contract

**Files:**
- Modify: `protocol/squaremap/bridge/v1/bridge.proto`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/config/ConfigBridgeExporter.java`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/backend/BackendModeLifecycleTest.java`
- Modify: `rust/crates/squaremap-server/src/bootstrap.rs`
- Modify: `rust/crates/squaremap-server/tests/bridge_mode.rs`

**Interfaces:**
- Choose and encode one explicit rule: in SHADOW, Java owns public HTTP/cache and Rust receives `http_enabled=false`; in RUST, Rust owns the configured HTTP listener.
- Preserve `Ready.http_enabled` as the observed ownership result.
- Reject contradictory ownership fields before binding.

- [x] Step 1: Add a failing Java lifecycle-policy test proving SHADOW exports Java HTTP ownership and Rust does not own HTTP.
- [x] Step 2: Add a Java exporter/immutability contract proving SHADOW disables Rust HTTP and reload cannot change the captured ownership mode.
- [x] Step 3: Add a failing Rust bridge-mode test proving a SHADOW-equivalent replacement (`http_enabled=false`) does not request Rust HTTP binding.
- [x] Step 4: Preserve the existing explicit ownership signal in `GlobalSettings.http_enabled`; no additional mode field is added because the authenticated config protocol does not carry backend mode and Java already exports the immutable mode decision.
- [x] Step 5: Wire Rust validation/bind behavior minimally, run focused Java/Rust ownership tests, and obtain workflowz review.

---

### Task 4: Build injected quiescence snapshots without claiming Paper proof

**Files:**
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/verification/QuiescenceSnapshot.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/verification/QuiescenceProbe.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/verification/QuiescenceProbeTest.java`

**Interfaces:**
- Produce immutable `QuiescenceSnapshot` containing readiness, bridge pending/ack state, render/cancel state, dirty/durable work, output inventory/hash/mtime, temporary files, recorder flush state, lifecycle state, and metric availability.
- Produce `QuiescenceProbe.await(Duration timeout, Duration quietInterval) -> QuiescenceSnapshot` using an injected `Supplier<QuiescenceSnapshot>`.
- Missing observations fail closed. Two consecutive equal snapshots separated by `quietInterval` are required.

- [x] Step 1: Write failing tests for stable two-sample success and each missing/unstable predicate.
- [x] Step 2: Run `./gradlew :squaremap-common:test --tests '*QuiescenceProbeTest' --no-daemon`; confirm symbols are absent.
- [x] Step 3: Implement immutable value types and condition-based waiting without production Paper hooks.
- [x] Step 4: Run focused quiescence tests.
- [x] Step 5: Have workflowz review that the probe cannot report success when metrics are unavailable.

---

### Task 5: Preserve replay as an explicit offline boundary

**Files:**
- Modify: `rust/crates/squaremap-compare/src/recording.rs`
- Modify: `rust/crates/squaremap-compare/tests/replay.rs`
- Modify: `rust/crates/squaremap-compare/src/main.rs`

**Interfaces:**
- Keep `replay --output` as bounded exact-frame round-trip only.
- Reject live replay without an authenticated sink, session bootstrap, sequence policy, and dispatch contract.
- Add explicit metadata validation for direction, monotonic timestamps, frame limits, and secret-free scope if recordings are extended.

- [x] Step 1: Add failing tests for malformed direction and timestamp metadata. Secret-bearing bootstrap exclusion remains blocked until a production recorder boundary and frame-classification contract exist.
- [x] Step 2: Run the focused replay tests and confirm the failures.
- [x] Step 3: Implement strict metadata validation without adding live replay. `encode_checked` and `decode` now enforce matching 1,000,000-frame, 64 MiB/frame, and 128 MiB aggregate payload bounds.
- [x] Step 4: Run compare tests and workflowz security review.

---

## Acceptance

This plan is not a migration-complete claim. The migration remains incomplete until current evidence proves workspace and Gradle builds, web build/lint, independent replay parity, live Java-primary/Rust-shadow behavior, Paper fault/recovery, isolated performance/RSS, release packaging/default, and clean Java-backend cutover.
