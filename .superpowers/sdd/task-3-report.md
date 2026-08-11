# Task 3 report: launch and authenticate the managed sidecar

## RED evidence

Command:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*SidecarSupervisorTest'
```

Result: `BUILD FAILED` during `:squaremap-common:compileTestJava` because the newly added tests referenced the not-yet-created `SidecarSupervisor`, `BridgeConnection`, `BridgeBootstrapConfig`, `BackendMode`, and `SidecarCommand` types. The compiler reported 17 unresolved-symbol errors. This is the expected genuine RED for the process-level tests.

## GREEN evidence

Command:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*SidecarSupervisorTest'
```

Result: `BUILD SUCCESSFUL`; focused XML report recorded 7 tests, 0 skipped, 0 failures, and 0 errors:

- valid handshake/readiness and idempotent close
- token mismatch rejection and cleanup
- protocol-major mismatch rejection and cleanup
- readiness timeout cleanup
- bounded stderr capture
- child arguments contain only the required address/version metadata and no 32-byte base64 token
- single-use start/no restart after close

The test JVM completed without a fixture process remaining:

```text
pgrep -af 'xyz.jpenilla.squaremap.common.bridge.process.FakeSidecar' || true
```

Output: no matching process.

Rust focused command:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server
```

Result: `6 passed (1 suite, 0.00s)`.

Required CLI smoke:

```text
cargo run --manifest-path rust/Cargo.toml -p squaremap-server -- --help
```

Result: exit 0 and output:

```text
Usage: squaremap-server bridge --connect 127.0.0.1:<port> --plugin-version <version>
```

## Process and protocol implementation

- Java binds a loopback ephemeral `ServerSocketChannel` before starting the child, sends only the bridge connect address/plugin version as child arguments, writes one base64 bootstrap-token line to stdin, and closes stdin.
- Java validates the first control `Hello`, protocol major 1, 16-byte session ID, and exact token bytes with `MessageDigest.isEqual`; it sends accepted/rejected `HelloAck` and tears down rejected handshakes.
- Java readiness defaults to 30 seconds; cleanup closes listener/socket/streams/executors and forcibly terminates failed children. Accepted connections send `Shutdown` during idempotent close, allow the 10-second grace period, then force termination.
- Child stderr is drained continuously into a 64 KiB bounded buffer; stdout is drained and discarded so either child stream cannot block the process.
- Rust accepts only loopback `SocketAddr`, bounds the stdin token line at 88 characters, requires exact 32-byte base64 decoding, uses zeroizing buffers, connects with a 30-second timeout, sends Hello major 1/minor 0 with a random 16-byte session ID, validates matching accepted HelloAck, and remains alive until Shutdown or EOF.
- Rust structured tracing fields are limited to `session_id`, `protocol_major`, and `plugin_version`; token bytes are not logged.
- `SquaremapCommon` starts the supervisor only for SHADOW/RUST, leaves JAVA behavior unchanged, keeps SHADOW launch failure nonfatal, propagates RUST launch failure, and performs one close path without restart.

## Files

Created:

- `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BackendMode.java`
- `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeBootstrapConfig.java`
- `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeConnection.java`
- `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/SidecarCommand.java`
- `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/SidecarSupervisor.java`
- `common/src/testFixtures/java/xyz/jpenilla/squaremap/common/bridge/process/FakeSidecar.java`
- `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/process/SidecarSupervisorTest.java`
- `rust/crates/squaremap-server/Cargo.toml`
- `rust/crates/squaremap-server/src/main.rs`
- `rust/crates/squaremap-server/src/bootstrap.rs`

Modified:

- `common/build.gradle.kts` (fixture source wiring)
- `common/src/main/java/xyz/jpenilla/squaremap/common/SquaremapCommon.java` (mode-gated lifecycle only)
- `rust/Cargo.toml` (workspace member)
- `rust/Cargo.lock` (server dependency lock entries)

## Self-review

- Call sites and lifecycle ownership were checked after adding the supervisor and `SquaremapCommon` injection; JAVA remains the default injected mode and does not launch a process.
- Rejected handshakes, startup timeout, explicit close, and pre-start close all route through idempotent cleanup; no restart path is present.
- No JNI, gRPC, HTTP, persistence, event/session ordering, backend routing, binary resolution/download, or automatic restart behavior was added.
- The required CLI command and focused tests pass. Cargo emits an existing `squaremap-protocol` unused-import warning; no formatter, linter, or project-wide suite was run per task constraints.
- No unresolved concerns were found in the focused scope.


## Review-fix evidence

The lifecycle/secret-handling review fixes were developed test-first against commit `0ea0c3f`.

RED command:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*SidecarSupervisorTest'
```

With the new configured-grace regression test and the baseline supervisor, the focused suite reported `8 tests completed, 1 failed`; `configuredShutdownGraceBoundsForcedTermination()` failed with `AssertionFailedError` at `SidecarSupervisorTest.java:88`. This demonstrated the baseline hardcoded 10-second grace defect.

GREEN Java command:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*SidecarSupervisorTest'
```

Result: `BUILD SUCCESSFUL`; focused XML report recorded 9 tests, 0 skipped, 0 failures, and 0 errors. Added coverage includes configured shutdown grace, startup close/failure/no-restart, and `BridgeConnection.isClosed()` after teardown.

GREEN Rust command:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server
```

Result: `7 passed (1 suite, 0.00s)`, including secret Hello-writer zeroization on writer error.

CLI smoke:

```text
cargo run --manifest-path rust/Cargo.toml -p squaremap-server -- --help
```

Result: exit 0 and help listed `bridge`.

Leak check:

```text
pgrep -af 'xyz.jpenilla.squaremap.common.bridge.process.FakeSidecar' || true
```

Output: no matching process.

The review fixes register-or-close listeners and accepted sockets under the supervisor lock, make startup state transitions deterministic, prevent readiness completion after cleanup, propagate configured shutdown grace, and zeroize the stdin line, decoded token, Hello payload, and serialized Hello protobuf buffer on success and error paths.

## Executor-cleanup follow-up

RED command against `7eb8f57`:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*SidecarSupervisorTest'
```

Result: `BUILD FAILED`; `closeStopsOwnedSidecarThreads()` failed at `SidecarSupervisorTest.java:42` with `10 tests completed, 1 failed`. The behavior-level test observed live `squaremap-sidecar` threads remaining after connection close.

GREEN command:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*SidecarSupervisorTest'
```

Result: `BUILD SUCCESSFUL`; the focused suite passed all 10 tests, including bounded observation that owned sidecar threads disappear after close. Cleanup now shuts down both the scheduler and worker executor on its first cleanup path using idempotent `shutdownNow()` calls, safe even when cleanup runs from an owned executor thread.

Post-test process check:

```text
pgrep -af 'xyz.jpenilla.squaremap.common.bridge.process.FakeSidecar' || true
```

Output: no matching process.