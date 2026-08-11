# Task 2 Report: Bounded Cross-Language Framing

## RED evidence

Rust focused command, run before the framing APIs existed:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-protocol --test frame_limits
```

Observed result: compilation failed as expected with unresolved imports for `envelope`, `read_envelope`, `write_envelope`, `ChunkSnapshot`, `ChunkSnapshotBody`, `Envelope`, `FrameClass`, `FrameError`, and `FrameLimits` from `squaremap_protocol`.

Java focused command, run before `FrameCodec` existed:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*FrameCodecTest'
```

Observed result: `BUILD FAILED` during `:squaremap-common:compileTestJava`; the compiler reported 9 errors because `FrameCodec` and `FrameCodec.ProtocolException` did not exist.

## GREEN evidence

Rust focused command:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-protocol --test frame_limits
```

Observed result: `15 passed` in the `frame_limits` suite.

Java focused command:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*FrameCodecTest'
```

Observed result: `BUILD SUCCESSFUL`; the focused XML report records `tests="15"`, `skipped="0"`, `failures="0"`, and `errors="0"`.

No formatter, linter, or project-wide suite was run.

## Allocation-order and framing review

Both codecs read exactly the five-byte class/unsigned-big-endian-length prefix into fixed storage. Rust uses a stack `[u8; 5]`; Java uses a five-byte `ByteBuffer`. Unknown classes, zero lengths, and class-specific declared-length violations are rejected before payload-sized allocation. Rust then allocates `BytesMut::zeroed(length)` and uses counted `read_exact`; Java converts the header with `Integer.toUnsignedLong`, checks the configured limit and `Integer.MAX_VALUE`, and only then calls `ByteBuffer.allocate((int) length)`. Both channel implementations loop over partial reads and writes, and both report early EOF with expected/actual counts.

Class is derived from the decoded payload on write. On read, every payload other than `ChunkSnapshot` requires control class, while `ChunkSnapshot` requires snapshot class. Protobuf decode, class mismatch, malformed class/length, early EOF, and snapshot validation are distinct error categories.

Snapshot validation is bounded and symmetric: compressed body must be non-empty and within the snapshot limit; declared uncompressed length must be within 128 MiB; declared length must not exceed compressed length multiplied by the named ratio maximum 4096 (with checked arithmetic); zstd decompression must succeed and produce exactly the declared number of bytes; CRC32C must match the uncompressed serialized `ChunkSnapshotBody`; and the body protobuf must decode. The same validation runs on read and write. Empty and invalid zstd inputs, absolute-limit violations, ratio violations, decompressed-length mismatches, CRC mismatches, invalid body protobuf, and unsigned declarations are covered by focused tests.

## Files changed

- `rust/Cargo.toml`
- `rust/Cargo.lock`
- `rust/crates/squaremap-protocol/Cargo.toml`
- `rust/crates/squaremap-protocol/src/lib.rs`
- `rust/crates/squaremap-protocol/src/frame.rs`
- `rust/crates/squaremap-protocol/src/limits.rs`
- `rust/crates/squaremap-protocol/tests/frame_limits.rs`
- `gradle/libs.versions.toml`
- `common/build.gradle.kts`
- `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/protocol/FrameCodec.java`
- `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/protocol/FrameLimits.java`
- `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/protocol/FrameCodecTest.java`

## Self-review

- Limits are exactly control 1,048,576 bytes, snapshot 67,108,864 bytes, and uncompressed snapshot 134,217,728 bytes in both implementations.
- Rust and Java use the Task 1 `ChunkSnapshot.compressed_body` contract and do not add sockets, sessions, process launch, persistence, HTTP, or Task 3 behavior.
- `.superpowers/sdd/task-2-report.md`
- Focused tests cover unknown class, zero length, both oversize frame classes, unsigned Java header handling, class mismatch in both directions, early EOF, trailing bytes, invalid envelope protobuf, absolute decompression limit, ratio 4096, empty/invalid zstd, CRC mismatch, unsigned snapshot length, valid one-byte split reads, and invalid write-side snapshots.
- Dependency changes are limited to Tokio/bytes/zstd/CRC32C for the Rust protocol crate and the zstd-jni catalog entry/dependency required by Java chunk-body validation. The existing NeoForge catalog entry remains unchanged.

## Concerns

None for the requested Task 2 scope.
