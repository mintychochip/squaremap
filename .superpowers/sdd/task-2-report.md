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
- Focused tests cover unknown class, zero length, both oversize frame classes, unsigned Java header handling, class mismatch in both directions, early EOF, trailing bytes, invalid envelope protobuf, absolute decompression limit, ratio 4096, empty/invalid zstd, CRC mismatch, unsigned snapshot length, valid one-byte split reads, invalid write-side snapshots, and deterministic zero-progress channel failures.
- Dependency changes are limited to Tokio/bytes/zstd/CRC32C for the Rust protocol crate and the pure-Java Aircompressor catalog entry/dependency for Java chunk-body validation. The existing NeoForge catalog entry remains unchanged.

## Concerns

None for the requested Task 2 scope.

## Review-fix evidence

### Zero-progress RED

After adding finite channels that return `0` once and then EOF/closed, the exact focused Java command was run:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*FrameCodecTest'
```

Observed result: `BUILD FAILED`; `17 tests completed, 2 failed`. `rejectsReadableChannelWithoutProgress` observed the old `early EOF` category instead of `no progress`, and `rejectsWritableChannelWithoutProgress` observed the old `IOException` instead of `ProtocolException`.

### Zero-progress and Aircompressor GREEN

The same exact focused Java command was rerun after the fixes:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*FrameCodecTest'
```

Observed result: `BUILD SUCCESSFUL`; the focused XML report records `tests="17"`, `skipped="0"`, `failures="0"`, and `errors="0"`.

The required Rust command was rerun unchanged:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-protocol --test frame_limits
```

Observed result: `15 passed` in the `frame_limits` suite.

### Pure-Java dependency verification

The selected dependency is `io.airlift:aircompressor:0.27`, whose verified source APIs are `io.airlift.compress.zstd.ZstdCompressor` and `ZstdDecompressor`; both implement the byte-array compressor/decompressor interfaces in Java source. The exact focused Gradle resolution command was:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:dependencyInsight --dependency aircompressor --configuration runtimeClasspath
```

Observed resolution output:

```text
io.airlift:aircompressor:0.27
  Variant runtime:
    org.gradle.category            library
    org.gradle.libraryelements     jar
    org.gradle.usage               java-runtime
io.airlift:aircompressor:0.27
\--- runtimeClasspath
BUILD SUCCESSFUL
```

The resolved jar was inspected at `~/.gradle/caches/modules-2/files-2.1/io.airlift/aircompressor/0.27/.../aircompressor-0.27.jar`; the artifact contains no `.so`, `.dll`, `.dylib`, JNI, or native entries, and the dependency-insight closure contains no transitive native/JNI artifact. `common/build.gradle.kts` uses `implementation(libs.aircompressor)`, so the dependency is not exposed through the public common API. Snapshot validation remains Java-side and retains preallocation limits, exact decompressed length, CRC32C, and typed body decode.

### Review self-check

- Read and write loops now reject `count == 0` immediately with the distinct `no progress` `ProtocolException`; finite one-byte partial I/O tests still pass.
- The Aircompressor replacement changes only Java chunk-body compression/decompression and test fixture generation; frame limits, ratio checks, CRC/body checks, class validation, and unsigned lengths remain unchanged.
- The first review-fix scope contained only the Task 2 Java dependency/codec/tests and this report update; the later follow-up review-fix scope adds the symmetric Rust/Java zstd policy and validation changes documented below.


## Follow-up review-fix evidence

### Thread safety, window policy, and zero-output RED

The required focused Rust command was run after adding the new contract tests but before adding the corresponding production variants:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-protocol --test frame_limits
```

Observed result: compilation failed as expected with two unresolved `SnapshotValidationReason` variants: `ZeroUncompressedLength` and `WindowLimit`.

The exact focused Java command was also run before the zero-output production fix:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*FrameCodecTest'
```

Observed result: `20 tests completed, 1 failed`; `rejectsTruncatedMagicPrefixedZeroLengthSnapshot` failed because the old decoder returned successfully for standard zstd magic followed by truncated data when the declared output length was zero. The concurrent distinct-body test was included in this run.

### Follow-up GREEN

