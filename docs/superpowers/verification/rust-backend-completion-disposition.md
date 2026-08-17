# Rust Backend Migration — Completion Disposition

**Date:** 2026-08-15
**Branch:** `rust-backend-migration` (uncommitted migration worktree)
**Objective:** develop the checked-in migration plan to completion against its locked acceptance gates.

## Current state (2026-08-16)

| Gate | Status |
|---|---|
| `cargo test --workspace` | **319 passed / 41 suites** (current) |
| `cargo check --workspace --tests` | **passed**; 2 existing unused-import warnings |
| `cargo build --release --workspace` | **passed** (current) |
| `./gradlew :squaremap-common:test --no-daemon --console=plain` | **BUILD SUCCESSFUL** (current) |
| `./gradlew :squaremap-common:compileJava :squaremap-common:compileTestJava` | **BUILD SUCCESSFUL** (current) |
| Native module compilation (`paper`, `fabric`, `neoforge`, `sponge`) | **BUILD SUCCESSFUL** (current) |
| Web lint/build | **passed**; 3 existing non-fatal oxlint diagnostics and one unresolved runtime image reference |
| Focused Java ConfigReplace rejection test | **passed**; matching retained replacement is discarded and `INVALID_CONFIG` completes |
| Rust loopback replay bootstrap | **passed**; watermark, bounded replay items, completion, ACK, ordering, and fatal unsolicited-payload rejection |
| Java replay/controller suite | **BUILD SUCCESSFUL** (current); Java non-empty-world continuation still requires Paper-backed world fixture |
| Checked-in Java/Rust output comparator | **0 mismatches across 6 paths**; report `/tmp/squaremap-parity-report.json` |
| Checked-in comparator benchmark | **passed**; 2,687.78 items/s over 3 iterations, threshold 100 items/s |
| Checked-in corpus integrity | **passed**; parity report, corpus inventory, and world fixture SHA-256 match recorded deterministic evidence |
| Rust CLI help smoke | **passed**; bridge and fixture-server commands are exposed |
| Rust HTTP contract suite | **11 passed**; confinement, atomic writes, headers, disabled mode, symlink rejection, and missing-tile behavior |
| Real Rust sidecar config/restart smoke | **BUILD SUCCESSFUL**; authenticated sidecar accepts ConfigReplace, emits Ready/ACK, persists SQLite state, and reconnects after forced process termination |
| Existing scoped live shadow comparison | **0 mismatches across 2 paths**; Paper 26.2-112 startup/churn/quiescence evidence exists, but scope explicitly excludes complete block/player/reload/fault parity |
| Comparator fixture tests | **historical 25 passed; checked-in narrow parity remains zero-mismatch evidence only** |
| Native target manifest | local generation/resolution only; no remote publication |
| `squaremap-compare gate --evidence .../rust-backend-completion-disposition.md` | failed; missing JSON evidence manifest |
| `:squaremap-paper:rustShadowSmoke` | blocked before launch by placeholder Paper/plugin/sidecar fixture artifacts and missing non-empty world corpus inventory |
| Paper lifecycle proof | not executed; production-equivalent Paper/plugin artifacts and complete world corpus unavailable |

## Verdict: migration INCOMPLETE

The implementation now has a tested bridge-owned dirty-state foundation and production bootstrap routing for authenticated identity, `ChunkDirty` persistence, and bounded dirty replay. The migration is still not complete because the plan requires external lifecycle, release, and cutover evidence that cannot be inferred from unit/build results.

## Completed in this worktree

