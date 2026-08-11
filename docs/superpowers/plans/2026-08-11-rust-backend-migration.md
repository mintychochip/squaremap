# Rust Backend Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace squaremap's Java HTTP/render/state backend with a managed Rust sidecar while retaining a thin Java Minecraft bridge, the public Java addon API, every supported loader, and the existing web UI contract.

**Architecture:** Java launches and supervises a version-matched Rust child process, then sends versioned Protobuf events and credit-limited chunk snapshots over an authenticated loopback stream. Rust owns durable backend state, JSON/assets, rendering, PNG tiles, HTTP, metrics, and comparison tooling; Java keeps only Minecraft/loader access and API facades. Java and Rust run in isolated-output shadow mode before Rust becomes default, and the Java backend is deleted after one observation release.

**Tech Stack:** Java 25, Gradle Kotlin DSL, Protobuf Java, Rust 2024 edition with Tokio, Prost, Axum/Tower, rusqlite, zstd, png, Serde, tracing, and Clap; Bun/Vite UI remains unchanged.

## Global Constraints

- Treat `docs/superpowers/specs/2026-08-11-rust-backend-architecture-design.md` as authoritative.
- Preserve `tiles/settings.json`, `tiles/players.json`, `tiles/<world>/settings.json`, `tiles/<world>/markers.json`, registered icon paths, PNG tile paths, HTTP cache headers, and missing-PNG behavior.
- Keep the public `squaremap-api` source and binary contract; addons remain Java callers.
- Keep Paper, Fabric, NeoForge, and Sponge platform integration in Java.
- Use a child process, loopback TCP, length-delimited Protobuf, zstd only for chunk bodies, and no JNI or gRPC.
- Never perform compression, socket I/O, or unbounded queue growth on a Minecraft tick/region thread.
- `java`, `shadow`, and `rust` modes must use separate output roots; no two backends may write one tree.
- Java remains the default until replay, live-shadow, fault, and isolated-performance gates all pass.
- The clean-cutover task removes the Java backend and legacy mode; no permanent compatibility shim remains.
- Rust targets are `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`, and `aarch64-apple-darwin`.
- Rust edition is 2024 and MSRV is 1.88.0; commit `Cargo.lock` for binaries.
- Every production change is test-first and committed with its contract tests as one atomic commit.

## Locked file structure

| Path | Responsibility |
|---|---|
| `protocol/squaremap/bridge/v1/bridge.proto` | Sole cross-language wire schema |
| `rust/Cargo.toml` | Rust workspace and shared dependency versions |
| `rust/rust-toolchain.toml` | Rust 1.88.0 toolchain and components |
| `rust/crates/squaremap-protocol` | Generated Prost types, framing, limits, compression |
| `rust/crates/squaremap-state` | Canonical state and SQLite repository |
| `rust/crates/squaremap-render` | Minecraft-independent rendering and tile pyramid |
| `rust/crates/squaremap-server` | Bridge session, HTTP server, scheduler, process entrypoint |
| `rust/crates/squaremap-compare` | Recording replay, parity comparison, benchmark reports |
| `common/.../bridge/protocol` | Generated Java types plus Java framing adapter |
| `common/.../bridge/process` | Binary resolution, launch, handshake, supervision |
| `common/.../bridge/outbox` | Bounded/coalescing Java-to-Rust publication |
| `common/.../bridge/snapshot` | Registry descriptors and immutable chunk wire snapshots |
| `common/.../backend` | Backend-neutral command/API facade during migration |
| `testdata/bridge/v1` | Versioned protocol recordings and expected semantic output |

Do not create a generic `utils` crate or a second schema/model hierarchy. Domain types live in the crate that owns their invariants.

---

### Task 1: Establish the shared protocol and Rust workspace

**Files:**
- Create: `protocol/squaremap/bridge/v1/bridge.proto`
- Create: `rust/Cargo.toml`
- Create: `rust/rust-toolchain.toml`
- Create: `rust/crates/squaremap-protocol/Cargo.toml`
- Create: `rust/crates/squaremap-protocol/build.rs`
- Create: `rust/crates/squaremap-protocol/src/lib.rs`
- Create: `rust/crates/squaremap-protocol/tests/golden_envelope.rs`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/protocol/GoldenEnvelopeTest.java`
- Modify: `.gitignore`
- Modify: `gradle/libs.versions.toml`
- Modify: `common/build.gradle.kts`

**Interfaces:**
- Produces: Protobuf `squaremap.bridge.v1.Envelope` with protocol `major=1`, `minor=0`.
- Produces: Rust `squaremap_protocol::wire` generated module.
- Produces: Java package `xyz.jpenilla.squaremap.bridge.v1`.
- Produces: one identical golden `Hello` envelope encoded by Java and Rust.

- [ ] **Step 1: Write the failing Java golden-envelope test**

```java
@Test
void encodesV1HelloGoldenFrame() throws Exception {
    final Envelope envelope = Envelope.newBuilder()
        .setProtocolMajor(1)
        .setProtocolMinor(0)
        .setSessionId(ByteString.copyFromHex("00112233445566778899aabbccddeeff"))
        .setSequence(1)
        .setHello(Hello.newBuilder()
            .setPluginVersion("test")
            .setBootstrapToken(ByteString.copyFromUtf8("token")))
        .build();
    assertArrayEquals(
        Files.readAllBytes(Path.of("../testdata/bridge/v1/hello.bin")),
        envelope.toByteArray()
    );
}
```

- [ ] **Step 2: Run the Java test and verify the generated types are absent**

Run: `./gradlew :squaremap-common:test --tests '*GoldenEnvelopeTest'`

Expected: compilation fails because `Envelope` and `Hello` do not exist.

- [ ] **Step 3: Add Protobuf generation and the complete v1 envelope**

Use Java package `xyz.jpenilla.squaremap.bridge.v1`, Rust module `wire`, and this stable payload numbering:

```proto
syntax = "proto3";
package squaremap.bridge.v1;
option java_package = "xyz.jpenilla.squaremap.bridge.v1";
option java_multiple_files = true;

message Envelope {
  uint32 protocol_major = 1;
  uint32 protocol_minor = 2;
  bytes session_id = 3;       // exactly 16 bytes
  uint64 sequence = 4;        // starts at 1 per sender/session
  uint64 correlation_id = 5;  // zero means no request correlation
  oneof payload {
    Hello hello = 10;
    HelloAck hello_ack = 11;
    Ready ready = 12;
    Heartbeat heartbeat = 13;
    Ack ack = 14;
    ProtocolError protocol_error = 15;
    Shutdown shutdown = 16;
    ConfigReplace config_replace = 20;
    BridgePolicyReplace bridge_policy_replace = 21;
    RegistryReplace registry_replace = 22;
    WorldUpsert world_upsert = 30;
    WorldRemove world_remove = 31;
    WorldStateReplace world_state_replace = 32;
    PlayersReplace players_replace = 40;
    MarkerLayersReplace marker_layers_replace = 41;
    IconsReplace icons_replace = 42;
    ChunkDirty chunk_dirty = 50;
    WorldResyncRequired world_resync_required = 51;
    ChunkSnapshotRequest chunk_snapshot_request = 52;
    ChunkSnapshot chunk_snapshot = 53;
    ChunkMissing chunk_missing = 54;
    RenderCommand render_command = 60;
    RenderProgress render_progress = 61;
    RenderResult render_result = 62;
    BackendHealth backend_health = 70;
  }
}
```

Define every named payload in the same file now. Use explicit scalar fields rather than `google.protobuf.Any`; represent world identity as `{string namespace, string value, uint64 epoch}`; represent coordinates as signed `sint32`; represent UUIDs as 16-byte values; use enums with `UNSPECIFIED = 0`; never reuse a field number.

Configure `com.google.protobuf` Gradle generation for Java and add JUnit Jupiter. Configure `prost-build` with vendored `protoc` in `build.rs`; do not require host-installed `protoc`.

- [ ] **Step 4: Create the Rust workspace and matching golden test**

```rust
#[test]
fn encodes_v1_hello_golden_frame() {
    let envelope = Envelope {
        protocol_major: 1,
        protocol_minor: 0,
        session_id: hex::decode("00112233445566778899aabbccddeeff").unwrap(),
        sequence: 1,
        correlation_id: 0,
        payload: Some(envelope::Payload::Hello(Hello {
            plugin_version: "test".into(),
            bootstrap_token: b"token".to_vec(),
        })),
    };
    assert_eq!(envelope.encode_to_vec(), include_bytes!("../../../../testdata/bridge/v1/hello.bin"));
}
```

Generate `testdata/bridge/v1/hello.bin` once from the deterministic scalar-only message and check it in. Add `target/` and local backend binaries to `.gitignore`, not all `.bin` files.

- [ ] **Step 5: Run both protocol tests**

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-protocol --test golden_envelope`

Expected: 1 passed.

Run: `./gradlew :squaremap-common:test --tests '*GoldenEnvelopeTest'`

Expected: 1 passed.

- [ ] **Step 6: Commit the shared contract**

```bash
git add .gitignore protocol rust/Cargo.toml rust/rust-toolchain.toml rust/crates/squaremap-protocol testdata/bridge/v1/hello.bin gradle/libs.versions.toml common/build.gradle.kts common/src/test
git commit -m "Add the Rust bridge protocol contract"
```

---

### Task 2: Implement bounded cross-language framing

