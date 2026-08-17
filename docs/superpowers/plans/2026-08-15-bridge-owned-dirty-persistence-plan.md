# Bridge-owned dirty persistence implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the workflowz contract gaps in bridge-owned dirty persistence: keyset-cursor replay, reconnect reassign, bounded replay query, and `replay_pending=0` for the scheduler page.

**Architecture:** Extend `bridge.proto` with a oneof cursor and `has_more`/`last_coordinate` in the completion. Add `Repository::dirty_replay_page` with a bounded keyset query. Call `Repository::reassign_bridge_lease` from the `BridgeIdentityReplace` handler. Update Rust dispatch and Java `BridgeBackendController` to request/emit pages. Add multi-page and mid-replay-mutation tests.

**Tech Stack:** Java 25, JUnit 5, Protobuf, Rust 2024, Tokio, SQLite, Gradle, Cargo.

## Global Constraints

- Work only in `/home/jlo/dev/squaremap/.worktrees/rust-backend-migration`.
- Do not switch the default backend from Java.
- Do not delete legacy Java backend paths.
- Every production behavior starts with a failing focused test.
- Java remains the default until replay, live-shadow, fault, and isolated-performance gates all pass.

---

### Task 1: Extend the protocol for keyset-cursor replay

**Files:**
- Modify: `protocol/squaremap/bridge/v1/bridge.proto`

**Interfaces:**
- `DirtyReplayRequest` carries `oneof cursor { ReplayStart start = 7; ChunkCoordinate continuation = 8; }`.
- `DirtyResyncComplete` carries `bool has_more = 8; ChunkCoordinate last_coordinate = 9;`.
- `ReplayStart` is an empty message defined before `DirtyReplayRequest`.

- [ ] **Step 1: Add the messages and fields**

Insert before `DirtyReplayRequest`:

```proto
// ReplayStart is the first-page cursor. It carries no state; the page begins
// at the first owned dirty row for the world.
message ReplayStart {}
```

Append to `DirtyReplayRequest`:

```proto
  oneof cursor {
    ReplayStart start = 7;
    ChunkCoordinate continuation = 8;
  }
```

Append to `DirtyResyncComplete`:

```proto
  bool has_more = 8;
  ChunkCoordinate last_coordinate = 9;
```

- [ ] **Step 2: Regenerate generated types**

Run:

```bash
./gradlew :squaremap-common:generateProto --no-daemon
cargo check --manifest-path rust/Cargo.toml --workspace
```

Expected: Prost/Java types now have `start`, `continuation`, `has_more`, and `last_coordinate` accessors.

- [ ] **Step 3: Commit**

```bash
git add protocol/squaremap/bridge/v1/bridge.proto
git commit -m "protocol: keyset cursor and has-more for dirty replay"
```

---

### Task 2: Implement bounded, owner-scoped, replay-pending replay page query

**Files:**
- Modify: `rust/crates/squaremap-state/src/repository.rs`
- Modify: `rust/crates/squaremap-state/src/lib.rs` (re-export `DirtyRow` if not already)

**Interfaces:**
- Add `Repository::dirty_replay_page(&self, bridge_id, session_id, world, cursor, limit) -> Result<Vec<DirtyRow>, RepositoryError>`.
- The query joins `worlds`, filters by `owner_bridge_id`, `owner_session_id`, `replay_pending=1`, the world, and a keyset cursor.
- It returns at most `limit + 1` rows so the caller can detect `has_more` without an extra count query.

- [ ] **Step 1: Write a failing Rust contract test for multi-page replay with mutation**

In `rust/crates/squaremap-server/tests/dirty_resync_contract.rs`, add:

```rust
#[test]
fn replay_cursor_pagination_with_mutation_between_pages() {
    // TODO: set up a Repository with 3 dirty rows, call dirty_replay_page with start,
    // insert a new row with x between the first and second page, then call with
    // the continuation. The second page must not duplicate the first page and must
    // include all rows with x > last_x or (x == last_x and z > last_z).
    assert!(false); // force failure until implemented
}
```

Run:

```bash
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test dirty_resync_contract
```

Expected: compile fails because `dirty_replay_page` does not exist, or the new test fails.