After the fixes, the exact focused Java command was rerun:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*FrameCodecTest'
```

Observed result: `BUILD SUCCESSFUL`; the focused XML report records `tests="21"`, `skipped="0"`, `failures="0"`, and `errors="0"`.

The exact focused Rust command was rerun:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-protocol --test frame_limits
```

Observed result: `18 passed` in the `frame_limits` suite.

The Java decompressor is now `ThreadLocal<ZstdDecompressor>`, and the concurrent test repeatedly validates eight distinct bodies in parallel, including independent CRCs and write/read round trips. Rust configures its decoder with `FrameLimits::MAX_ZSTD_WINDOW_LOG` (23). Java uses Aircompressor 0.27, whose decoder applies the same 8 MiB maximum window; both suites reject a valid frame encoded with a larger 10 MiB window. Both suites also accept a 9,000,006-byte body encoded with the compliant 8 MiB window, preserving the separate 128 MiB decompressed-output limit.

The interim follow-up rejected `uncompressed_length == 0` symmetrically before decompression, closing the Aircompressor zero-output bypass but over-constraining the valid default Protobuf body. The final review fix below supersedes that behavior with bounded one-byte probing and exact single-frame validation.

Allocation order remained bounded throughout: fixed five-byte prefix, class-specific length check, bounded envelope allocation, protobuf decode, compressed-body bound, absolute/ratio checks, then bounded output allocation and decompression. Rust configures the shared window limit before reading from the decoder.

### Dependency inspection

The dependency check was rerun after the follow-up:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:dependencyInsight --dependency aircompressor --configuration runtimeClasspath
```

Observed resolution was only `io.airlift:aircompressor:0.27` on `runtimeClasspath` with `org.gradle.libraryelements=jar`, followed by `BUILD SUCCESSFUL`. The resolved jar contains no `.so`, `.dll`, `.dylib`, JNI, or native entries, and the resolved closure contains no native/JNI artifact.

### Follow-up self-review

- Shared zstd policy is named in both `FrameLimits` classes as `MAX_ZSTD_WINDOW_LOG = 23` and `MAX_ZSTD_WINDOW_BYTES = 1L << 23`; Rust applies the log directly to its decoder and Java's Aircompressor decoder enforces the same window bound.
- Java decompressor mutable state is not shared across threads.
- The interim zero-output rejection was superseded by the final bounded empty-output validation below.
- High-window rejection, compliant-window large-body acceptance, concurrent validation, and zero-output bypass coverage are now in the focused contract tests.
- No formatter, linter, project-wide suite, Task 3 work, or progress-ledger change was made.

## Final review-fix evidence

### Single-frame and valid-empty RED

The final Java contract tests added explicit acceptance of a valid zstd-compressed empty `ChunkSnapshotBody` and rejection of skippable, concatenated, and trailing zstd data. Before the Java production fix, the exact focused command was:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*FrameCodecTest'
```

Observed result: `BUILD FAILED`; `23 tests completed, 1 failed`. `acceptsEmptySnapshotBody` was rejected by the blanket zero-uncompressed-length check. The malformed zero-output, skippable, concatenated, and trailing cases remained finite.

### Final GREEN

After replacing the blanket rejection with bounded empty-output validation and adding exact single-standard-frame validation, the same Java command produced `BUILD SUCCESSFUL`. The focused XML records `tests="23"`, `skipped="0"`, `failures="0"`, and `errors="0"`.

The Rust focused command was rerun against its single-frame implementation:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-protocol --test frame_limits
```

Observed result: `20 passed`.

Java validates one standard zstd frame structurally before decompression: frame-header field lengths, the shared 8 MiB window, block headers and stored lengths, optional checksum bytes, and exact end-of-input are all bounds-checked without allocating from block declarations. For a declared empty body, it supplies a one-byte decompression probe so Aircompressor must parse the entire frame, then treats the logical output as the zero-byte Protobuf body for CRC32C and typed decode. Rust requires the same standard magic, decodes one frame with window log 23, and rejects any bytes remaining in either the decoder's buffer or inner reader.

Both bindings therefore accept `zstd(ChunkSnapshotBody::default())` with declared length and CRC32C zero, while rejecting magic-prefixed truncation, skippable frames, concatenated frames, and trailing compressed data. All prior allocation, ratio, CRC, concurrency, no-progress, and no-JNI invariants remain covered.