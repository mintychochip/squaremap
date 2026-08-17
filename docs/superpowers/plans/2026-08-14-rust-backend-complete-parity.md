# Rust Backend Complete Parity Plan

**Status:** implementation plan; not a parity, release, or cutover claim  
**Default during this plan:** Java  
**Primary rule:** no phase advances on indirect, stale, synthetic-only, or scope-mismatched evidence.

## Target ownership

Java retains only loader lifecycle, Minecraft events and safe snapshot capture, player privacy decisions, the public addon API, platform commands/permissions, and sidecar supervision. Rust must own HTTP, canonical UI state, JSON/assets, dirty/render persistence, scheduler, renderer, PNG pyramid, atomic output, metrics, and recovery. The web URL/file contract stays unchanged.

## Evidence contract

Every gate artifact contains fields required by its `evidence_kind`; irrelevant fields are explicit `not_applicable`, never fabricated. All include commit SHA, dirty-tree state, timestamp, toolchain, exact command/exit code, input/config hashes, comparator version, and raw artifact hashes. Live/Paper/performance evidence also includes Java/Rust/Paper PIDs, process generations, roots/ports, scenario and quiescence hashes, sample counts, and metric sources. Remote evidence includes immutable URL/digest, request transcript, target host/OS/architecture/toolchain attestation, and execution result. Canary/observation evidence binds every start/restart to that digest. Fixtures cannot satisfy Paper, remote, performance, canary, or observation gates. Missing observations fail closed.

Current authoritative baseline is `docs/superpowers/verification/rust-backend-completion-disposition.md`. Older summaries and A/B JSON are historical where they contradict it or lack commit/provenance metadata.

## Phase 0 — Build the parity matrix and evidence gate

**Files:** `docs/superpowers/verification/rust-backend-parity-matrix.json`, `rust/crates/squaremap-compare/src/{main,report,compare}.rs`, comparator tests.

1. Define one row per Java-observable behavior: startup/shutdown; protocol/security; config/locale/privacy; worlds/players/markers/icons; render controls/results/progress; pixels/PNG; JSON; HTTP GET/HEAD/cache/ETag/missing tiles/disabled HTTP/dev frontend; dirty/replay/reconnect; addon API; loaders; packaging; rollback; deletion.
2. Each row records Java oracle, Rust implementation, deliberate divergence or `none`, scenario, denominator, evidence kind, status, dependencies, and artifacts.
3. Implement `squaremap-compare gate --evidence FILE` to reject stale metadata, incomplete inventories, wrong evidence kind, ignored mismatches, absent raw hashes/metrics, and unproven ownership.
4. Recorder truncation, dropped frames, unflushed output, redaction failure, or hash mismatch fails closed.

**Exit:** every architecture acceptance criterion and Java behavior has exactly one independently reviewed row.

## Phase 1 — Complete transport, security, durable identity, and replay

**Files:** `bridge.proto`; Java `BridgeConnection`, `SidecarSupervisor`, `BridgePublisher`, `DirtyChunkPublisher`; Rust `bootstrap.rs`, `session.rs`, `dirty_resync.rs`, `repository.rs`.

1. Wire producer and consumer dispatch, acknowledgements, typed errors, and no-silent-drop tests for `ResumeWatermark`, `DirtyReplayRequest`, ordered `DirtyReplayItem`, `DirtyResyncComplete`, and `WorldResyncRequired` in both production peers. Reconnect must replay current `ConfigReplace` before readiness, controls, or state baselines.
2. On reconnect: authenticate; publish stable `bridge_id`; bind fresh `session_id`; load durable checkpoint; negotiate watermark; replay owner-scoped pages; atomically complete/defer; then admit live dirty traffic.
3. Preserve accepted-but-unacknowledged events. Enforce replay uniqueness, contiguous indices, max 1024, duplicate idempotency, stale epoch/session/revision rejection, owner isolation, lease reclaim, bounded retry, and hard failure on gaps.
4. Keep dirty write and checkpoint transactionally consistent. Never infer identity for legacy rows: leave them unassigned/quarantined until explicit migration. Lease release keys include full world identity, and retry rows are removed only after matching owner/revision completion.
5. Verify loopback-only peers, one-use 256-bit token through stdin, startup/handshake timeouts, exact major and compatible-minor negotiation on both peers, framing limits before allocation, zstd length/CRC/decompression bounds, path confinement, secret/address-free logs, and no unauthenticated control endpoint.
6. Bound every queue; expose depth/high-water/drop/resync metrics; retain snapshot credits.

**Tests:** cross-language golden frames; malformed lengths/compression; crash before ready; wrong token/version; disconnect before/after commit; duplicate/out-of-order/gap replay; reconnect during replay; stale epoch; two identities; saturation; lease reclaim; shutdown during replay.