**Files:**
- Create: `rust/crates/squaremap-protocol/src/frame.rs`
- Create: `rust/crates/squaremap-protocol/src/limits.rs`
- Create: `rust/crates/squaremap-protocol/tests/frame_limits.rs`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/protocol/FrameCodec.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/protocol/FrameLimits.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/protocol/FrameCodecTest.java`
- Modify: `rust/crates/squaremap-protocol/src/lib.rs`

**Interfaces:**
- Produces Rust `FrameLimits { max_control_bytes: 1_048_576, max_snapshot_bytes: 67_108_864, max_uncompressed_snapshot_bytes: 134_217_728 }`.
- Produces Rust `read_envelope`/`write_envelope` over Tokio `AsyncRead`/`AsyncWrite`.
- Produces Java `FrameCodec.read(ReadableByteChannel)` and `FrameCodec.write(WritableByteChannel, Envelope)`.
- Frame layout: one-byte class (`0 = control`, `1 = chunk snapshot`), four-byte unsigned big-endian length, then exactly one Protobuf envelope.

- [ ] **Step 1: Write malformed-frame tests in both languages**

Cover unknown frame class, zero length, control frame over 1 MiB, snapshot frame over 64 MiB, class/payload mismatch, early EOF, trailing bytes, invalid Protobuf, snapshot decompression above 128 MiB, CRC mismatch, and a valid frame split across one-byte reads.

```rust
#[tokio::test]
async fn rejects_declared_control_frame_over_limit_before_allocating() {
    let mut bytes = Vec::from([FrameClass::Control as u8]);
    bytes.extend_from_slice(&1_048_577_u32.to_be_bytes());
    let err = read_envelope(&mut bytes.as_slice(), FrameLimits::default()).await.unwrap_err();
    assert!(matches!(err, FrameError::DeclaredLength { length: 1_048_577, .. }));
}
```

- [ ] **Step 2: Run the focused tests and verify failure**

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-protocol --test frame_limits`

Expected: fails because `read_envelope` does not exist.

Run: `./gradlew :squaremap-common:test --tests '*FrameCodecTest'`

Expected: compilation fails because `FrameCodec` does not exist.

- [ ] **Step 3: Implement the exact framing algorithm**

Rust must read the five-byte class/length prefix into a fixed stack array, select the class-specific limit, reject the length before allocating, use `BytesMut::zeroed(length)`, `read_exact`, then `Envelope::decode`. Java must use a five-byte `ByteBuffer`, reject with `ProtocolException` before `ByteBuffer.allocate(length)`, loop until each buffer is complete, and reject EOF with the expected/actual counts.

Require control class for every payload except `ChunkSnapshot`; require snapshot class for `ChunkSnapshot`; reject a class/payload mismatch after decode. Enforce compressed chunk length, declared uncompressed length, decompression ratio, and CRC again inside the snapshot decoder.

- [ ] **Step 4: Run focused framing tests**

Run the two commands from Step 2.

Expected: all Rust and Java framing cases pass.

- [ ] **Step 5: Commit framing and allocation limits**

```bash
git add rust/crates/squaremap-protocol common/src/main/java/xyz/jpenilla/squaremap/common/bridge/protocol common/src/test/java/xyz/jpenilla/squaremap/common/bridge/protocol
git commit -m "Enforce bounded bridge protocol frames"
```

---

### Task 3: Launch and authenticate the managed sidecar

**Files:**
- Create: `rust/crates/squaremap-server/Cargo.toml`
- Create: `rust/crates/squaremap-server/src/main.rs`
- Create: `rust/crates/squaremap-server/src/bootstrap.rs`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BackendMode.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeBootstrapConfig.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/SidecarCommand.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/SidecarSupervisor.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/process/SidecarSupervisorTest.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/SquaremapCommon.java:33-142`
- Modify: `rust/Cargo.toml`

**Interfaces:**
- Produces `BackendMode.JAVA`, `BackendMode.SHADOW`, and `BackendMode.RUST`.
- Produces `SidecarSupervisor.start(BridgeBootstrapConfig): CompletionStage<BridgeConnection>` and idempotent `close()`.
- Rust CLI: `squaremap-server bridge --connect 127.0.0.1:<port> --plugin-version <version>`; the 32-byte token is read as one base64 line from stdin.
- Readiness timeout is 30 seconds and shutdown grace is 10 seconds. Automatic restart is deliberately activated only with recovery tests in Task 15.

- [ ] **Step 1: Write a process-level handshake test**

Use a tiny Java fixture process under `common/src/testFixtures/java/.../FakeSidecar.java` that reads stdin, connects to the supplied loopback address, and emits either a valid or invalid `Hello`. Assert valid readiness, token mismatch rejection, protocol-major mismatch rejection, timeout cleanup, stderr capture, and no restart after explicit close.

- [ ] **Step 2: Run the test and verify failure**

Run: `./gradlew :squaremap-common:test --tests '*SidecarSupervisorTest'`

Expected: compilation fails because `SidecarSupervisor` is absent.

- [ ] **Step 3: Implement Java launch and supervision**

`SidecarSupervisor` must bind `ServerSocketChannel` to `InetAddress.getLoopbackAddress()` and port `0` before process launch, create a 32-byte token with `SecureRandom`, pass only address/version as arguments, write the base64 token to child stdin, and close stdin. Require the first valid frame to be `Hello` with the same token and protocol major `1`; compare token bytes with `MessageDigest.isEqual`.

Do not add restart behavior to `SquaremapCommon` yet. Start only when mode is `SHADOW` or `RUST`, and keep `JAVA` behavior byte-for-byte unchanged.

- [ ] **Step 4: Implement the Rust bootstrap client**

Parse only loopback `SocketAddr`, read exactly one token line with an 88-character cap, zero the token buffer after handshake, connect with a 30-second timeout, send `Hello`, require `HelloAck`, and exit non-zero on rejection. Emit structured tracing fields `session_id`, `protocol_major`, and `plugin_version`; never log the token.

- [ ] **Step 5: Run process tests and a real smoke handshake**

Run: `./gradlew :squaremap-common:test --tests '*SidecarSupervisorTest'`

Expected: all supervisor cases pass.

Run: `cargo run --manifest-path rust/Cargo.toml -p squaremap-server -- --help`

Expected: help lists the `bridge` subcommand and exits 0.

- [ ] **Step 6: Commit sidecar lifecycle**

```bash
git add rust/Cargo.toml rust/crates/squaremap-server common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process common/src/main/java/xyz/jpenilla/squaremap/common/SquaremapCommon.java common/src/test
git commit -m "Launch an authenticated Rust sidecar"
```

---

### Task 4: Add ordered sessions and a bounded coalescing outbox

**Files:**
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/outbox/BridgeEvent.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/outbox/CoalescingOutbox.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/outbox/BridgePublisher.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/outbox/CoalescingOutboxTest.java`
- Create: `rust/crates/squaremap-server/src/session.rs`
- Create: `rust/crates/squaremap-server/tests/session_ordering.rs`
- Modify: `rust/crates/squaremap-server/src/main.rs`

**Interfaces:**
- `BridgeEvent` sealed variants: `ReplaceState(key, Envelope)`, `DirtyChunk(WorldKey, epoch, x, z, revision)`, and `ResyncWorld(WorldKey, epoch)`.
- `CoalescingOutbox` caps unique dirty keys at 65,536, retains only the newest replace-state value, and converts overflow for one world into one `WorldResyncRequired`.
- `BridgePublisher.publish` is non-blocking and returns `ACCEPTED`, `COALESCED`, or `RESYNC_MARKED`.
- Rust `SessionCursor.accept(sequence)` returns `New`, `Duplicate`, or `Gap { expected, actual }`.

- [ ] **Step 1: Write coalescing and sequence tests**

Test 100 updates to one coordinate produce one newest revision; state replacement retains only the newest payload; 65,537 distinct chunks trigger one world resync; draining preserves sequence monotonicity; reconnect starts a new session UUID; duplicate sequences are acknowledged without reapplication; a gap requests full resync.

- [ ] **Step 2: Run focused tests and verify failure**

Run: `./gradlew :squaremap-common:test --tests '*CoalescingOutboxTest'`

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test session_ordering`

Expected: both fail on missing types.

- [ ] **Step 3: Implement bounded Java publication**

Use one lock around maps keyed by replace-state key and packed world/chunk key; do not use an unbounded executor. `publish` mutates only in-memory keys and signals one bridge writer thread. The writer assigns sequence numbers at drain time, writes frames, and removes acknowledged durable events only after `Ack`.

- [ ] **Step 4: Implement Rust ordering and resync decisions**

Reject wrong session IDs. Return `Ack` for a duplicate without invoking handlers. On a gap, stop applying subsequent events and return a structured `ProtocolError` requesting bootstrap replacement. Reset only after a new authenticated session.

- [ ] **Step 5: Run focused tests**

Expected: all coalescing and session-order cases pass.

- [ ] **Step 6: Commit event-flow invariants**

```bash
git add common/src/main/java/xyz/jpenilla/squaremap/common/bridge/outbox common/src/test/java/xyz/jpenilla/squaremap/common/bridge/outbox rust/crates/squaremap-server
git commit -m "Bound and order bridge event delivery"
```

---

### Task 5: Persist canonical Rust state and dirty work

**Files:**
- Create: `rust/crates/squaremap-state/Cargo.toml`
- Create: `rust/crates/squaremap-state/src/lib.rs`
- Create: `rust/crates/squaremap-state/src/model.rs`
- Create: `rust/crates/squaremap-state/src/repository.rs`
- Create: `rust/crates/squaremap-state/migrations/0001_initial.sql`
- Create: `rust/crates/squaremap-state/src/legacy_import.rs`
- Create: `rust/crates/squaremap-state/tests/recovery.rs`
- Modify: `rust/Cargo.toml`
- Modify: `rust/crates/squaremap-server/Cargo.toml`
- Modify: `rust/crates/squaremap-server/src/session.rs`

**Interfaces:**
- Produces `Repository::open(path)`, `apply_world`, `remove_world`, `mark_dirty`, `complete_dirty`, `create_render_job`, `update_render_job`, `recover`, and `import_legacy_state`.
- Durable identity is `(world_namespace, world_value, world_epoch, chunk_x, chunk_z)`.
- `mark_dirty` and update of the session checkpoint occur in one SQLite transaction before `Ack`.

- [ ] **Step 1: Write restart and idempotency tests**

Create a temporary database, apply duplicate dirty revision 7 twice, reopen, and assert one row at revision 7. Apply revision 6 afterward and assert it is ignored. Create a running render job, reopen, and assert recovery returns it as resumable. Remove/re-add a world with epoch 2 and assert epoch-1 dirty rows cannot reappear. Import valid, corrupt, and duplicate `dirty_chunks.json`/`resume_render.json` fixtures and assert imports are transactional and idempotent.

- [ ] **Step 2: Run the recovery test and verify failure**

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-state --test recovery`

