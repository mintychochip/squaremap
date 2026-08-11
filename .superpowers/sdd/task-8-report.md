# Task 8 — Backend-neutral commands and configuration

## RED evidence

Focused RED checks were captured before each remediation: the initial response/failure ownership edit failed Java compilation; Rust controls initially returned fake success; config staging mutated state before session acceptance; exporter tests exposed missing visibility/config values; timeout tests showed that queued cancellation did not cover writer-batch extraction; and config rejection tests showed a late revision could complete a newer pending config. Command handlers also had silent non-success defaults. These failures drove the final fence, revision correlation, geometry validation, and localized outcome mapper.

## GREEN evidence

- `./gradlew :squaremap-common:test --tests '*BackendControllerTest' :squaremap-paper:compileJava :squaremap-fabric:compileJava :squaremap-neoforge:compileJava :squaremap-sponge:compileJava` — BUILD SUCCESSFUL (30 actionable tasks).
- `./gradlew :squaremap-common:test --tests '*BackendControllerTest' --tests '*CoalescingOutboxTest' --tests '*BackendCommandMessagesTest'` — BUILD SUCCESSFUL; includes the deterministic parked-batch fence test, late config rejection test, publisher writer-failure tests, and player/console sender-visible unavailable/timeout/failure assertions.
- `cargo test -p squaremap-server config::` — 4 passed, including reversed/zero-width rectangle rejection with active-state retention.
- `SQUAREMAP_OUTPUT_ROOT=/tmp/squaremap-test cargo test -p squaremap-server bootstrap::` — 7 passed.
- `cargo test -p squaremap-server control::` — 5 passed.
- `cargo test -p squaremap-protocol` — 22 passed.
- `cargo check -p squaremap-server` — successful (pre-existing warnings only).
- `git diff --check` — no whitespace errors.

## Changed behavior

- Added the backend-neutral `BackendController` facade with seven `CompletionStage<BackendResult>` control methods and one immutable sealed request hierarchy. JAVA invokes only legacy behavior, RUST invokes only the bridge, and SHADOW executes both while returning the Java result.
- Migrated every map-control command caller away from direct `RenderManager` access. Every stable backend failure (`UNKNOWN_WORLD`, `INVALID_REQUEST`, `INVALID_CONFIG`, `BACKEND_UNAVAILABLE`, `BACKEND_TIMEOUT`, `FAILED`) now produces a localized sender-visible result for player and console paths.
- Added correlated bridge control requests with world epochs, bounded pending state, ten-second timeouts, exact-once completion, queued cancellation, and a deterministic session fence: if timeout wins after batch extraction/in-flight dispatch, the active sidecar session is closed before `BACKEND_TIMEOUT` is completed. Already-executed work is not claimed to be rolled back; late responses are discarded.
- Added `ProtocolError.config_revision`; Rust includes the rejected revision and Java completes only the matching pending configuration. Late rejection/policy for revision N cannot complete pending revision N+1.
- Added strict rectangle geometry validation (`min_x < max_x`, `min_z < max_z`) in Java shape construction and Rust staging, with validation-before-swap preserving the prior active configuration.
- Added strict Rust configuration staging with required groups/values, nested world and visibility validation, policy derivation from Java privacy/event settings, and atomic control-world replacement only after accepted session processing.
- Added independent inbound/outbound sequence validation, writer-failure propagation, policy-driven frame/outbox limits, and startup config publication through the backend controller response owner.
- RUST reload reparses Java configuration without a second startup publication, then publishes one complete replacement snapshot and waits for the matching policy revision.

## Commit

Single atomic commit intent: `Route map control through a backend facade`. The final amended hash is reported by the parent after review; no intermediate remediation commit was created.

## Concerns

- Rust bootstrap emits a nonfatal protocol error for rejected config in addition to the normal session acknowledgement; Java maps only a matching nonzero `config_revision` to `INVALID_CONFIG`.
- A dispatched timeout terminates the active sidecar session before reporting `BACKEND_TIMEOUT`; the control may have executed before the fence, and this is intentionally modeled as a timeout rather than a rollback claim. Remaining pending requests receive `BACKEND_UNAVAILABLE` from session failure.
- Rust currently exposes typed control availability and config/policy handling; actual authoritative rendering remains Java until a Rust executor is introduced, so configured Rust controls intentionally return `BACKEND_UNAVAILABLE`.