**Exit:** a real Rust process reconnect proves authenticated replacement, preserved watermark, bounded replay, zero lost/duplicated dirty coordinates, isolation, and convergence. Validators alone are insufficient.

## Phase 2 — Complete controls, scheduling, and durable render lifecycle

**Files:** Java `BackendController`, `LegacyBackendController` oracle, `BridgeBackendController`; Rust `control.rs`, `bootstrap.rs`, scheduler modules, state repository/model.

1. Map every Java operation/result: full/radius, in-progress, invalid radius, cancel, pause/resume, reset, reload/config sync, health, progress restart, timeout/failure, rendered counts, substitutions.
2. Exercise production dispatch in `bootstrap.rs`; `control.rs` is only baseline validation.
3. Implement true full-world coordinate discovery from Java/Minecraft or an explicit bounded enumeration protocol. Dirty rows are not a full-world catalog.
4. Persist jobs/cursor/progress and correlated results. Convert asynchronous failures from log-only events into typed outcomes.
5. Prove exactly-once controls, restart recovery, reset, no late writes after cancel/reset/shutdown, and no temp files. Serialize cancellation/reset generation with tile installation so a concurrent cancel cannot publish afterward.

**Exit:** representative-world matrix has identical command/result/progress behavior, including coordinate-less full render over the enabled-world workload.

## Phase 3 — Complete state, addon, output, PNG, and HTTP parity

**Files:** Java bridge state exporters and config exporter; Rust canonical/view state, views/output/config, renderer/pyramid/PNG/visibility, HTTP modules.

1. Match global settings, players, world settings, marker geometry/layers, icons/assets, visibility, epochs, and volatile-field semantics. Resolve ambiguous geometry tags; no ignored list.
2. Preserve addon API behavior: immutable world collections, optional lookup, layer/marker/icon add/remove/update, player manager, serializers, web directory; exported mutations must reach Rust output.
3. Cover config reload, locale/messages, names, spectator/invisibility/privacy, visibility limits, HTTP enable/bind/port, external web server, cache, progress, world enablement, and atomic invalid-config rejection.
4. Compare decoded RGBA for base tiles and full pyramid across missing chunks, negative/region boundaries, north/south shading, visibility, fluids/glass, modded descriptors, atomic replacement.
5. Exercise HTTP GET and HEAD for index/static, settings, players, world settings, markers, icons, existing/missing PNG, ETag/304, cache-control, traversal, dev frontend, and disabled listener/socket ownership.
6. Rust is sole writer for its root; roots and ports never overlap.

**Exit:** complete hash-addressed corpus and HTTP inventory pass with exact RGBA/semantic JSON/icon equality and zero ignored mismatches. This remains deterministic evidence, not live proof.

## Phase 4 — Production quiescence and live shadow

Wire `QuiescenceProbe` to production readiness, session/ack watermark, queues, jobs/cancel state, dirty/replay rows, output inventory/hash/mtime, temp files, recorder flush, HTTP ownership, lifecycle, and metric availability. Require two equal samples over a quiet interval; missing observations fail.

Drive real Java-primary/Rust-shadow scenarios: startup; positive/negative/region-boundary place-break-mutate; affected and neighboring shading tiles; players/privacy; markers/icons/addons; reload/world epoch; full/radius/pause/resume/cancel/reset; saturation; sidecar crash/restart; crash-before-ready; shutdown. Capture event/frame/epoch provenance and before/after hashes. Assert stale-epoch rejection, exactly-once controls, durable checkpoints, no late writes/temp files, bounded queues, isolated roots/ports.

**Exit:** zero mismatches except predeclared matrix-row-linked volatile fields or deliberate divergences, with raw evidence preserved; zero lost dirty work from a real Paper process. Post-hoc explanations and fixtures cannot pass. Saturation resync must converge without bridge teardown.

## Phase 5 — Loader and platform parity

Run Paper full matrix, Fabric lifecycle/events, modded NeoForge descriptor/render, and Sponge lifecycle smoke. Verify enable/disable/reload/repeated restart, listener/task/network registration, API services, commands/permissions, disabled HTTP, clean process/thread/port shutdown, safe scheduler use, bounded tick/event work, credit-limited snapshot copy, and off-thread compression/socket I/O.

**Exit:** all four pinned loader bundles pass with transcripts, hashes, PIDs, ownership, and clean shutdown evidence.

## Phase 6 — Paper fault, recovery, and rollback proof

Kill Rust while Paper remains alive; verify capped backoff/restart budget, authenticated replacement, watermark/replay, config replay before readiness, convergence, and no two writers. Test token/version mismatch, corrupt state, output-lock and HTTP-bind conflicts, missing binary, crash loops, shutdown during recovery, and restart rollback to Java. Before cutover, prove idempotent import while preserving `dirty_chunks.json`/`resume_render.json` untouched. Do not create final `.migrated-v1` backups until the observation rollback window completes.