Expected: fails because the crate/repository is absent.

- [ ] **Step 3: Add the explicit SQLite schema**

```sql
CREATE TABLE schema_version(version INTEGER PRIMARY KEY);
CREATE TABLE worlds(
  namespace TEXT NOT NULL,
  value TEXT NOT NULL,
  epoch INTEGER NOT NULL,
  config BLOB NOT NULL,
  PRIMARY KEY(namespace, value)
);
CREATE TABLE dirty_chunks(
  namespace TEXT NOT NULL,
  value TEXT NOT NULL,
  epoch INTEGER NOT NULL,
  x INTEGER NOT NULL,
  z INTEGER NOT NULL,
  revision INTEGER NOT NULL,
  PRIMARY KEY(namespace, value, epoch, x, z)
);
CREATE TABLE render_jobs(
  id BLOB PRIMARY KEY,
  namespace TEXT NOT NULL,
  value TEXT NOT NULL,
  epoch INTEGER NOT NULL,
  kind INTEGER NOT NULL,
  state INTEGER NOT NULL,
  payload BLOB NOT NULL,
  completed_chunks INTEGER NOT NULL
);
CREATE TABLE session_checkpoints(
  session_id BLOB PRIMARY KEY,
  durable_sequence INTEGER NOT NULL
);
CREATE TABLE legacy_imports(
  relative_path TEXT PRIMARY KEY,
  content_sha256 BLOB NOT NULL,
  imported_at_epoch_seconds INTEGER NOT NULL
);
INSERT INTO schema_version(version) VALUES (1);
```

Set WAL, `synchronous=NORMAL`, foreign keys, a five-second busy timeout, and SQLite application ID `0x53514D50` (`SQMP`). Reject a newer schema version; migrate older versions only through checked-in SQL.

- [ ] **Step 4: Implement transactional repository methods**

Use `INSERT ... ON CONFLICT ... DO UPDATE ... WHERE excluded.revision > dirty_chunks.revision`. Never hold a SQLite transaction across an await. Run blocking SQLite work through a bounded Tokio blocking pool owned by `squaremap-state`.

- [ ] **Step 5: Import legacy render state without breaking rollback**

Parse each world's existing `dirty_chunks.json` and `resume_render.json` with strict bounds, import in one transaction keyed by a content SHA-256, and record that hash so a restart cannot duplicate work. Through Java-primary, shadow, opt-in, and observation modes, treat these files as read-only because Java rollback still needs them. Task 21 renames them to `.migrated-v1` only after the fallback is removed. A corrupt file remains untouched, produces a path-specific error, and does not partially import.

- [ ] **Step 6: Run recovery tests**

Expected: all restart, duplicate, stale-revision, job, epoch, valid-import, corrupt-import, and repeated-import cases pass.

- [ ] **Step 7: Commit durable backend state**

```bash
git add rust/Cargo.toml rust/crates/squaremap-state rust/crates/squaremap-server
git commit -m "Persist Rust backend render state"
```

---

### Task 6: Reproduce the existing HTTP and filesystem contract

**Files:**
- Create: `rust/crates/squaremap-server/src/http/mod.rs`
- Create: `rust/crates/squaremap-server/src/http/static_files.rs`
- Create: `rust/crates/squaremap-server/src/http/cache.rs`
- Create: `rust/crates/squaremap-server/src/http/dev_frontend.rs`
- Create: `rust/crates/squaremap-server/src/output.rs`
- Create: `rust/crates/squaremap-server/tests/http_contract.rs`
- Create: `rust/crates/squaremap-server/tests/dev_frontend.rs`
- Modify: `rust/crates/squaremap-server/src/main.rs`
- Modify: `rust/crates/squaremap-server/Cargo.toml`

**Interfaces:**
- Produces `HttpServer::bind(HttpConfig, OutputRoot)` and graceful shutdown.
- Produces `OutputRoot::atomic_write(relative_path, bytes)` with root-confined paths.
- `/tiles/**` returns `Cache-Control: max-age=0, must-revalidate, no-cache`.
- Existing files expose strong last-modified-derived ETags and honor `If-None-Match` with 304.
- Missing `/tiles/**/*.png` returns status 200 with an empty body; every other missing path returns 404.
- Disabled HTTP mode performs no bind while output writes remain active.
- Development mode launches `bun run dev` in the configured frontend directory, extracts the loopback Vite URL, proxies HTTP and WebSocket/HMR traffic for every non-`/tiles` and non-registered-icon request, and terminates the Vite child during graceful shutdown.

- [ ] **Step 1: Write HTTP contract tests against a temporary output tree**

Assert index/static file bytes, JSON content type, ETag/304, tile cache-control, existing PNG bytes, missing PNG 200/empty, traversal rejection for plain and percent-encoded `..`, non-tile 404, disabled mode leaving the chosen port bindable, dev proxy exclusion for `/tiles` and `/images/icon/registered`, WebSocket upgrade tunneling, Vite early exit, and Vite child termination.

- [ ] **Step 2: Run the HTTP contract test and verify failure**

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test http_contract --test dev_frontend`

Expected: fails because `HttpServer` and `OutputRoot` are absent.

- [ ] **Step 3: Implement root-confined atomic output**

Validate every relative component before joining; reject absolute paths, prefixes, parent components, NUL, and symlink escapes. Write a sibling temporary file, flush it, then atomically rename. Keep one owner for output writes.

- [ ] **Step 4: Implement Axum/Tower routes, headers, and development proxy**

Route all GET/HEAD requests through the validated output root. Compute ETags from `(mtime nanos, length)`, quote them per HTTP syntax, and avoid reading a body for a matching `If-None-Match`. Set JSON and PNG content types explicitly. In development mode, spawn `bun run dev`, read merged stdout/stderr until a loopback URL appears or 15 seconds expires, proxy HTTP and WebSocket upgrades for only the same path set as current `IntegratedServer`, stream later Vite logs, and terminate the whole child tree on shutdown.

- [ ] **Step 5: Run HTTP tests and smoke the server**

Run the contract test; expected all cases pass.

Run: `cargo run --manifest-path rust/Cargo.toml -p squaremap-server -- serve-fixture --root web --bind 127.0.0.1:0`

Expected: process prints one `READY http_addr=127.0.0.1:<port>` line. Stop it after fetching `/` and one missing tile.

- [ ] **Step 6: Commit the Rust web-serving contract**

```bash
git add rust/crates/squaremap-server
git commit -m "Serve the squaremap web contract from Rust"
```

---

### Task 7: Mirror world, player, marker, and icon state into Rust

**Files:**
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/state/WorldStateExporter.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/state/PlayerStateExporter.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/state/MarkerStateExporter.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/state/IconStateExporter.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/state/BridgeStatePublisher.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/state/StateExporterTest.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/state/BridgeStatePublisherTest.java`
- Create: `rust/crates/squaremap-state/src/view.rs`
- Create: `rust/crates/squaremap-server/src/views.rs`
- Create: `rust/crates/squaremap-server/tests/view_contract.rs`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/task/UpdatePlayers.java:32-137`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/task/UpdateWorldData.java:28-162`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/task/UpdateMarkers.java:39-296`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/IconRegistry.java:19-91`
- Modify: `paper/src/main/java/xyz/jpenilla/squaremap/paper/SquaremapPaper.java:77-121`
- Modify: `fabric/src/main/java/xyz/jpenilla/squaremap/fabric/SquaremapFabric.java`
- Modify: `neoforge/src/main/java/xyz/jpenilla/squaremap/forge/SquaremapForge.java`
- Modify: `sponge/src/main/java/xyz/jpenilla/squaremap/sponge/SquaremapSponge.java`
- Modify: `paper/src/main/java/xyz/jpenilla/squaremap/paper/data/PaperMapWorld.java`
- Modify: `fabric/src/main/java/xyz/jpenilla/squaremap/fabric/data/FabricMapWorld.java`
- Modify: `neoforge/src/main/java/xyz/jpenilla/squaremap/forge/data/ForgeMapWorld.java`
- Modify: `sponge/src/main/java/xyz/jpenilla/squaremap/sponge/data/SpongeMapWorld.java`

