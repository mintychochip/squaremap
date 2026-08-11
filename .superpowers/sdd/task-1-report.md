# Task 1 Report: Shared Protocol and Rust Workspace

## RED

Command:

```text
./gradlew :squaremap-common:test --tests '*GoldenEnvelopeTest'
```

Result: expected compilation failure before protobuf/JUnit/generated-code setup. The compiler reported absent dependencies and generated types, including:

```text
error: package com.google.protobuf does not exist
error: package org.junit.jupiter.api does not exist
error: package xyz.jpenilla.squaremap.bridge.v1 does not exist
error: cannot find symbol
  symbol:   class Envelope
error: cannot find symbol
  symbol:   class Hello
12 errors
BUILD FAILED
```

The RED state was caused by the new test importing `Envelope` and `Hello` before the shared contract and Java generation configuration existed.

## GREEN

Rust command:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-protocol --test golden_envelope
```

Result:

```text
running 1 test
test encodes_v1_hello_golden_frame ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Java command:

```text
./gradlew :squaremap-common:test --tests '*GoldenEnvelopeTest'
```

Result:

```text
> Task :squaremap-common:test UP-TO-DATE
BUILD SUCCESSFUL in 1s
22 actionable tasks: 1 executed, 21 up-to-date
```

The Rust and Java tests both read the checked-in `testdata/bridge/v1/hello.bin`. The deterministic bytes encode protocol major 1, minor 0, the fixed 16-byte session ID, sequence 1, and the scalar-only `Hello` payload (`test` / `token`).

## Files

- `protocol/squaremap/bridge/v1/bridge.proto`: shared proto3 contract with the exact `Envelope` payload field numbers from the brief and every named v1 payload.
- `rust/Cargo.toml`: Rust workspace, edition 2024, MSRV/rust-version 1.85, and shared dependency versions.
- `rust/rust-toolchain.toml`: pinned Rust 1.85.0 toolchain.
- `rust/crates/squaremap-protocol/Cargo.toml`: protocol crate dependencies.
- `rust/crates/squaremap-protocol/build.rs`: vendored-protoc `prost-build` generation without requiring a host `protoc` installation.
- `rust/crates/squaremap-protocol/src/lib.rs`: generated `squaremap_protocol::wire` module.
- `rust/crates/squaremap-protocol/tests/golden_envelope.rs`: Rust golden compatibility test.
- `rust/Cargo.lock`: locked Rust dependency graph.
- `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/protocol/GoldenEnvelopeTest.java`: Java golden compatibility test.
- `common/build.gradle.kts`: Java protobuf generation, shared protocol source directory, protobuf runtime, JUnit Jupiter, and JUnit platform setup.
- `gradle/libs.versions.toml`: protobuf Gradle plugin/compiler/runtime and JUnit versions.
- `.gitignore`: Rust backend-local output directories while leaving `.bin` files trackable.
- `testdata/bridge/v1/hello.bin`: shared deterministic golden frame.

## Self-review

- `Envelope` uses protocol fields 1–5 exactly as specified and all payload tags exactly as specified: control payloads 10–16, replace payloads 20–22, world payloads 30–32, UI snapshots 40–42, chunk payloads 50–54, render payloads 60–62, and health 70.
- Every payload named by the brief is defined in the schema. Payload internals use explicit scalar/message fields; no `google.protobuf.Any` or other opaque payload wrapper is used.
- World identity is represented by `WorldIdentity { namespace, value, epoch }`; chunk/player/marker coordinates use signed `sint32`; UUID-bearing fields use `bytes`; chunk sections carry local palettes and packed index bytes; snapshot metadata carries world epoch, coordinates, build-height bounds, ceiling flag, revision, uncompressed length, and CRC32C.
- Every schema enum has an explicit zero `*_UNSPECIFIED` value and no field number is reused within a message.
- Java generated sources were observed under `xyz/jpenilla/squaremap/bridge/v1`; Rust generated definitions were observed inside `squaremap_protocol::wire`, including `Envelope`, `envelope::Payload`, and `Hello`.
- Java and Rust serialize the same 37-byte golden frame. Java uses the brief-approved `HexFormat` fallback for the fixed session ID because `ByteString.copyFromHex` is not assumed available.
- Rust build generation selects the `protoc-bin-vendored` executable before invoking `prost-build`; no host-installed protoc is required. The Java Gradle plugin uses the declared Maven protoc artifact.
- Dependency scope is limited to protobuf generation/runtime, JUnit test execution, and the Rust protocol crate. No framing, allocation limits, sockets, process launch, persistence, rendering, or HTTP behavior was added.
- The focused commands above were the only Rust/Java validation commands run; no project-wide test suite, formatter, linter, or broad build was run.

## Concerns

None identified for Task 1. Payload-internal field choices follow the meanings and Minecraft-independent representation in the authoritative architecture design; the brief only fixes the envelope payload tags and the Hello golden fields.