- [ ] **Step 2: Add the repository method**

Add to `rust/crates/squaremap-state/src/repository.rs` after `dirty_rows_for_replay`:

```rust
pub async fn dirty_replay_page(
    &self,
    bridge_id: &[u8],
    session_id: &[u8],
    world: &WorldId,
    cursor: Option<&ChunkCoordinate>,
    limit: usize,
) -> Result<Vec<DirtyRow>, RepositoryError> {
    if bridge_id.len() != MAX_BRIDGE_ID_BYTES || bridge_id.iter().all(|byte| *byte == 0) {
        return Err(ModelError::Bounds("bridge ID must be exactly 16 non-zero bytes").into());
    }
    if session_id.len() != MAX_SESSION_ID_BYTES || session_id.iter().all(|byte| *byte == 0) {
        return Err(ModelError::Bounds("session ID must be exactly 16 non-zero bytes").into());
    }
    world.validate()?;
    if limit == 0 {
        return Err(ModelError::Bounds("limit must be positive").into());
    }
    let bridge_id = bridge_id.to_vec();
    let session_id = session_id.to_vec();
    let world = world.clone();
    let (cursor_x, cursor_z) = cursor.map(|c| (Some(c.x), Some(c.z))).unwrap_or((None, None));
    // SQLite binding: Option<i64> None -> SQL NULL.
    let cursor_x = cursor_x.map(i64::from);
    let cursor_z = cursor_z.map(i64::from);
    let limit = i64::try_from(limit.checked_add(1).unwrap_or(usize::MAX)).unwrap_or(i64::MAX);
    self.blocking(move |connection| {
        let mut statement = connection.prepare(
            "SELECT dirty_chunks.namespace,dirty_chunks.value,dirty_chunks.epoch,dirty_chunks.x,dirty_chunks.z,dirty_chunks.revision
             FROM dirty_chunks
             INNER JOIN worlds USING(namespace,value)
             WHERE worlds.epoch >= 0 AND dirty_chunks.epoch=worlds.epoch
               AND dirty_chunks.owner_bridge_id=?1
               AND dirty_chunks.owner_session_id=?2
               AND dirty_chunks.replay_pending=1
               AND dirty_chunks.namespace=?3 AND dirty_chunks.value=?4 AND dirty_chunks.epoch=?5
               AND (?6 IS NULL OR (dirty_chunks.x > ?6 OR (dirty_chunks.x = ?6 AND dirty_chunks.z > ?7)))
             ORDER BY dirty_chunks.x, dirty_chunks.z
             LIMIT ?8",
        )?;
        let rows = statement.query_map(
            params![
                bridge_id,
                session_id,
                world.namespace,
                world.value,
                world.epoch as i64,
                cursor_x,
                cursor_z,
                limit,
            ],
            |row| {
                let world = WorldId::new(
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    checked_u64(row.get::<_, i64>(2)?, "dirty epoch").map_err(|_| rusqlite::Error::InvalidQuery)?,
                );
                world.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
                Ok(DirtyRow {
                    world,
                    coordinate: ChunkCoordinate { x: row.get(3)?, z: row.get(4)? },
                    revision: checked_u64(row.get(5)?, "dirty revision").map_err(|_| rusqlite::Error::InvalidQuery)?,
                })
            },
        )?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }).await
}
```

- [ ] **Step 3: Fix `dirty_page_for_owner` to require `replay_pending=0`**

In the `WHERE` clause of `dirty_page_for_owner` add:

```sql
AND dirty_chunks.replay_pending=0
```

- [ ] **Step 4: Update `reassign_bridge_lease` to not return rows (avoid unbounded load)**

Change the signature to `pub async fn reassign_bridge_lease(&self, bridge_id: &[u8], session_id: &[u8]) -> Result<(), RepositoryError>` and remove the `let rows = self.dirty_rows_for_replay(bridge_id).await?;` line. Update `owner_lease.rs` tests to query separately.

- [ ] **Step 5: Run the new repository tests**

```bash
cargo test --manifest-path rust/Cargo.toml -p squaremap-state --test owner_lease -q
cargo test --manifest-path rust/Cargo.toml -p squaremap-state --test recovery -q
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add rust/crates/squaremap-state
git commit -m "state: keyset replay page, replay_pending guard, reassign returns unit"
```