**Interfaces:**
- Exporters return immutable protocol snapshots and contain no filesystem code.
- `PlayersReplace` contains already-filtered public players; Rust never receives hidden/private players.
- Marker geometry supports icon, circle, ellipse, rectangle, polyline, polygon, and multi-polygon with current options.
- Rust emits byte-compatible field names and semantically equivalent values for every existing JSON document.
- `BridgeStatePublisher` is the permanent Java event facade: platform-safe schedules call `publishWorlds`, `publishPlayers`, and `publishMarkers(world)`; it exports once and routes that immutable value to the active backend(s).

- [ ] **Step 1: Capture representative legacy JSON and write failing semantic tests**

Build fixed Java inputs for empty state, two worlds, a hidden player, health/armor, every marker shape, and an icon. Assert exporter protocol values. Store expected JSON under `testdata/bridge/v1/views/` and assert Rust output canonicalizes to those values.

- [ ] **Step 2: Run focused tests and verify failure**

Run: `./gradlew :squaremap-common:test --tests '*StateExporterTest'`

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test view_contract`

Expected: missing exporter/view implementations.

- [ ] **Step 3: Extract pure Java exporters without changing Java output**

Refactor each existing update task so its current JSON writer and `BridgeStatePublisher` consume the same immutable exporter result. In `JAVA`, route only to the legacy writer. In `SHADOW`, route one collected value to both. In `RUST`, route only to the bridge. Do not independently recollect players or marker registries, which would give A/B different inputs.

Replace platform scheduling targets with `BridgeStatePublisher` methods while preserving current 1-second player, 5-second world, and configured per-world marker intervals. Existing Java update tasks become legacy sinks rather than schedule owners, so Task 21 can delete them without removing event publication. `IconRegistry.register` keeps the public method contract but publishes normalized RGBA dimensions/bytes to the bridge; legacy Java mode continues current PNG writing.

- [ ] **Step 4: Generate Rust views from canonical state**

Use typed Serde structs with explicit `#[serde(rename = "...")]`; do not construct `serde_json::Value` maps throughout the code. Preserve omission/null behavior and collection ordering from the captured contract. Write JSON via `OutputRoot::atomic_write` and retain the latest bytes for HTTP ETags.

- [ ] **Step 5: Run Java and Rust state tests**

Expected: exporter and publisher tests pass; every Rust view matches canonical expected JSON; hidden player never appears in protocol or output; platform schedules invoke one collection per interval in every mode.

- [ ] **Step 6: Commit state mirroring**

```bash
git add common/src/main/java/xyz/jpenilla/squaremap/common/bridge/state common/src/main/java/xyz/jpenilla/squaremap/common/task common/src/main/java/xyz/jpenilla/squaremap/common/IconRegistry.java common/src/test paper/src/main/java/xyz/jpenilla/squaremap/paper fabric/src/main/java/xyz/jpenilla/squaremap/fabric neoforge/src/main/java/xyz/jpenilla/squaremap/forge sponge/src/main/java/xyz/jpenilla/squaremap/sponge rust/crates/squaremap-state rust/crates/squaremap-server testdata/bridge/v1/views
git commit -m "Mirror map view state into Rust"
```

---

### Task 8: Introduce backend-neutral commands and configuration

**Files:**
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/BackendController.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/LegacyBackendController.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/BridgeBackendController.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/BackendResult.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/backend/BackendControllerTest.java`
- Create: `rust/crates/squaremap-server/src/config.rs`
- Create: `rust/crates/squaremap-server/src/control.rs`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/config/Config.java:39-118`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/config/WorldConfig.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/command/commands/FullRenderCommand.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/command/commands/RadiusRenderCommand.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/command/commands/CancelRenderCommand.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/command/commands/PauseRenderCommand.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/command/commands/ResetMapCommand.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/command/commands/ReloadCommand.java`

**Interfaces:**
- `BackendController` methods: `fullRender`, `radiusRender`, `cancelRender`, `pauseRenders`, `resetMap`, `reload`, and `health`, each returning `CompletionStage<BackendResult>`.
- `BackendResult` contains a stable result-code enum and typed localization substitution values; Java remains responsible for localized messages.
- Initial direction is Java `ConfigReplace` -> Rust. Final direction is Rust `BridgePolicyReplace` -> Java for privacy/event policy and snapshot credits.

- [ ] **Step 1: Write command-routing and validation tests**

Assert `JAVA` invokes only `LegacyBackendController`, `SHADOW` executes control against Java and mirrors a non-authoritative request to Rust, and `RUST` invokes only `BridgeBackendController`. Assert unknown world, duplicate render, cancel-without-render, pause state, and reload validation map to existing message keys.

- [ ] **Step 2: Run the focused test and verify failure**

Run: `./gradlew :squaremap-common:test --tests '*BackendControllerTest'`

Expected: missing backend facade.

- [ ] **Step 3: Add the facade and migrate every command caller**

Inject `BackendController`; no command may reach `RenderManager` directly afterward. Use correlation IDs for Rust requests, complete pending futures exactly once, time out control requests after 10 seconds, and discard late responses without reusing IDs.

- [ ] **Step 4: Add strict Rust config/control handling**

Parse the current YAML-derived protocol values into typed structs, reject unknown enums and invalid bounds, and stage reload before swapping state. Return `BridgePolicyReplace` only after successful validation. Implement structured control results; do not send preformatted English strings from Rust.

- [ ] **Step 5: Run command tests and full Java compilation**

Run: `./gradlew :squaremap-common:test --tests '*BackendControllerTest' :squaremap-paper:compileJava :squaremap-fabric:compileJava :squaremap-neoforge:compileJava :squaremap-sponge:compileJava`

Expected: tests pass and all loader modules compile.

- [ ] **Step 6: Commit backend-neutral control**

```bash
git add common/src/main/java/xyz/jpenilla/squaremap/common/backend common/src/main/java/xyz/jpenilla/squaremap/common/command common/src/main/java/xyz/jpenilla/squaremap/common/config common/src/test rust/crates/squaremap-server
git commit -m "Route map control through a backend facade"
```

---

### Task 9: Export registry descriptors and compact chunk snapshots

**Files:**
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/snapshot/RegistryDescriptorExporter.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/snapshot/ChunkSnapshotEncoder.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/snapshot/SnapshotRequestService.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/snapshot/ChunkSnapshotEncoderTest.java`
- Create: `rust/crates/squaremap-render/Cargo.toml`
- Create: `rust/crates/squaremap-render/src/lib.rs`
- Create: `rust/crates/squaremap-render/src/snapshot.rs`
- Create: `rust/crates/squaremap-render/src/registry.rs`
- Create: `rust/crates/squaremap-render/tests/snapshot_decode.rs`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/util/chunksnapshot/ChunkSnapshot.java:24-85`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/data/BlockColors.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/data/LevelBiomeColorData.java`
- Modify: `rust/Cargo.toml`

**Interfaces:**
- `RegistryDescriptorExporter.snapshot()` returns stable session-local IDs for every block state, biome, and fluid descriptor used by rendering.
- `ChunkSnapshotEncoder.encode(ChunkSnapshot, worldEpoch, revision)` returns compressed payload, uncompressed length, and CRC32C.
- `SnapshotRequestService` allows at most 96 in-flight requests by default and returns `ChunkMissing` for absent/not-generated chunks.
- Rust `Snapshot::decode(&wire, &Registry, Limits)` validates before allocating section arrays.

- [ ] **Step 1: Write a cross-language snapshot fixture test**

Construct a snapshot with negative min Y, two palettes, packed indices crossing 64-bit boundaries, four biome cells, a world-surface heightmap, air-only sections, water, glass, and an unknown descriptor ID. Assert valid roundtrip and explicit rejection for unknown IDs, oversized section count, invalid packed width, CRC mismatch, and decompression bomb.

- [ ] **Step 2: Run Java/Rust tests and verify failure**

Run: `./gradlew :squaremap-common:test --tests '*ChunkSnapshotEncoderTest'`

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-render --test snapshot_decode`

Expected: missing encoder/decoder types.

- [ ] **Step 3: Export deterministic render descriptors**

Sort descriptors by Minecraft registry ID and full block-state property key before assigning session IDs. Include RGB map color, clear/invisible flag, glass alpha/tint, fluid kind, biome tint kind, and dimension-specific color inputs. Fail registry bootstrap if an ID used by a snapshot has no descriptor; do not invent a clear-color fallback.

- [ ] **Step 4: Encode palette-packed snapshots off-thread**

Copy Minecraft `PalettedContainer` values during the loader-safe snapshot stage, then pack IDs and compress on the bridge worker. Encode no Java class names, NBT blobs, block entities, or filesystem paths. Ensure cancellation and stale world epochs release buffers promptly.

- [ ] **Step 5: Decode with strict Rust limits**

Validate coordinate range, min/max Y, section count derived from height, palette lengths, bits-per-entry, exact packed-word count, heightmap length 256, descriptor existence, uncompressed length, compression ratio, and CRC before exposing `Snapshot`.

- [ ] **Step 6: Run cross-language snapshot tests**

Expected: the valid fixture decodes identically; every malformed case returns its named error; peak request count never exceeds 96.

- [ ] **Step 7: Commit portable snapshot transport**

```bash
git add common/src/main/java/xyz/jpenilla/squaremap/common/bridge/snapshot common/src/main/java/xyz/jpenilla/squaremap/common/util/chunksnapshot/ChunkSnapshot.java common/src/main/java/xyz/jpenilla/squaremap/common/data common/src/test rust/Cargo.toml rust/crates/squaremap-render testdata/bridge/v1
git commit -m "Send portable chunk snapshots to Rust"
```

---

### Task 10: Port render coordinates, visibility, and color primitives

**Files:**
- Create: `rust/crates/squaremap-render/src/coordinates.rs`
- Create: `rust/crates/squaremap-render/src/visibility.rs`
- Create: `rust/crates/squaremap-render/src/color.rs`
- Create: `rust/crates/squaremap-render/tests/render_primitives.rs`
- Reference: `common/src/main/java/xyz/jpenilla/squaremap/common/util/Numbers.java`
- Reference: `common/src/main/java/xyz/jpenilla/squaremap/common/visibilitylimit/`
- Reference: `common/src/main/java/xyz/jpenilla/squaremap/common/util/Colors.java`

**Interfaces:**
- Produces floor-correct block/chunk/region/tile conversion for negative coordinates.
- Produces `VisibilityLimit::contains_block` and `contains_chunk` for rectangle, circle, polygon, and world border.
- Produces exact ARGB mix, shade, depth-checkerboard, glass, and biome blend functions.

- [ ] **Step 1: Port Java behavior into table-driven failing tests**

Use boundary values `-513, -512, -511, -33, -32, -31, -17, -16, -15, -1, 0, 1, 15, 16, 17, 31, 32, 33, 511, 512, 513`. Store Java-produced expected coordinate/color values under `testdata/bridge/v1/render/primitives.json`; include polygon edge and vertex points.

- [ ] **Step 2: Run the primitive test and verify failure**

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-render --test render_primitives`