**Exit:** real Paper evidence proves bounded recovery and restart-selected rollback. Automatic live failover remains forbidden.

## Phase 7 — Isolated performance and resource gate

Run fresh Java-primary and Rust-primary pinned Paper twins, linked to the same Phase 4/5 action and quiescence transcript hashes, with fixed warmup and at least three measured runs plus confidence intervals. Collect tick p50/p95/p99, snapshot copy, render throughput, dirty-to-visible p50/p95/p99, HTTP throughput/latency, heap/GC, Rust and combined Paper+sidecar RSS from process-boundary sampling, CPU/disk bytes, startup/shutdown/recovery, queue high-water/convergence, compression ratio. Standalone renderer/unit/build-tool runs cannot satisfy this phase.

Threshold formulas compare Rust-primary confidence bounds with Java-primary: Rust throughput lower bound >= Java lower bound; HTTP and dirty-visible p95 regression upper bound <=10%; tick p95 regression upper bound <=5%; combined steady-state Paper+sidecar RSS regression upper bound <=15%; all queues converge. Predeclare units, CI method, warmup, samples, and threshold precedence.

## Phase 8 — Native release and verified execution

Build and publish Linux x86_64/aarch64, Windows x86_64, macOS x86_64/aarch64 assets. Independently fetch every immutable URL; verify name/size/hash/execution/target rejection on each target host. Close verification-to-execution TOCTOU by executing verified bytes/descriptor or an atomic immutable handoff. Wire the production configured-path -> verified cache -> verified download resolution flow; test offline cache/miss, symlink/traversal/permissions, partial/corrupt download, and readiness only after recovery/HTTP ownership.

**Exit:** remote evidence passes all five target hosts. Local manifest generation is insufficient. The exact immutable URL/digest is mandatory in canary and observation manifests and verified at every start/restart.

## Phase 9 — Opt-in primary canary

Before execution, freeze immutable canary and observation manifests: duration, cohort/server denominator, scenarios, evidence inventory, severity policy, thresholds, rollback triggers, and exact release URL/digest. Transfer ownership through an explicit writer lease/lock handoff with monotonic timestamps and continuous overlap detection: disable Java writer/supervisor generation, drain and quiesce, release Java root/port, acquire Rust root lock, then permit Rust writes/listener. Any overlap fails. Keep Java restart rollback in a separate root; never automatic live failover. Run complete cohorts, rollback drills, upgrades/restarts/state migration, and diagnostics.

**Exit:** the predeclared canary window and denominator complete without unresolved P0/P1 parity, loss, recovery, ownership, or rollback defect; every process generation executed the Phase 8 immutable digest.

## Phase 10 — Rust-default observation release

Make Rust default only in the release after canary. Keep tested restart rollback for one full observation release. Predefine duration, cohort denominator, severity policy, metrics availability, rollback triggers, and final rollback drill.

## Phase 11 — Clean cutover

**Pre-delete gate:** rerun the full matrix on the observation commit, including remote artifacts, loaders, live parity, performance, recovery, rollback, quiescence, and sole-writer ownership. After the observation rollback window and final Rust checkpoint, atomically create `.migrated-v1` backups; verify rollback is no longer required before deleting readers.

Delete via symbol-aware migration: Java `IntegratedServer`, `JsonCache`, `ViteRunner`, Undertow; `RenderManager` and production `task/render/*`; Java tile encoding and render persistence; `LegacyBackendController`, `BackendMode`, shadow-only roots/ports; obsolete cache/state publishers/update tasks; migrated-file readers after backups; obsolete backend responsibilities in `MapWorldInternal`; duplicate config/docs/tests/dependencies. Retain loader/API/privacy/snapshot/supervision code. Migrate every caller; no shims or permanent dual backend.

**Post-delete gate:** rerun the full matrix—not a smoke subset—on the deletion commit: Cargo/Gradle/web, protocol/comparator, four-loader lifecycle, real Paper live/recovery/quiescence/ownership, full performance, clean-start/upgrade, and remote publish/fetch/execute for the final package. Symbol audit finds no Java HTTP listener, cache/publisher, render scheduler/pixel renderer/tile encoder, or dirty/render persistence owner. Rust is sole backend/output/HTTP owner.

## Completion

Complete only when every matrix row has current-commit evidence, deterministic and live comparisons have zero ignored mismatches, every loader/recovery/performance/release/canary/observation gate passes, rollback was exercised, legacy Java backend code is deleted, and the post-delete suite passes. Missing external prerequisites block phases; they never narrow completion.
