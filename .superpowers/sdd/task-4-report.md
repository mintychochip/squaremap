# Task 4 report: ordered sessions and bounded coalescing outbox

## RED evidence

Java command:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*CoalescingOutboxTest'
```

Result: `BUILD FAILED` during `:squaremap-common:compileTestJava`; the new test had 47 unresolved-symbol errors for the not-yet-created `BridgeEvent`, `CoalescingOutbox`, and `BridgePublisher` types. This was genuine missing-type RED evidence.

Rust command:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test session_ordering
```

Result: `cargo test` failed because `crates/squaremap-server/src/session.rs` did not yet exist. This was genuine missing-module RED evidence.

## GREEN evidence

Java command:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*CoalescingOutboxTest'
```

Result: `BUILD SUCCESSFUL`; focused XML recorded 7 tests, 0 skipped, 0 failures, and 0 errors. The tests cover newest revision retention, latest replacement state, the 65,537-key world/epoch overflow marker and post-overflow suppression, sequence assignment, durable pending/ack behavior, stale acknowledgement protection, reconnect UUID/baseline resend, caller publication isolation, and worker shutdown.

Rust command:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test session_ordering
```

Result: `6 passed (1 suite, 11 warnings, 0.00s)`. The warnings are existing protocol-crate unused-import warnings; no formatter, linter, or project-wide suite was run.

## Implementation

- `BridgeEvent` is a sealed immutable model with public `WorldKey`, replacement state, dirty chunk, and world resync variants.
- `CoalescingOutbox` uses one ownership lock and deterministic `TreeMap` keys. It retains the highest revision per coordinate, latest replacement value per state key, and one resync marker per world/epoch. Dirty keys are capped at 65,536; overflow removes that world/epoch's queued dirties and records a marker. Repeated dirties covered by the marker cannot regrow the set.
- `BridgePublisher` performs only bounded in-memory work on `publish`, signals one owned daemon writer, assigns sequences during draining, tracks explicit in-flight identities, leaves current values available for reconnect resend, and removes only acknowledged in-flight values. A stale acknowledgement cannot erase a newer value. Overflow envelopes use `WorldResyncRequired` with `RESYNC_REASON_QUEUE_FULL`. Close is idempotent and joins the writer.
- Rust `SessionCursor` returns exact `New`, `Duplicate`, or latched `Gap { expected, actual }` decisions without sequence wrap ambiguity. `Session` validates the exact authenticated 16-byte ID, invokes a durable handler only for New, acknowledges only after handler success, returns duplicate Ack without re-invocation, and returns a fatal structured full-bootstrap replacement ProtocolError for wrong sessions/gaps. Handler failures restore the cursor so no event is prematurely acknowledged. `Session::authenticated` generates a fresh 16-byte reconnect ID.
- Server bootstrap and Task 3 framing/supervision remain unchanged apart from declaring the session module; no persistence, routing, HTTP, restart, or later-task behavior was added.

## Self-review

- Dirty cap is enforced in the outbox and publisher accounts for distinct in-flight dirty identities before admitting a new key. Overflow only intentionally discards the affected world's queued dirties and emits its marker.
- Revision and replacement coalescing are keyed deterministically. In-flight sequence/value identity is retained so an older Ack cannot remove a newer queued/current value. Current values remain eligible for reconnect resend, while acknowledged state does not count as pending until re-enqueued.
- Sequence numbers reset only with a new random 16-byte session ID. Drain checks exhaustion instead of wrapping. Publication callbacks run only on the owned writer thread, never on the caller thread; shutdown signals and joins that worker idempotently.
- Rust wrong-session, duplicate, gap latch, handler failure, and reconnect cases are all directly asserted. New events are not Acked until the supplied handler returns success. No server loop path fabricates durability or Ack behavior.
- The worktree was checked with `git diff --check`; only the Task 4 Java outbox/event/publisher files, focused tests, Rust session module/test, server module declaration, and this report are in scope.

Atomic task commit: this commit; SHA recorded in the progress ledger.

## Review-fix RED/GREEN evidence

Review-fix Java RED command:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*CoalescingOutboxTest'
```

Result: `BUILD FAILED` during test compilation with 20 expected missing-API errors for the new authenticated Ack, supplied-session reconnect, closeable Writer, failure accessor, and review regressions. The production implementation was then changed to satisfy those contracts.

Review-fix Rust RED command:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test session_ordering
```

Result: `cargo test` failed on the newly added `HandlerPanicked` assertion because panic-safe handler handling was not yet implemented.