Expected: missing modules/functions.

- [ ] **Step 3: Implement integer and geometry invariants**

Use Euclidean division (`div_euclid`/`rem_euclid`), not truncating division. Use integer arithmetic for chunk/region/tile boundaries. Define polygon boundary points as visible to match Java behavior and include overflow-safe coordinate validation before multiplication.

- [ ] **Step 4: Implement exact color arithmetic**

Match Java channel extraction, truncation, alpha handling, checkerboard parity, and shading order before optimizing. Do not use SIMD until fixtures are exact. Keep color values as `u32` ARGB internally and convert once at PNG boundaries.

- [ ] **Step 5: Run primitive and property tests**

Add proptest invariants for coordinate roundtrips, visibility monotonicity, alpha bounds, and no panic over validated coordinates. Expected: table and property tests pass.

- [ ] **Step 6: Commit render primitives**

```bash
git add rust/crates/squaremap-render testdata/bridge/v1/render/primitives.json
git commit -m "Port map render primitives to Rust"
```

---

### Task 11: Port chunk-to-pixel rendering with exact parity

**Files:**
- Create: `rust/crates/squaremap-render/src/biome.rs`
- Create: `rust/crates/squaremap-render/src/chunk.rs`
- Create: `rust/crates/squaremap-render/src/region.rs`
- Create: `rust/crates/squaremap-render/tests/chunk_render.rs`
- Reference: `common/src/main/java/xyz/jpenilla/squaremap/common/task/render/AbstractRender.java:226-520`
- Reference: `common/src/main/java/xyz/jpenilla/squaremap/common/data/BiomeColors.java`

**Interfaces:**
- `render_chunk(RenderContext, north: Option<&Snapshot>, chunk: &Snapshot, south: Option<&Snapshot>) -> ChunkPixels`.
- `ChunkPixels` is exactly 16x16 ARGB plus the south-edge height state needed by adjacent shading.
- A missing north/south snapshot follows current Java reschedule/transparent behavior.

- [ ] **Step 1: Record failing Java-oracle chunk fixtures**

Record input snapshots and expected 16x16 ARGB for flat terrain, north/south height discontinuities, ceiling dimensions, iterate-up/down, max-height clipping, water/lava depth, clear and stained glass, invisible blocks, biome blend radii 0 and nonzero, missing neighbors, and empty chunks.

- [ ] **Step 2: Run the Rust chunk-render test and verify mismatch**

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-render --test chunk_render`

Expected: fails because `render_chunk` is absent.

- [ ] **Step 3: Implement surface selection and neighboring shading**

Port the loop order and `lastY[16]` semantics from `AbstractRender` exactly. Use world-surface height, configured max height, ceiling traversal, invisible/clear scanning, glass descent, fluid depth capped at 10, and north/south neighbor rows. Check cancellation between columns, not every individual arithmetic operation.

- [ ] **Step 4: Implement biome lookup and blending**

Resolve biome IDs from the snapshot's 4x4x4 palette, apply the descriptor tint category, and reproduce blend radius/order from Java fixtures. Cache immutable lookup tables per registry generation; do not share mutable per-render global state.

- [ ] **Step 5: Run exact pixel tests**

Expected: every fixture has 256/256 exact ARGB matches and malformed snapshots are rejected before rendering.

- [ ] **Step 6: Add a criterion benchmark without a release threshold yet**

Benchmark the fixed corpus and report chunks/second plus allocations/chunk. The benchmark establishes measurement plumbing; Task 16 sets comparative gates.

- [ ] **Step 7: Commit the Rust chunk renderer**

```bash
git add rust/crates/squaremap-render testdata/bridge/v1/render
git commit -m "Render map chunks in Rust"
```

---

### Task 12: Port the PNG tile pyramid and atomic image writes

**Files:**
- Create: `rust/crates/squaremap-render/src/tile.rs`
- Create: `rust/crates/squaremap-render/src/pyramid.rs`
- Create: `rust/crates/squaremap-render/src/png.rs`
- Create: `rust/crates/squaremap-render/tests/tile_pyramid.rs`
- Modify: `rust/crates/squaremap-server/src/output.rs`
- Reference: `common/src/main/java/xyz/jpenilla/squaremap/common/data/Image.java:26-161`

**Interfaces:**
- Base tiles are 512x512 RGBA.
- `TilePyramid::apply_region(region, pixels)` updates zoom 0 through configured max zoom and returns changed paths.
- Existing neighboring pixels survive partial updates; output comparison is decoded RGBA, not PNG bytes.

- [ ] **Step 1: Record and write failing pyramid fixtures**

Cover all four quadrants, negative tile coordinates, transparent pixels, overwrite of one 16x16 chunk inside an existing tile, zoom boundaries, compressed/uncompressed PNG settings, and concurrent updates to two regions feeding one parent tile.

- [ ] **Step 2: Run the pyramid test and verify failure**

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-render --test tile_pyramid`

Expected: missing `TilePyramid`.

- [ ] **Step 3: Implement tile composition and parent downsampling**

Use one keyed async mutex per destination tile, read existing PNG if present, replace only the affected rectangle, compute parent pixels in the same sample order as Java, encode RGBA8, and release the lock after atomic rename. Remove idle keyed locks so the lock map cannot grow forever.

- [ ] **Step 4: Verify crash-safe writes**

Inject failure before rename and assert the previous PNG remains valid. Inject two concurrent writers and assert the final decoded image contains both non-overlapping updates. Fsync the file before rename where supported; surface directory-fsync failure as a warning with the path.

- [ ] **Step 5: Run pyramid tests**

Expected: decoded RGBA exactly matches Java fixtures at every zoom and failure injection never leaves a truncated destination.

- [ ] **Step 6: Commit tile output**

```bash
git add rust/crates/squaremap-render rust/crates/squaremap-server/src/output.rs testdata/bridge/v1/render
git commit -m "Build map tile pyramids in Rust"
```

---

### Task 13: Move render scheduling and live dirty updates to Rust

**Files:**
- Create: `rust/crates/squaremap-server/src/scheduler/mod.rs`
- Create: `rust/crates/squaremap-server/src/scheduler/dirty.rs`
- Create: `rust/crates/squaremap-server/src/scheduler/jobs.rs`
- Create: `rust/crates/squaremap-server/src/snapshot_client.rs`
- Create: `rust/crates/squaremap-server/tests/render_recovery.rs`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/state/DirtyChunkPublisher.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/state/DirtyChunkPublisherTest.java`
- Modify: `paper/src/main/java/xyz/jpenilla/squaremap/paper/listener/MapUpdateListeners.java:50-234`
- Modify: `fabric/src/main/java/xyz/jpenilla/squaremap/fabric/listener/FabricMapUpdates.java:22-81`
- Modify: `neoforge/src/main/java/xyz/jpenilla/squaremap/forge/event/ForgeMapUpdates.java`
- Modify: `sponge/src/main/java/xyz/jpenilla/squaremap/sponge/listener/MapUpdateListener.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/BridgeBackendController.java`

**Interfaces:**
- Scheduler owns background, full, and radius jobs and snapshot credits.
- Default maximum active snapshots is 96; background interval comes from world config.
- Dirty completion occurs only after changed tile files are atomically installed.
- Full/radius progress is durable and resumes after sidecar restart.

- [ ] **Step 1: Write scheduler recovery and cancellation tests**

Use a fake snapshot bridge. Assert dirty coalescing, 96-request ceiling, retry of transient disconnect, permanent `ChunkMissing`, neighboring south dependency, pause, cancel, resume after reopening SQLite, world-epoch invalidation, and fair progress between two worlds.

- [ ] **Step 2: Run scheduler tests and verify failure**

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test render_recovery`

Expected: missing scheduler.

- [ ] **Step 3: Implement durable scheduling**

Select dirty work in bounded pages, request snapshots with a Tokio semaphore, and commit tile completion plus dirty-row removal as one logical completion. For full/radius jobs persist the coordinate cursor and completed count at bounded intervals and on shutdown. A canceled job cannot be resurrected by a late snapshot.

- [ ] **Step 4: Publish existing loader events to the bridge**

