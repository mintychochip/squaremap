# Task 1 Report: Shared Protocol and Rust Workspace

## Review-fix RED

The original Task 1 Java RED remains captured below. The reviewer-completeness RED was then captured with focused contract tests before schema changes.

Original Java command:

```text
./gradlew :squaremap-common:test --tests '*GoldenEnvelopeTest'
```

Original result: expected compilation failure because protobuf/JUnit/generated types were absent, including `package xyz.jpenilla.squaremap.bridge.v1 does not exist`, `cannot find symbol class Envelope`, and `cannot find symbol class Hello` (`12 errors`, `BUILD FAILED`).

Rust schema-contract command before the reviewer fix:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-protocol --test schema_contract
```

Result: expected compilation failure. The compiler reported that `ChunkSnapshotBody` did not exist and that the generated types lacked `ConfigReplace.global/advanced/world/locale/render/ui`, `World.icon/order`, `Player.world/display_name/armor/health`, `PlayersReplace.max_players`, `Marker.geometry/style/tooltip`, `ChunkSnapshot.body/uncompressed_length`, and envelope payload fields (`27 previous errors`).

Java schema-contract command before the reviewer fix:

```text
./gradlew :squaremap-common:test --tests '*SchemaContractTest'
```

Result: expected compilation failure. The compiler reported missing generated `AdvancedSettings`, `GlobalSettings`, `LocaleSettings`, `RenderSettings`, `UiSettings`, `WorldSettings`, `ChunkSnapshotBody`, marker geometry/style/tooltip messages, and missing builders/accessors for world/player/max/body/presence fields (`32 errors`, `BUILD FAILED`).

Controller-strengthened Java command:

```text
./gradlew :squaremap-common:test --tests '*SchemaContractTest'
```

Result: expected compilation failure after the contract test added the remaining UI locale labels, marker-layer controls, and explicit compressed-body name. Generated builders lacked `setSpawnMarkerLabel`, `setShowControls`, and `setCompressedBody` (`4 errors`, `BUILD FAILED`).

Fractional marker-coordinate RED was captured in both bindings before changing `Point`:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-protocol --test schema_contract
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*SchemaContractTest'
```

Rust rejected floating-point literals where generated `Point.x/z` were `i32` (`4 errors`). Java rejected `setX(-1.25)` as a lossy `double`-to-`int` conversion (`1 error`, `BUILD FAILED`).

## Review-fix GREEN

Focused Rust command:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-protocol --test golden_envelope --test schema_contract
```

Result:

```text
running 1 test
test encodes_v1_hello_golden_frame ... ok

test result: ok. 1 passed; 0 failed

running 1 test
test exposes_complete_typed_state_contract ... ok

test result: ok. 1 passed; 0 failed
```

Focused Java command:

```text
./gradlew --no-daemon --no-configuration-cache :squaremap-common:test --tests '*GoldenEnvelopeTest' --tests '*SchemaContractTest'
```

Result:

```text
BUILD SUCCESSFUL in 11s
22 actionable tasks: 6 executed, 16 up-to-date
```

An earlier daemon-backed controller rerun produced passing XML for both tests but hung during Gradle teardown and timed out, so it is not counted as verification. The clean non-daemon result above is authoritative.

The unchanged Rust and Java Hello tests still compare the same checked-in deterministic 37-byte `testdata/bridge/v1/hello.bin`; the schema tests exercise typed configuration/world/player/marker/chunk construction, all seven marker geometries, fractional marker coordinates, marker-layer controls and visibility, locale labels, compressed-body naming, and absent player coordinates.

## Reviewer fixes

- Corrected the workspace package `rust-version` to `1.88` and pinned `rust/rust-toolchain.toml` to `1.88.0`.
- Expanded `ConfigReplace` into typed `GlobalSettings`, `AdvancedSettings`, `WorldSettings`, `LocaleSettings`, `RenderSettings`, and `UiSettings`, covering the existing Java global, advanced color/registry, world map/tracker/marker/zoom, locale, render-progress, and UI settings. No map, `Any`, or opaque configuration blob is used.
- Expanded `World` with icon/order/environment/enabled state, typed spawn, player-tracker, zoom, marker/tile intervals, height/ceiling metadata, and typed settings.
- Expanded `Player`/`PlayersReplace` with world identity, optional display name, optional armor/health, optional signed `x/y/z/yaw`, and top-level `max_players`. Proto3 `optional` preserves tracker-disabled coordinate absence; UUID remains a 16-byte `bytes` field.
- Replaced the partial marker model with explicit typed `MarkerIcon`, `MarkerCircle`, `MarkerEllipse`, `MarkerRectangle`, `MarkerPolyline`, `MarkerPolygon`, and `MarkerMultiPolygon` oneof geometry, typed `MarkerStyle`/`MarkerTooltip`, and complete layer visibility/control/order metadata. `MarkerLayersReplace` carries the world identity so an empty world snapshot remains routable.
- Added typed `ChunkSnapshotBody` for sections and heightmap. `ChunkSnapshot` now has one body field: `compressed_body` is exactly the zstd-compressed serialization of `ChunkSnapshotBody`; `uncompressed_length` and `crc32c` cover the uncompressed serialized body bytes. The old competing inline section/heightmap fields were removed.

## Files

- `protocol/squaremap/bridge/v1/bridge.proto`
- `rust/Cargo.toml`
- `rust/Cargo.lock`
- `rust/rust-toolchain.toml`
- `rust/crates/squaremap-protocol/Cargo.toml`
- `rust/crates/squaremap-protocol/build.rs`
- `rust/crates/squaremap-protocol/src/lib.rs`
- `rust/crates/squaremap-protocol/tests/golden_envelope.rs`
- `rust/crates/squaremap-protocol/tests/schema_contract.rs`
- `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/protocol/GoldenEnvelopeTest.java`
- `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/protocol/SchemaContractTest.java`
- `common/build.gradle.kts`
- `gradle/libs.versions.toml`
- `.gitignore`
- `testdata/bridge/v1/hello.bin`
- `.superpowers/sdd/task-1-report.md`

## Self-review

- Envelope payload field numbers remain exactly the brief contract: control 10–16, replace 20–22, world 30–32, UI 40–42, chunk 50–54, render 60–62, health 70.
- Every named payload remains defined. No `google.protobuf.Any`, `google.protobuf.Struct`, `map<...>`, or generic configuration/marker payload exists. The only byte fields are protocol UUID/token/image/compressed chunk-body data and packed chunk indices.
- World identity remains `{namespace, value, epoch}`. Minecraft/chunk/player coordinates use signed `sint32`; marker and visibility-limit points preserve the public API's floating-point X/Z geometry. UUIDs are byte strings.
- Every enum has an explicit zero `*_UNSPECIFIED` value; field numbers are not reused inside messages.
- Generated Java remains in `xyz.jpenilla.squaremap.bridge.v1`; generated Rust remains in `squaremap_protocol::wire`. Vendored `protoc-bin-vendored` is still selected by `build.rs`, so Rust generation does not require host-installed protoc.
- Focused tests exercised both language bindings and the shared Hello bytes. No framing, declared-length allocation, sockets, process launch, persistence, rendering, or HTTP behavior was added.
- No formatter, linter, project-wide suite, or broad build was run.

## Concerns

None for the requested reviewer fixes. `ChunkSnapshot.compressed_body` contains the zstd-compressed serialized typed `ChunkSnapshotBody`; later framing/decoder work owns zstd validation and limits.