1. `dirty_chunks` owner identity, session, lease expiry, and replay-pending state are schema-backed.
2. Owner-scoped assignment, expiry reclaim, pagination/order, completion, deferral, retry cleanup, stale-epoch isolation, and reconnect lease reassignment are covered by focused tests.
3. `BridgeIdentityReplace` is validated against the persisted identity, durably checkpointed with the accepted sequence, activated without resetting the session cursor, and reconnect lease reassignment is performed before owner scheduling.
4. Production `ChunkDirty` envelopes route through `Session::process_with_repository`; generic view replacement no longer silently handles dirty events.
5. The owner scheduler leases and completes/defers only rows assigned to its bridge identity.
6. `DirtyReplayRequest` is routed through `Session::process`, advances the inbound session sequence, and emits `DirtyReplayItem`/`DirtyResyncComplete`/Ack; inbound `DirtyReplayItem`/`DirtyResyncComplete` is treated as a fatal protocol violation.
7. `session_ordering` now proves three consecutive new payloads are accepted and acked (replay N+1 continuity).
8. Java-side `ResumeWatermark` / `DirtyReplayRequest` / `DirtyReplayItem` / `DirtyResyncComplete` / `WorldResyncRequired` dispatch is wired in `BridgeBackendController` via `setReplayListener(this::dispatch)`, with `ReplayState` keyed on `(configRevision, replayId)`, replay-scoped owner/session validation, contiguous index enforcement, and `MapWorldInternal.chunkModified` application; replay state is cleared on connection failure/replacement, `abortForRestart`, and `close`; `hasMore` continuation `DirtyReplayRequest` is dispatched.
9. Reload ownership is intentionally split: Java consumes `Reload`, performs the Java reload, and emits the resulting `ConfigReplace`; Rust must not receive raw `Reload`. The Java boundary test `bridgeReloadUsesJavaReloadPath` proves no bridge correlation is allocated, while Rust config-reload tests cover the resulting `ConfigReplace` HTTP listener reuse/handoff and bind-failure preservation. Rust's baseline `Reload` fallback is not parity coverage.

## Remaining acceptance gates

1. **~~Reconnect watermark negotiation and bounded replay protocol~~** — *completed at the unit/component level; still requires a real Rust sidecar reconnect to prove convergence end-to-end.*
2. **Full replay parity:** exact decoded-RGBA tile and semantic JSON equality over the complete recording corpus, with no ignore list beyond explicitly volatile fields.
3. **Live Java-primary/Rust-shadow comparison:** block/chunk churn, player/marker changes, reload, quiescence, and sidecar fault while Paper remains alive; zero mismatches after quiescence.
4. **Paper production sidecar fault/reconnect/replay proof:** in-lifecycle kill, authenticated restart, bounded replay, and final convergence using pinned production-equivalent artifacts.
5. **Isolated end-to-end performance and RSS:** scheduler, bridge, repository, HTTP, and full lifecycle must be measured; the renderer benchmark alone is insufficient.
6. **Release publication and remote asset URL resolution:** local five-target manifest generation exists, but no release upload or independently verified remote URLs were exercised.
7. **Rust observation-release default:** remains Java-default until gates 1–6 pass.
8. **Clean cutover:** legacy Java classes remain present (`IntegratedServer`, `JsonCache`, `ViteRunner`, `RenderManager`, `LegacyBackendController`, `BackendMode`, and related paths); deletion is prohibited before gates 2–6 pass.

## Required next execution order

```
A. Finish runtime ResumeWatermark/DirtyReplay dispatch and Java reconnect integration.  [DONE in this session]
B. Run full parity corpus, live shadow matrix, Paper fault/reconnect proof, and isolated E2E/RSS.
C. Publish all native targets and independently verify manifest URLs/hashes.
D. Add Rust-default upgrade/rollback evidence.
E. Delete Java legacy paths via symbol-aware migration, then rerun cargo/Gradle/web plus all lifecycle gates.
```

## Evidence policy

- Do not mark a gate passed without current-commit evidence matching its full scope.
- Do not enable Rust default or delete legacy paths while lifecycle, performance, or release gates remain unproven.
- Do not fabricate Paper, remote release, full replay, or end-to-end performance evidence.
- Local unit/component/build evidence is not Paper lifecycle proof.