Refactor each listener's final `chunkModified` call through `DirtyChunkPublisher`. `JAVA` keeps existing behavior, `SHADOW` sends the same coordinate/revision to both backends, and `RUST` publishes only to Rust. Preserve Paper/Folia scheduler safety and current surface-height filtering.

- [ ] **Step 5: Run Rust tests and compile all loaders**

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test render_recovery`

Run: `./gradlew :squaremap-common:test --tests '*DirtyChunkPublisherTest' :squaremap-paper:compileJava :squaremap-fabric:compileJava :squaremap-neoforge:compileJava :squaremap-sponge:compileJava`

Expected: all pass.

- [ ] **Step 6: Perform a real Paper smoke scenario in Rust mode**

Launch the existing Paper development server, place/break one visible block, wait one configured background interval, and fetch the affected PNG. Expected: its decoded pixel hash changes once, player/settings endpoints remain valid, and logs show no Java render thread.

- [ ] **Step 7: Commit live Rust rendering**

```bash
git add rust/crates/squaremap-server common/src/main/java/xyz/jpenilla/squaremap/common/bridge/state common/src/main/java/xyz/jpenilla/squaremap/common/backend paper/src fabric/src neoforge/src sponge/src common/src/test
git commit -m "Drive live map rendering from Rust"
```

---

### Task 14: Add recording, replay, and semantic comparison tooling

**Files:**
- Create: `rust/crates/squaremap-compare/Cargo.toml`
- Create: `rust/crates/squaremap-compare/src/main.rs`
- Create: `rust/crates/squaremap-compare/src/recording.rs`
- Create: `rust/crates/squaremap-compare/src/compare.rs`
- Create: `rust/crates/squaremap-compare/src/report.rs`
- Create: `rust/crates/squaremap-compare/tests/replay.rs`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/recording/BridgeRecorder.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/recording/BridgeRecorderTest.java`
- Create: `testdata/bridge/v1/manifest.json`
- Create: `testdata/bridge/v1/reference.rec`
- Create: `testdata/bridge/v1/output/java/tiles/settings.json`
- Create: `testdata/bridge/v1/output/rust/tiles/settings.json`
- Create: `testdata/bridge/v1/output/java/tiles/players.json`
- Create: `testdata/bridge/v1/output/java/tiles/world/settings.json`
- Create: `testdata/bridge/v1/output/java/tiles/world/markers.json`
- Create: `testdata/bridge/v1/output/java/tiles/world/0/0.png`
- Create: `testdata/bridge/v1/output/java/images/icon/registered/test.png`
- Create: `testdata/bridge/v1/output/rust/tiles/players.json`
- Create: `testdata/bridge/v1/output/rust/tiles/world/settings.json`
- Create: `testdata/bridge/v1/output/rust/tiles/world/markers.json`
- Create: `testdata/bridge/v1/output/rust/tiles/world/0/0.png`
- Create: `testdata/bridge/v1/output/rust/images/icon/registered/test.png`
- Modify: `rust/Cargo.toml`

**Interfaces:**
- CLI subcommands: `replay`, `compare-output`, and `report`.
- Recording is a stream of `{u64 monotonic_nanos, u8 direction, u32 frame_length, frame_bytes}` plus manifest metadata.
- PNG comparison decodes to RGBA and reports first mismatch coordinate and total mismatches.
- JSON comparison canonicalizes object keys while preserving array order and numeric value semantics.

- [ ] **Step 1: Write failing replay/comparison tests**

Assert deterministic replay, truncated recording rejection, protocol-version rejection, exact RGBA equality despite different PNG compression, one-pixel mismatch location, JSON key-order equivalence, JSON array-order mismatch, missing/extra path reporting, and no overwrite of reports.

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-compare --test replay`

Expected: crate absent.

- [ ] **Step 3: Implement opt-in Java recording**

Record the exact post-coalescing outbound frames and inbound responses on a dedicated bounded writer. Recording is disabled by default, refuses paths outside the configured diagnostics root, redacts bootstrap tokens, and stops with a visible error rather than dropping frames silently. Produce the small checked-in `reference.rec` and both expected output trees from the deterministic fixture server; list every file and SHA-256 in `manifest.json`.

- [ ] **Step 4: Implement replay and comparison**

Replay timestamps with `--speed 0` for deterministic fastest execution or a positive multiplier. Produce JSON with schema version, commits, environment, compared path counts, mismatches, and timing. Exit `0` only on full equality, `1` on comparison mismatch, and `2` on invalid input/tool failure.

- [ ] **Step 5: Run tests and compare one Java/Rust fixture pair**

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-compare`

Run: `cargo run --manifest-path rust/Cargo.toml -p squaremap-compare -- compare-output --java testdata/bridge/v1/output/java --rust testdata/bridge/v1/output/rust --report build/ab/parity.json`

Expected: exit 0 and report `mismatch_count: 0`.

- [ ] **Step 6: Commit comparison tooling**

```bash
git add rust/Cargo.toml rust/crates/squaremap-compare common/src/main/java/xyz/jpenilla/squaremap/common/bridge/recording common/src/test testdata/bridge/v1
git commit -m "Compare Java and Rust map output"
```

---

### Task 15: Wire isolated shadow mode and fault-injection scenarios

**Files:**
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/BackendPaths.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/backend/BackendPathsTest.java`
- Create: `rust/crates/squaremap-server/tests/fault_recovery.rs`
- Create: `rust/crates/squaremap-compare/tests/live_shadow.rs`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/config/Config.java:69-79`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/SidecarSupervisor.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/SquaremapCommon.java:75-97`

**Interfaces:**
- `BackendPaths.resolve(mode, configuredRoot)` guarantees distinct canonical Java/Rust roots and rejects overlap/symlink aliasing.
- Shadow HTTP binds loopback only on port `0` unless an explicit diagnostics port is configured.
- Fault suite covers kill/restart, token rejection, sequence gap, queue saturation, SQLite busy/reopen, truncated output temp file, world reload, and shutdown during render.
- Sidecar restart delays are 1, 2, 4, 8, and 16 seconds with at most five starts per rolling ten minutes; explicit shutdown cancels pending restarts.

- [ ] **Step 1: Write path-isolation and fault tests**

Assert equal roots, parent/child roots, symlink aliases, and same HTTP port are rejected before either backend starts. In Rust tests, kill the session after durable dirty ACK and before tile completion, reconnect, and assert eventual exact output with one dirty completion.

- [ ] **Step 2: Run tests and verify failure**

Run: `./gradlew :squaremap-common:test --tests '*BackendPathsTest'`

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test fault_recovery`

Expected: missing isolation/recovery behavior.

- [ ] **Step 3: Enforce one-writer roots and shadow binding**

Canonicalize existing ancestors, resolve non-existing suffixes, and reject overlap in both Java and Rust. Print both assigned roots and ports once at startup. Activate the capped restart policy only after an authenticated session has existed; reset its failure counter after ten healthy minutes; cancel it on explicit shutdown. Never automatically promote shadow Rust to primary or point it at Java's root.

- [ ] **Step 4: Implement deterministic fault hooks under tests only**

Expose failpoints through Rust test features, not production HTTP endpoints. Verify state convergence, bounded queue high-water marks, no corrupt destination file, and no stale epoch output after every injected failure.

- [ ] **Step 5: Run fault tests and a live shadow comparison**

Run the focused tests, then a Paper shadow session with block churn, player join/leave, marker mutation, full-render cancel/resume, sidecar kill/restart, and world unload/load. Quiesce, run `compare-output`, and require `mismatch_count: 0`.

- [ ] **Step 6: Commit safe shadow mode**

```bash
git add common/src/main/java/xyz/jpenilla/squaremap/common/backend common/src/main/java/xyz/jpenilla/squaremap/common/config/Config.java common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/SidecarSupervisor.java common/src/main/java/xyz/jpenilla/squaremap/common/SquaremapCommon.java common/src/test rust/crates/squaremap-server rust/crates/squaremap-compare
git commit -m "Run Rust safely beside the Java backend"
```

---

### Task 16: Add benchmark metrics and enforce A/B gates

**Files:**
- Create: `rust/crates/squaremap-server/src/metrics.rs`
- Create: `rust/crates/squaremap-compare/src/benchmark.rs`
- Create: `rust/crates/squaremap-compare/tests/benchmark_report.rs`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/metrics/BridgeMetrics.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/metrics/BridgeMetricsTest.java`
- Modify: `rust/crates/squaremap-compare/src/main.rs`
- Modify: `testdata/bridge/v1/manifest.json`
- Create: `testdata/bridge/v1/benchmark/java-command.json`
- Create: `testdata/bridge/v1/benchmark/rust-command.json`
- Create: `testdata/bridge/v1/benchmark/synthetic-pass.json`
- Create: `testdata/bridge/v1/benchmark/synthetic-fail.json`

**Interfaces:**
- CLI subcommands: `benchmark` and `gate`.
- Report schema records commit, platform, CPU, JVM, config hash, warmup duration, measured runs, sample count, p50/p95/p99, confidence interval, and raw sample artifact paths.
- Gate thresholds: render throughput not lower; p95 HTTP and dirty-to-visible no more than 10% slower; Minecraft p95 tick no more than 5% worse; combined RSS no more than 15% higher; every queue converges after input stops.

- [ ] **Step 1: Write report math and threshold tests**

Use fixed samples to assert percentile interpolation, confidence intervals, warmup exclusion, missing metric rejection, higher-is-better throughput direction, lower-is-better latency/RSS direction, exact threshold boundaries, and non-converging queue failure. Check in deterministic command descriptors for the fixture Java and Rust launchers plus synthetic pass/fail reports used to test the gate exit status.

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-compare --test benchmark_report`