---

### Task 3: Wire reconnect reassign and Rust dirty replay dispatch

**Files:**
- Modify: `rust/crates/squaremap-server/src/bootstrap.rs`
- Modify: `rust/crates/squaremap-server/src/dirty_resync.rs`

**Interfaces:**
- `BridgeIdentityReplace` handler calls `Repository::reassign_bridge_lease` before returning the accepted ack.
- `dispatch_replay_request` uses `dirty_replay_page`, handles `limit + 1` detection, emits `DirtyResyncComplete` with `has_more` and `last_coordinate`.
- `dirty_resync::validate_request` validates the oneof cursor and `max_items`.
- `dirty_resync::validate_completion` validates `has_more` and `last_coordinate` consistency.

- [ ] **Step 1: Write a failing integration test for reconnect reassign**

In `rust/crates/squaremap-server/tests/dirty_resync_contract.rs`, add a test that creates dirty rows, reconnects with a new session, and asserts `dirty_replay_page` with the new `session_id` returns those rows. Run it and confirm it fails because `reassign` is not yet wired.

- [ ] **Step 2: Update `dirty_resync.rs` validation**

Modify `validate_request` to require the `cursor` oneof to be set and to reject `continuation` coordinates that do not match the page bounds. Modify `validate_completion` to check that `has_more` implies `last_coordinate` is present and `item_count == request.max_items`, and that `!has_more` implies no `last_coordinate`.

- [ ] **Step 3: Wire `reassign_bridge_lease` in `bootstrap.rs`**

After the `BridgeIdentityReplace` handler in `run_bridge` receives an accepted ack and sets `stable_bridge_id`, call:

```rust
repository.reassign_bridge_lease(bridge_id, &session_id).await
    .map_err(|error| BootstrapError::Rejected(error.to_string()))?;
```

- [ ] **Step 4: Update `dispatch_replay_request` in `bootstrap.rs`**

Replace the `dirty_rows_for_replay` call with:

```rust
let cursor = match request.cursor {
    Some(DirtyReplayRequestCursor::Continuation(coord)) => Some(coord),
    _ => None,
};
let mut rows = repository
    .dirty_replay_page(bridge_id, session_id, &world, cursor.as_ref(), request.max_items as usize)
    .await
    .map_err(|error| BootstrapError::Rejected(error.to_string()))?;
let has_more = rows.len() > request.max_items as usize;
if has_more {
    rows.pop();
}
```

Emit items with page-relative indices. After the loop, build `DirtyResyncComplete` with `has_more` and `last_coordinate` set to the last emitted row when `has_more` is true.

- [ ] **Step 5: Run focused Rust tests**

```bash
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test dirty_resync_contract -q
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --lib -q
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add rust/crates/squaremap-server/src/bootstrap.rs rust/crates/squaremap-server/src/dirty_resync.rs rust/crates/squaremap-server/tests/dirty_resync_contract.rs
git commit -m "server: wire reconnect reassign and keyset replay dispatch"
```

---

### Task 4: Update Java multi-page replay

**Files:**
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/BridgeBackendController.java`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/backend/BridgeBackendControllerReplayTest.java`

**Interfaces:**
- `onResumeWatermark` sends a `DirtyReplayRequest` with `start` cursor and `start_item_index = 0`.
- `onDirtyResyncComplete` removes state when `has_more` is false; when true, it sends a new `DirtyReplayRequest` with `continuation` set to `last_coordinate` and a fresh `replay_id`.
- `ReplayState` tracks `startIndex` and `nextIndex` (page-relative).

- [ ] **Step 1: Write a failing Java multi-page replay test**

In `BridgeBackendControllerReplayTest`, add a test where the recording connection returns a full page (item_count == MAX_REPLAY_ITEMS, has_more true, last_coordinate set) and assert a second `DirtyReplayRequest` is published with the continuation cursor.

- [ ] **Step 2: Update `BridgeBackendController`**

Modify `onResumeWatermark` to build `DirtyReplayRequest` with `setStart(ReplayStart.getDefaultInstance())`.

