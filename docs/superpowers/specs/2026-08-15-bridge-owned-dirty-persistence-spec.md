# Bridge-owned dirty persistence specification

## Goal

Define the contract that lets a managed Rust sidecar own durable dirty-chunk state, reconnect replay, and owner-scoped render scheduling while Java remains the default backend.

## Architecture

- Java persists a stable 16-byte `bridge_id` in the server data directory (`bridge-identity.bin`) and publishes it to Rust via `BridgeIdentityReplace`.
- Rust stores one `bridge_checkpoints` row per bridge (`bridge_id` primary key, `session_id`, `durable_sequence`).
- `dirty_chunks` carries `owner_bridge_id`, `owner_session_id`, `lease_expires_epoch_seconds`, and `replay_pending`.
- Reconnect reassigns every row owned by the bridge to the new session, sets `replay_pending=1`, and refreshes the checkpoint session without changing the durable watermark.
- After a `ResumeWatermark`, Java requests replay pages with a keyset cursor. Rust returns contiguous `DirtyReplayItem`s, a `DirtyResyncComplete` with `has_more` and `last_coordinate`, and uses a bounded SQL query.
- The scheduler's `dirty_page_for_owner` only sees `replay_pending=0` rows, so replay and live rendering cannot race for the same chunk.

## Global constraints

- Java remains the default backend.
- `bridge_id` and `session_id` are always exactly 16 bytes; a zero bridge identity is invalid.
- Replay pages are bounded at `MAX_REPLAY_ITEMS = 1024`.
- The cursor is a keyset over `(x, z)` and is never an `OFFSET`, so mid-replay `mark_dirty` inserts cannot shift or duplicate pages.
- Legacy or stale-epoch rows are never assigned an authenticated identity and are never replayed.
- Missing or malformed observations fail closed.

## Durable owner relation

- `dirty_chunks` has `owner_bridge_id`, `owner_session_id`, `lease_expires_epoch_seconds`, `replay_pending`.
- `bridge_checkpoints` has `bridge_id` primary key, `session_id`, `durable_sequence`.
- `mark_dirty` writes a dirty row and updates the bridge checkpoint in one transaction.
- `assign_dirty_lease` atomically pages one eligible row, sets the bridge as owner, marks it `replay_pending=1`, and assigns a lease expiry.
- `dirty_page_for_owner` returns only `replay_pending=0` unleased/unexpired rows ordered deterministically.
- `complete_dirty_for_owner` and `defer_dirty_for_owner` are scoped to the owning bridge.
- `reassign_bridge_lease` updates all rows owned by a bridge to a new session and marks them `replay_pending=1` in one transaction.

## Authenticated identity and reconnect ordering

- Java `SidecarSupervisor` loads or creates `bridge-identity.bin` before the first sidecar start.
- On a new authenticated connection, Java publishes `BridgeIdentityReplace` with the stable `bridge_id`.
- Rust validates length/non-zero, persists it in `bridge_checkpoints`, and then invokes `reassign_bridge_lease` for the fresh `session_id`.
- Only after identity is accepted and rows are reassigned may Rust send `ResumeWatermark`.

## Watermark and replay

- `ResumeWatermark` carries `bridge_id`, `session_id`, `config_revision`, `last_durable_sequence`.
- `DirtyReplayRequest` carries a `oneof` cursor:
  - `start` (empty `ReplayStart`) for the first page.
  - `continuation` (`ChunkCoordinate`) for the next page, where the coordinate is the `last_coordinate` from the previous `DirtyResyncComplete`.
- `DirtyResyncComplete` carries `has_more` and `last_coordinate` (populated only when `has_more` is true).
- The replay query for a page is bounded:
  ```sql
  SELECT ... FROM dirty_chunks
  INNER JOIN worlds USING(namespace,value)
  WHERE worlds.epoch >= 0 AND dirty_chunks.epoch=worlds.epoch
    AND dirty_chunks.owner_bridge_id=?
    AND dirty_chunks.owner_session_id=?
    AND dirty_chunks.replay_pending=1
    AND dirty_chunks.namespace=? AND dirty_chunks.value=? AND dirty_chunks.epoch=?
    AND (? IS NULL OR (dirty_chunks.x > ? OR (dirty_chunks.x = ? AND dirty_chunks.z > ?)))
  ORDER BY dirty_chunks.x, dirty_chunks.z
  LIMIT ?
  ```
  The `LIMIT` is `max_items + 1` so the caller can detect `has_more` without a count query.
- `DirtyReplayItem` indices are page-relative, contiguous from `0`, and duplicates/gaps are rejected.

## Atomicity

- Identity persistence and reassign are in the same database transaction? No — identity is persisted first; reassign is a separate transaction, and if it fails the connection closes. The next reconnect will re-run the idempotent reassign.
- `mark_dirty` + checkpoint update in one transaction.
- `complete_dirty_for_owner` and `defer_dirty_for_owner` each run in one transaction.

## Isolation

- `dirty_chunks` rows are owner-scoped by `bridge_id`.
- `complete/defer/page/replay` never touch another bridge's rows.
- Stale-epoch rows are filtered by the `INNER JOIN worlds` and `dirty_chunks.epoch=worlds.epoch` predicate.

## Pagination, ordering, and retry

- Pages are ordered by `(x, z)`.
- `MAX_REPLAY_ITEMS = 1024`.
- `dirty_page_for_owner` uses the same ordering and a `LIMIT`.
- `defer_dirty_for_owner` uses a bounded exponential backoff and a `dirty_retries` table.
- `dirty_page_for_owner` must include `replay_pending=0`.

## Acceptance criteria

| Gate | Evidence |
|---|---|
| Durable owner relation | `cargo test -p squaremap-state --test owner_lease` passes |
| Identity + reconnect | new test `cargo test -p squaremap-server --test dirty_resync_contract` covers reconnect reassign and multi-page replay |
| Watermark/replay | new `dirty_resync_contract` multi-page test with `mark_dirty` between pages passes |
| Atomicity | repository transaction tests in `owner_lease` and `recovery` pass |
| Isolation | cross-bridge and stale-epoch tests in `owner_lease` pass |
| Pagination/retry | `dirty_page_for_owner` and multi-page replay tests pass |
| Java integration | `BridgeBackendControllerReplayTest` multi-page test passes |