Expected: missing benchmark module.

- [ ] **Step 3: Instrument both sides without high-cardinality labels**

Measure snapshot copy/encode, frame bytes, compression ratio, in-flight count, outbox depth/high-water, durable dirty depth, render throughput, dirty-to-visible, HTTP latency/status, startup-ready, recovery, CPU/RSS, and Java tick/GC data. World labels may use configured web world name; never label by player, chunk coordinate, request ID, or session UUID.

- [ ] **Step 4: Implement separate-run benchmark orchestration**

Run Java and Rust workloads sequentially with identical recording/config/JVM/host, one warmup and at least three measured runs. Refuse to compare different config hashes or environments unless `--allow-environment-mismatch` is explicitly supplied; such a report cannot pass `gate`.

- [ ] **Step 5: Run fixture benchmarks and gate logic**

Run: `cargo run --manifest-path rust/Cargo.toml -p squaremap-compare -- benchmark --recording testdata/bridge/v1/reference.rec --backend-command-java testdata/bridge/v1/benchmark/java-command.json --backend-command-rust testdata/bridge/v1/benchmark/rust-command.json --output build/ab/performance.json`

Run: `cargo run --manifest-path rust/Cargo.toml -p squaremap-compare -- gate --parity build/ab/parity.json --performance build/ab/performance.json`

Expected: fixture report validates and gate command exits according to the checked-in synthetic threshold cases. Do not claim real performance until Task 18 runs on representative servers.

- [ ] **Step 6: Commit A/B measurement gates**

```bash
git add rust/crates/squaremap-server rust/crates/squaremap-compare common/src/main/java/xyz/jpenilla/squaremap/common/bridge/metrics common/src/test testdata/bridge/v1
git commit -m "Measure Java and Rust backend parity"
```

---

### Task 17: Build, verify, and resolve native release binaries

**Files:**
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BackendManifest.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BinaryResolver.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/process/BinaryResolverTest.java`
- Create: `common/src/test/resources/xyz/jpenilla/squaremap/common/bridge/process/backend-manifest.json`
- Create: `build-logic/src/main/kotlin/RustBackend.kt`
- Generate: `common/build/generated/resources/squaremap-backends.json`
- Modify: `.github/workflows/build.yml:9-92`
- Modify: `common/build.gradle.kts`
- Modify: `build-logic/src/main/kotlin/squaremap.platform.gradle.kts`

**Interfaces:**
- Resolution order: configured absolute path, versioned verified cache, verified release download.
- Manifest key is exact plugin version plus Rust target triple and contains URL, byte length, and lowercase SHA-256; the packaged JAR embeds the build-generated manifest as `/squaremap-backends.json`.
- Unsupported OS/architecture and offline cache miss produce actionable startup errors before process launch.

- [ ] **Step 1: Write resolver security tests**

Use a local HTTP fixture and assert valid download, exact cache reuse, wrong hash, wrong length, redirect off approved HTTPS origin, partial download, executable replacement race, unsupported target, explicit path, offline cache hit/miss, and Windows/Linux executable handling.

- [ ] **Step 2: Run the resolver test and verify failure**

Run: `./gradlew :squaremap-common:test --tests '*BinaryResolverTest'`

Expected: missing resolver.

- [ ] **Step 3: Implement atomic verified resolution**

Download to a new file in the versioned cache, stream SHA-256 while enforcing declared length, fsync, re-check the embedded manifest entry, set owner execution permission on POSIX, and atomically rename. Open/verify immediately before launch and reject writable-by-others POSIX files. Never execute the temporary path.

- [ ] **Step 4: Split CI into native-build and package jobs**

Build the five target triples, run Rust tests on native Linux, upload each stripped binary, collect artifacts in a package job, generate `squaremap-backends.json` from actual bytes under `common/build/generated/resources`, then build/test Gradle and Bun with that generated directory on `processResources`. Release uploads include the JAR, five binaries, manifest, and checksums. PRs build Linux x86_64 plus run cross-target `cargo check`; releases build the full matrix. Never commit release hashes generated from local non-release binaries.

- [ ] **Step 5: Run local build integration**

Run: `cargo test --manifest-path rust/Cargo.toml --workspace`

Run: `./gradlew build`

Expected: both pass; the development launcher resolves the locally built host binary without network access.

- [ ] **Step 6: Commit native distribution**

```bash
git add .github/workflows/build.yml build-logic common/build.gradle.kts common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process common/src/test
git commit -m "Distribute verified Rust backend binaries"
```

---

### Task 18: Run the required A/B matrix and publish Rust as opt-in

**Files:**
- Create: `docs/superpowers/verification/rust-backend-ab-report.json`
- Create: `docs/superpowers/verification/rust-backend-ab-summary.txt`
- Modify: `README.md:90-112`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/config/Config.java`
- Modify: locale files under `common/src/main/resources/locale/` only for new actionable backend errors

**Interfaces:**
- Checked-in report contains parity and performance results for Paper, Fabric, representative modded NeoForge, and Sponge smoke.
- `backend: rust` remains opt-in in this task; `backend: java` remains default.
- Rollback is mode change plus restart and never changes an output root in place.

- [ ] **Step 1: Run deterministic replay and require exact parity**

Run `squaremap-compare replay` over every manifest fixture and `compare-output` against Java expected output. Expected: 100% decoded RGBA equality, semantic JSON equality, identical icon hashes, and no ignored mismatch list.

- [ ] **Step 2: Run the live correctness matrix**

For Paper and Fabric, run block churn, players, all marker shapes, full/radius/background renders, cancel/resume, world reload, sidecar kill/restart, and queue saturation. Run representative modded block/fluid parity on NeoForge and lifecycle/render smoke on Sponge. Quiesce before comparison. Expected: zero unexplained mismatch and zero lost durable dirty coordinate.

- [ ] **Step 3: Run isolated performance comparison**

On one documented host, run the same recording/config in Java then Rust, one warmup and at least three measured runs. Generate the machine report and gate it. Expected: all thresholds in Task 16 pass; if one fails, stop this task and fix the responsible earlier component rather than editing thresholds.

- [ ] **Step 4: Test canary and restart rollback**

Expose shadow Rust only through a separately protected route, verify an explicit test cohort, select Rust primary, then select Java primary after restart. Expected: each mode serves only its own root, both roots remain valid, and no automatic live failover occurs.

- [ ] **Step 5: Check in evidence and operator instructions**

The summary must name exact commits, environments, sample counts, gate values, and report path. README documents executable resolution, offline path, the three temporary modes, diagnostics, and rollback. Do not claim loader coverage absent from the report.

- [ ] **Step 6: Run repository verification**

Run: `cargo test --manifest-path rust/Cargo.toml --workspace`

Run: `./gradlew build`

Run: `cd web && bun run lint && bun run build`

Expected: all pass.

- [ ] **Step 7: Commit opt-in release evidence**

```bash
git add docs/superpowers/verification README.md common/src/main/java/xyz/jpenilla/squaremap/common/config/Config.java common/src/main/resources/locale
git commit -m "Document the verified Rust backend rollout"
```

---

### Task 19: Transfer map configuration ownership to Rust

