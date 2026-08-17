# Bridge-owned dirty persistence — fresh workflowz review

**Reviewer:** workflowz (external contract review)
**Status:** review unavailable for this session
**Scope:** bridge-owned dirty persistence contract in `rust-backend-migration`
**Authoritative source:** this document plus `docs/superpowers/specs/2026-08-15-bridge-owned-dirty-persistence-spec.md`

## Workflowz disposition

A live workflowz review could not be obtained for this worktree. The prior 2026-08-14 workflowz audit is therefore treated as non-current evidence.

A source audit by this session found the following contract gates are required before the migration can claim that bridge-owned dirty persistence is production-ready:

1. **Durable owner relation** — `dirty_chunks` owner columns, `bridge_checkpoints` table, assignment/reclaim/lease semantics, owner-scoped page/complete/defer.
2. **Authenticated identity/reconnect ordering** — stable Java-persisted `bridge_id`, Rust validation, reconnect reassigns all owned rows to the fresh `session_id` and sets `replay_pending` before replay.
3. **Durable watermark/replay** — `ResumeWatermark` after identity negotiation; `DirtyReplayRequest` with an explicit `start` state and a keyset cursor; bounded, world-scoped, owner-scoped, `replay_pending=1` query.
4. **Atomicity** — dirty write and checkpoint in one transaction; reassign in one transaction; complete/defer are owner-scoped.
5. **Cross-bridge and stale-epoch isolation** — rows are never shared between bridge IDs; stale-epoch rows are not replayed or rendered.
6. **Pagination, ordering, and retry** — pages bounded at 1024 items, deterministic `ORDER BY x,z` keyset pagination, `dirty_page_for_owner` must include `replay_pending=0`, bounded retry backoff.

These six items are the acceptance criteria for the implementation plan. Each must be covered by a focused test that would have failed before the implementation and must pass after it.