Review-fix Java GREEN command:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*CoalescingOutboxTest'
```

Result: `BUILD SUCCESSFUL`; focused XML recorded 15 tests, 0 skipped, 0 failures, and 0 errors. Added regressions cover in-flight revision maxima, queued/in-flight union capacity, authenticated/status-filtered Acks, supplied-session reconnect barriers, writer failure requeue/cause, blocked close, resync recovery, bounded acknowledged dirty history, replacement protocol/correlation fields, monotonic drain sequencing, worker-only dispatch, and sequence exhaustion.

Review-fix Rust GREEN command:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test session_ordering
```

Result: `9 passed (1 suite, 15 warnings, 0.01s)`. Added cancellation/panic retry and RFC 4122 v4 UUID bit assertions; bootstrap Hello now masks the generated UUID to version 4 and RFC variant bits.

## Review-fix invariant review

- Java has one authoritative `current` identity map for queued/in-flight dirty values. It retains the maximum revision, counts each dirty identity once, removes acknowledged dirty/resync history, and keeps only replacement snapshots as reconnect baselines. A fresh post-resync dirty therefore cannot be erased by an acknowledged marker.
- Acks require an envelope from the current 16-byte session and only accepted/duplicate statuses. Unknown, old-session, rejected, unspecified, and duplicate/removed sequences are no-ops. Reconnect waits for the old generation's callback batch to finish before activating the new session and resetting sequence 1.
- The writer is a private single worker. Callback failures requeue canonical current values, expose the cause, close the publisher, and close the writer. Close invokes the unblocking writer contract and joins without a timeout; sequence capacity is preflighted before batch removal. Replacement envelopes force protocol 1/0 while retaining correlation and payload.
- Rust evaluates a cloned cursor candidate, commits only after a panic-safe durable handler returns success, latches gaps immediately, and leaves cancellation/panic retries eligible for application. Session IDs are exact 16-byte RFC 4122 v4 values in both the session generator and bootstrap Hello.

## Final follow-up RED evidence against c109663

The final regression tests were copied into a temporary detached checkout at parent commit `c109663`; the temporary checkout was removed after capture.

Java focused command with the final `CoalescingOutboxTest`:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*CoalescingOutboxTest'
```

Result: `BUILD FAILED` during `:squaremap-common:compileTestJava` with exactly 2 errors. The parent constructor accepted only `(byte[], Writer)`, while the final `unacknowledgedOverflowMarkerSuppressesPostOverflowDirtiesOnReconnect` and `replacementKeysHaveDeterministicBoundAndCloseIsExactlyOnce` tests supplied `(byte[], Writer, int, int)`. This is the expected RED for the newly required bounded publisher API.

To exercise the two runtime regressions against the unchanged parent implementation, a temporary `FinalReviewRedTest` used the same final test scenarios with the parent's public constructor and a default-capacity overflow. The focused command was:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*FinalReviewRedTest'
```

Result: `2 tests completed, 2 failed, 0 errors`. `reconnectFromWriterIsRejectedWithoutDeadlock()` failed because its 500 ms callback latch assertion observed `expected true but was false`; `unacknowledgedOverflowMarkerSuppressesPostOverflowDirties()` failed with `expected <0> but was <101>` canonical dirty values after the marker.

Rust focused command against the same parent checkout:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test session_ordering
```

Result: `10 tests`: 9 passed and 1 failed, `synchronously_panicking_handler_does_not_consume_sequence`, which propagated `synchronous handler panic` from the handler invocation.

These failures were captured before restoring the implementation worktree to the final branch.

## Final follow-up GREEN evidence

Java command:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*CoalescingOutboxTest'
```

Result: `BUILD SUCCESSFUL`; focused XML recorded 18 tests, 0 skipped, 0 failures, and 0 errors. The final regressions cover writer-thread reconnect rejection without deadlock, suppression of post-overflow dirties across reconnect, deterministic replacement-key bounds, and exactly-once writer close.

Rust command:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test session_ordering
```

Result: `10 passed (1 suite, 15 warnings, 0.01s)`. The added synchronous handler-panic case confirms that invoking the handler itself is panic-safe and leaves the sequence retryable.

## Final follow-up self-review

- `BridgePublisher.reconnect` rejects worker-thread re-entry before waiting on its own in-flight callback; external reconnects still wait for the generation barrier.
- The outbox and publisher both enforce the documented replacement-state bound before adding a new key. Repeated keys coalesce, while overflow is deterministic and does not grow either canonical map.
- The publisher checks the world/epoch resync marker before inserting later dirty identities, so reconnect cannot resurrect dirties covered by an unacknowledged marker. `Writer.close` is guarded by one ownership flag across callback failure and repeated external closes.
- Rust wraps handler invocation inside the panic-safe future, catches both synchronous invocation and asynchronous polling panics, and commits the cursor only after success.

Final validation also passed `git diff --check`; only the intended Task 4 implementation, focused tests, and report are modified.