**Files:**
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeBootstrapConfig.java`
- Create: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/state/BridgePolicyStore.java`
- Create: `common/src/test/java/xyz/jpenilla/squaremap/common/config/RustConfigCompatibilityTest.java`
- Create: `rust/crates/squaremap-server/src/config/source.rs`
- Create: `rust/crates/squaremap-server/src/config/schema.rs`
- Create: `rust/crates/squaremap-server/tests/config_compatibility.rs`
- Create: `testdata/bridge/v1/config/default-config.yml`
- Create: `testdata/bridge/v1/config/default-advanced.yml`
- Create: `testdata/bridge/v1/config/world-overrides.yml`
- Create: `testdata/bridge/v1/config/lang.yml`
- Create: `testdata/bridge/v1/config/invalid-config.yml`
- Create: `testdata/bridge/v1/config/expected.json`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/config/ConfigManager.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/SquaremapCommon.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/state/PlayerStateExporter.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/state/DirtyChunkPublisher.java`
- Modify: `paper/src/main/java/xyz/jpenilla/squaremap/paper/listener/MapUpdateListeners.java`
- Modify: `fabric/src/main/java/xyz/jpenilla/squaremap/fabric/listener/FabricMapUpdates.java`
- Modify: `neoforge/src/main/java/xyz/jpenilla/squaremap/forge/event/ForgeMapUpdates.java`
- Modify: `sponge/src/main/java/xyz/jpenilla/squaremap/sponge/listener/MapUpdateListener.java`

**Interfaces:**
- Java parses only bootstrap keys needed before Rust exists: backend mode, executable path/download policy, cache path, startup timeout, diagnostics root, and output-root selection.
- Rust reads the existing global, advanced, world, and locale YAML files from the canonical data root passed at process launch.
- Rust validates the complete configuration before activating it, then sends a typed `BridgePolicyReplace` containing privacy rules, event-capture filters, and snapshot credit limits needed by Java.
- Existing YAML keys and generated defaults retain their meanings; an unsupported or malformed key fails reload without replacing the active configuration.

- [ ] **Step 1: Export current Java defaults and write failing compatibility tests**

Generate fixtures for default global/advanced/world settings, every visibility-limit shape, custom world overrides, locale UI values, malformed bounds, and an unknown key. Java records the current effective values; Rust tests must parse the same files into equal typed values. Java tests assert `BridgePolicyStore` applies only a complete newer generation and rejects stale/wrong-session policies.

- [ ] **Step 2: Run the focused tests and verify failure**

Run: `cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test config_compatibility`

Run: `./gradlew :squaremap-common:test --tests '*RustConfigCompatibilityTest'`

Expected: Rust source/schema and Java policy store are absent.

- [ ] **Step 3: Parse existing map configuration in Rust**

Use typed Serde YAML structs with `deny_unknown_fields`, explicit defaults copied from the Java fixtures, bounded numeric types, and path validation relative to the canonical data root. Load all files into a candidate, validate cross-field/world invariants, then atomically swap one `Arc<ConfigGeneration>`. A failed reload returns structured diagnostics with file/key paths and leaves the previous generation active.

- [ ] **Step 4: Reduce Java to bootstrap configuration and policy application**

`BridgeBootstrapConfig` reads a small `bridge.yml` before sidecar launch. On first migration, copy only backend/bootstrap values from the old global config and leave map settings untouched for Rust. Replace Java `Config`/`WorldConfig` reads in player privacy, event filtering, snapshot limits, and exporters with immutable `BridgePolicyStore` snapshots received from Rust.

- [ ] **Step 5: Verify reload and backward compatibility**

Run both focused suites. Start with an unmodified current squaremap data directory, reload a valid world override, then attempt an invalid reload. Expected: initial output matches Java, valid reload advances one generation on both peers, invalid reload reports exact keys and preserves prior behavior.

- [ ] **Step 6: Commit Rust configuration ownership**

```bash
git add common/src/main/java/xyz/jpenilla/squaremap/common/bridge common/src/main/java/xyz/jpenilla/squaremap/common/config/ConfigManager.java common/src/main/java/xyz/jpenilla/squaremap/common/SquaremapCommon.java common/src/test rust/crates/squaremap-server testdata/bridge/v1/config
git commit -m "Move map configuration ownership to Rust"
```

---

### Task 20: Make Rust default for one observation release

**Files:**
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeBootstrapConfig.java`
- Modify: `README.md`
- Modify: `docs/superpowers/verification/rust-backend-ab-report.json`
- Test: all Java, Rust, web, and loader smoke suites

**Interfaces:**
- New installations default to `rust`; upgraded installations retain their explicit/current mode.
- `java` remains restart-selected fallback for exactly this observation release.
- The report gains release duration, install count/sample source, crash/restart rate, and unresolved issue count without player-identifying data.

- [ ] **Step 1: Add a failing config-upgrade test**

Assert a fresh config chooses `rust`, an existing config without a mode is migrated conservatively to `java` with one notice, and explicit `java`, `shadow`, or `rust` is preserved.

- [ ] **Step 2: Run the test and verify failure**

Run: `./gradlew :squaremap-common:test --tests '*Config*Test'`

Expected: fresh default is still Java.

- [ ] **Step 3: Change only the fresh-install default and upgrader**

Do not delete legacy code in this task. Emit one actionable upgrade message, preserve separated roots, and keep rollback instructions accurate.

- [ ] **Step 4: Repeat smoke, fault, and release verification**

Run all commands from Task 18 Step 6 plus loader smoke scenarios. Expected: clean fresh Rust startup, conservative upgraded Java startup, explicit Rust upgrade, and tested Java rollback.

- [ ] **Step 5: Commit the observation default**

```bash
git add common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeBootstrapConfig.java common/src/test README.md docs/superpowers/verification/rust-backend-ab-report.json
git commit -m "Make the Rust backend the default"
```

- [ ] **Step 6: Hold the release gate**

Ship one observation release. Do not begin Task 21 until its report records no unresolved parity/reliability issue and the required A/B gates still pass on the release build. A failure reopens the owning earlier task; it does not extend dual-backend code indefinitely without an explicit architecture review.

---

### Task 21: Delete the Java backend and legacy migration paths

**Files:**
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/httpd/IntegratedServer.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/httpd/JsonCache.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/httpd/ViteRunner.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/task/render/`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/task/UpdatePlayers.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/task/UpdateWorldData.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/task/UpdateMarkers.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/data/Image.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/data/RenderManager.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/util/ImageIOExecutor.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/LegacyBackendController.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BackendMode.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/BackendPaths.java`
- Rename with LSP: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/BridgeBackendController.java` to `common/src/main/java/xyz/jpenilla/squaremap/common/backend/BackendControllerImpl.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/config/AbstractConfig.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/config/AbstractWorldConfig.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/config/Advanced.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/config/Config.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/config/ConfigManager.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/config/ConfigUpgrader.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/config/Transformations.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/config/WorldAdvanced.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/config/WorldConfig.java`
- Delete: Java-only visibility/color/render helpers after `lsp references` proves no bridge/API caller remains
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/SquaremapCommon.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/data/MapWorldInternal.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeBootstrapConfig.java`
- Modify: `common/build.gradle.kts:13-55`
- Modify: `build-logic/src/main/kotlin/squaremap.platform.gradle.kts:35`
- Modify: `README.md`

**Interfaces:**
- Only Rust owns HTTP, views, dirty/render persistence, rendering, PNG output, and map-control execution.
- Java retains loader adapters, event capture, privacy filtering, public API registries, snapshot extraction, command/permission facades, and sidecar supervision.
- There is no selectable backend mode after cutover: the supervised Rust sidecar is always authoritative. Offline diagnostic recording/replay remains, but legacy Java execution and live shadow dual-writing are removed. Java parses only `bridge.yml`; Rust is the sole parser for map/render/HTTP/UI configuration.

- [ ] **Step 1: Prove every legacy caller has migrated**

Use LSP references for `IntegratedServer`, `JsonCache`, `RenderManager`, `AbstractRender`, `Image`, each update task, Java map/render configuration classes, `LegacyBackendController`, `BackendPaths`, `BackendMode`, and `BridgeBackendController`. Expected: only deletion/rename targets, tests, and docs remain. Verify all platform schedules now call `BridgeStatePublisher`. Migrate any real caller before deleting; do not leave adapters or deprecated aliases.

- [ ] **Step 2: Add a failing architecture-boundary test**

Add a Gradle test that inspects resolved runtime dependencies and Java class outputs. Assert no `io.undertow`, no classes under `common.httpd` or `common.task.render`, no Java references to `javax.imageio` outside addon icon normalization, no Java map/render YAML parser, no `BackendMode`, no `BackendPaths`, and no legacy backend controller.

- [ ] **Step 3: Run the boundary test and verify failure**

Run: `./gradlew :squaremap-common:test --tests '*BackendBoundaryTest'`

Expected: fails on Undertow and legacy classes.

- [ ] **Step 4: Delete the backend in one clean-cutover change**

Remove every listed implementation, its Guice bindings/factories, Java map/render configuration parsers, config keys used only by legacy/shadow mode, migrated JSON/resume-file readers after their one supported upgrade window, and Undertow dependency/minimizer exception. Use LSP file rename for `BridgeBackendController` so every injected/reference type becomes `BackendControllerImpl`; do not leave the bridge-qualified alias. After the final successful Rust checkpoint, atomically rename each untouched `dirty_chunks.json` and `resume_render.json` to its `.migrated-v1` backup before removing the importer. Shrink `MapWorldInternal` to the public API/bridge world handle; it must not own executors, dirty sets, render managers, paths for Java tile writes, or image encoding.

- [ ] **Step 5: Verify public API and every supported loader**

Run: `cargo test --manifest-path rust/Cargo.toml --workspace`

Run: `./gradlew build`

Run: `cd web && bun run lint && bun run build`

Launch Paper, Fabric, NeoForge, and Sponge smoke servers with Rust backend; fetch settings, players, markers, one existing tile, and one missing tile; invoke full render, pause, resume, and cancel; register an addon marker and icon. Expected: all contracts pass and process inspection shows no Java HTTP/render threads.

- [ ] **Step 6: Repeat replay and performance gates on the deletion commit**

Run `squaremap-compare gate` using newly generated parity/performance reports from this exact commit. Expected: all correctness and performance gates pass; report contains no legacy fallback.

- [ ] **Step 7: Commit the clean cutover**

```bash
git add -A common
git add build-logic README.md docs/superpowers/verification
git commit -m "Remove the legacy Java map backend"
```

---

## Final program acceptance

Before calling the migration complete, verify all of these against the current commit:

- [ ] Java has no HTTP listener, JSON publisher/cache, render scheduler, pixel renderer, tile encoder, or dirty/render persistence.
- [ ] Rust is the sole parser and owner of map/render/HTTP/UI configuration; Java parses only sidecar bootstrap settings and consumes `BridgePolicyReplace`.
- [ ] Rust serves the unchanged UI/file/HTTP contract.
- [ ] Existing Java addon API usage produces equivalent marker/icon output.
- [ ] Paper, Fabric, NeoForge, and Sponge smoke scenarios pass.
- [ ] Protocol malformed-input, disconnect, saturation, recovery, world-epoch, cancel/resume, and disabled-HTTP scenarios pass.
- [ ] Deterministic replay has exact decoded RGBA and semantic JSON equality with no ignore list.
- [ ] Live-shadow evidence and isolated A/B performance evidence are attached and pass the fixed gates.
- [ ] Rust was default for one observation release before legacy deletion.
- [ ] Final `cargo test --manifest-path rust/Cargo.toml --workspace`, `./gradlew build`, and web lint/build pass on the clean-cutover commit.
