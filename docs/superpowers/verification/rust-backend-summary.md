# Rust backend migration verification

Current branch: `rust-backend-migration`

- `cargo test --manifest-path rust/Cargo.toml --workspace`: **272 tests passed across 34 suites** (fresh current run).
- `cargo test --manifest-path rust/crates/squaremap-state/Cargo.toml --test recovery`: **18 tests passed**.
- `cargo test --manifest-path rust/crates/squaremap-server/Cargo.toml --lib`: **30 tests passed**, including the production-linked pre-session dirty/identity rejection boundary.
- `cargo test --manifest-path rust/crates/squaremap-server/Cargo.toml --test dirty_resync_contract`: **7 tests passed**.
- `cargo check --manifest-path rust/Cargo.toml --workspace`: passed.
- `./gradlew :squaremap-common:test --no-daemon`: **passed**; full common test suite green.
- `./gradlew build --no-daemon`: **passed**; 83 actionable tasks completed with the full Gradle build green.
- `cargo test --manifest-path rust/crates/squaremap-compare/Cargo.toml --test replay -q`: **12 tests passed**.
- Rust bridge HTTP lifecycle tests: 2 passed.
- Rust configuration compatibility tests: 2 passed, including atomic rejection retaining the prior active revision.
- Web verification: `cd web && bun run lint && bun run build`; build passed and lint completed with three warnings.
- Comparator CLI produced `mismatch_count: 0` for the checked-in bridge view fixture roots across 6 paths; this validates independent fixture parity, not production shadow behavior. The narrower live-shadow artifact compared 2 captured paths with zero mismatches and explicitly does not cover full block/player/reload/fault parity.
- Renderer workload benchmark: passed at 10.8 K chunks/s with 0 allocations/chunk in the timed renderer path; excludes Scheduler, bridge, repository, and HTTP.
- Real Rust sidecar restart smoke: passed with supervisor-owned child termination, a distinct replacement connection, and SQLite repository state persisted.
- Backend manifest generation: **passed** locally for all five target entries; this does not establish release publication or remote URL resolution.
- Paper fixture validation: **passed** for the fail-closed metadata/placeholder contract; real Paper execution remains blocked by placeholder artifacts and unavailable production-equivalent runtime inputs.

## Fail-closed replay boundary

- Workflowz gate refresh: the four next recovery implementation gates remain blocked. Bootstrap still constructs a zero bridge identity and rejects `BridgeIdentityReplace` and `ChunkDirty`; no runtime branch consumes `ResumeWatermark` or `DirtyReplay*`; unsupported view payloads still return `Unsupported`, while identity replacement is rejected by the bootstrap gate; Java response dispatch filters replay messages; `bridge_checkpoints` stores checkpoint metadata only; dirty page/complete/defer remain global.
- Replay remains unwired because fresh sessions restart sequence numbering, no checkpoint continuity handshake exists, and `dirty_chunks` has no bridge owner or world-scoped replay cursor.
- Enabling replay without those contracts would permit cross-bridge or cross-world data exposure; Java remains the default backend.
- Current source audit found no new stable identity persistence, reconnect watermark negotiation, durable dirty owner, or production replay dispatcher. A fresh workflowz refresh was attempted but cancelled after stalling without producing an independent disposition; the prior workflowz review remains the available external review evidence. No recovery gate is safe to advance from the current source.
- Historical completion-plan checkboxes remain intentionally unchecked where their acceptance scope exceeds current evidence. The checked-in comparator and shadow artifacts satisfy only their documented narrow scopes; they do not satisfy full replay parity, Paper lifecycle shadowing, fault/replay recovery, isolated end-to-end performance, release publication, or clean cutover.

## Workflowz ownership disposition

The scoped workflowz review confirmed that no complete bridge-owned dirty persistence contract is source-supported yet. Before implementation, the plan must specify:

- The durable owner relation (owner column versus owner table), assignment, reclaim, and lease semantics.
- Authenticated `BridgeIdentityReplace` negotiation and reconnect ordering.
- Durable watermark/session continuity and replay request/item/completion semantics.
- Atomic boundaries for dirty write plus checkpoint and owner-scoped page/complete/defer operations, including lease, idempotency, and expiry rules.
- Cross-bridge and stale-epoch isolation rules.
- Pagination, ordering, retry bounds, and focused integration tests.

No partial replay or sentinel owner is safe.

## Not yet passed

- Full independent Java-primary/Rust-shadow comparison across block/chunk churn, player/marker changes, reload, quiescence, and sidecar fault while Paper remains alive.
- Paper production sidecar fault/reconnect/replay evidence.
- Isolated end-to-end performance and RSS evidence; the current benchmark excludes Scheduler, bridge, repository, and HTTP.
- Remote five-target release publication and asset URL resolution.
- Rust observation-release default and clean deletion of legacy Java backend paths.
- Full replay parity: exact RGBA tile comparison and semantic JSON coverage beyond the checked two-path artifact.

The migration is not complete until every item above has direct current-commit evidence.