Modify `ReplayState` to add `final int startIndex;` and initialize `nextIndex` to `0`.

Modify `onDirtyResyncComplete`:

```java
if (complete.getHasMore()) {
    final int nextStart = state.nextIndex.get();
    final long nextReplayId = this.nextReplayId.incrementAndGet();
    final DirtyReplayRequest nextRequest = DirtyReplayRequest.newBuilder()
        .setBridgeId(ByteString.copyFrom(state.bridgeId))
        .setSessionId(ByteString.copyFrom(state.sessionId))
        .setConfigRevision(state.configRevision)
        .setReplayId(nextReplayId)
        .setWorld(state.world)
        .setMaxItems(MAX_REPLAY_ITEMS)
        .setContinuation(complete.getLastCoordinate())
        .build();
    this.replayStates.put(
        new ReplayKey(state.configRevision, nextReplayId),
        new ReplayState(state.configRevision, nextReplayId, state.bridgeId, state.sessionId, state.world, nextStart)
    );
    connection.publish(new BridgeEvent.Transient(Envelope.newBuilder().setDirtyReplayRequest(nextRequest).build()));
} else {
    this.replayStates.remove(key);
}
```

- [ ] **Step 3: Run Java replay tests**

```bash
./gradlew :squaremap-common:test --tests '*BridgeBackendControllerReplayTest' --no-daemon --console=plain
```

Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add common/src/main/java/xyz/jpenilla/squaremap/common/backend/BridgeBackendController.java common/src/test/java/xyz/jpenilla/squaremap/common/backend/BridgeBackendControllerReplayTest.java
git commit -m "common: multi-page dirty replay with keyset continuation"
```

---

### Task 5: Clean compiler warnings and run full verification

**Files:**
- Modify: `rust/crates/squaremap-server/src/dirty_resync.rs`
- Modify: `rust/crates/squaremap-server/src/scheduler/live.rs`
- Modify: `rust/crates/squaremap-server/src/config.rs`
- Modify: `rust/crates/squaremap-server/src/session.rs`
- Modify: `rust/crates/squaremap-server/src/bootstrap.rs`
- Modify: `rust/crates/squaremap-compare/src/evidence.rs`

**Interfaces:**
- Remove or use unused imports and dead functions to eliminate `cargo check` warnings in the contract path.

- [ ] **Step 1: Fix unused imports and dead code**

In `dirty_resync.rs`, remove `WorldEnumerationStatus` from the import if no longer used. In `scheduler/live.rs`, remove unused `WorldEnumerationComplete` and `WorldEnumerationItem`. In `config.rs`, remove unused `Point` and `VisibilityLimit` and the unused `valid` function. In `session.rs`, mark or remove unused methods (`accept`, `authenticated`, `bridge_id`, `cursor_mut`, `accept_sequence`, `random_session_id`). In `bootstrap.rs`, remove or use `handle_control_request`. In `evidence.rs`, remove the unused `PathBuf` import.

- [ ] **Step 2: Run full Rust and Java suites**

```bash
cargo test --manifest-path rust/Cargo.toml --workspace -q
./gradlew :squaremap-common:test --no-daemon --console=plain
```

Expected: all pass.

- [ ] **Step 3: Run Gradle build and web build**

```bash
./gradlew build --no-daemon --console=plain
cd web && bun run lint && bun run build
```

Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: clean compiler warnings after dirty persistence contract"
```

---

### Task 6: Update completion disposition and finish

**Files:**
- Modify: `docs/superpowers/verification/rust-backend-completion-disposition.md`
- Modify: `.superpowers/sdd/progress.md`

- [ ] **Step 1: Record current evidence**

Add a section to `rust-backend-completion-disposition.md` listing:

- `cargo test --workspace` 290 passed / 35 suites
- `cargo test -p squaremap-state --test owner_lease` 8 passed
- `cargo test -p squaremap-server --test dirty_resync_contract` N passed (multi-page tests added)
- `BridgeBackendControllerReplayTest` passes
- `./gradlew build` and `bun run lint && bun run build` pass

- [ ] **Step 2: Mark plan complete and finish branch**

Use `superpowers:finishing-a-development-branch` to verify tests and present merge/cleanup options.
