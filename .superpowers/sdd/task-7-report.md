# Task 7 report: Mirror map view state into Rust

## RED evidence

- `./gradlew :squaremap-common:test --tests '*StateExporterTest'` before implementation failed during test compilation with six expected missing-symbol errors for the new exporter/publisher contracts (artifact 1127).
- `cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test view_contract` before implementation failed with unresolved `squaremap_server::views`, `squaremap_state::view`, and `OutputRoot::latest_bytes` (artifact 1129).

## Contracts and fixtures

- Added `testdata/bridge/v1/views/empty.json`, `settings.json`, `world-settings.json`, `markers.json`, `icons.json`, and `players.json`; fixtures cover empty state, namespaced legacy world names, public-player fields, all marker geometry shapes/options, layer timestamps, and normalized icon metadata.
- Added `StateExporterTest` and `BridgeStatePublisherTest`; the latter proves SHADOW receives one collected immutable value with identity preserved to both sinks.
- `StateExporterTest` parses every checked-in golden document and compares full semantic JSON emitted by production exporter cores plus exact legacy sink document builders; fixed observations cover player privacy/health/armor/world identity, both worlds/settings, every real marker shape/options, reload timestamps, and multiple icon pixels. Rust applies the production-equivalent icon replacement, compares complete `icons.json`, and decodes the emitted PNG pixels.
- Added typed Rust view structs with explicit JSON names and omission behavior; marker geometry includes icon, circle, ellipse, rectangle, polyline, polygon, and multi-polygon.
- Extended protocol `Icon` with normalized RGBA `width` and `height`, `WorldStateReplace` with `UiSettings`, and `MarkerLayer` with a content-change timestamp.

## Implementation and routing

- Added immutable Java world/player/marker/icon exporters with one canonical `ServerLevel` load token, monotonic revisions, unchanged-snapshot suppression, and deterministic identity ordering. Marker layer timestamp state is cleared when `MarkerRouter` binds a new load token while remaining stable for same-load schedules. Production exports and tests share pure mapping cores.
- `PlayerStateExporter` emits the actual dimension namespace/value and applies production visibility filters. `UpdatePlayers.publish`, `UpdateWorldData.publish`, and `UpdateMarkers.publish` consume supplied snapshots only; old scheduling/collection/cache responsibilities are removed, and their shared legacy document builders are tested against complete fixtures.
- `BridgeStatePublisher`, `SidecarSupervisor`, `BridgeBootstrapConfig`, `IconRegistry`, and API provider are explicit singleton graph objects. YAML `settings.bridge.*` keys are loaded lazily after config initialization, so the started supervisor and scheduled publisher share one configured instance. Replacement outbox drains worlds before epoch-dependent markers/players.
- `IconRegistry` is API-owned and shared, publishes sorted mutation snapshots in SHADOW/RUST, writes legacy files only when configured, and unregisters transactionally. Rust retains canonical replacement state separately from latest output bytes, reconciles removed world/icon outputs, preserves active world settings across epoch reloads, and removes only stale markers for reloaded worlds.
- Paper, Fabric, NeoForge, and Sponge schedules invoke the publisher. Player interval remains 1 second, world interval remains 5 seconds, and marker initial delay/configured interval remain unchanged.
- `SidecarSupervisor` propagates `SQUAREMAP_OUTPUT_ROOT`, closes and clears failed connections, and makes failed SHADOW publication coalesced/nonthrowing. Root isolation canonicalizes existing ancestors and rejects same/ancestor/descendant aliases.
- Rust validates revisions and world epochs before writes, allows valid initial out-of-order baseline snapshots to converge, rejects unsupported payloads, emits structured `ProtocolError` on handler/write failures before close, and ACKs only after replacement output/reconciliation succeeds. Child restart/reconnect remains explicitly deferred to Task 15.

## Verification

- RED: `./gradlew :squaremap-common:test --tests '*StateExporterTest'` failed before implementation with six expected missing-symbol errors (artifact 1127).
- RED: `cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test view_contract` failed before implementation with unresolved views/latest-bytes symbols (artifact 1129).
- `./gradlew :squaremap-common:test --tests '*StateExporterTest' --tests '*BridgeStatePublisherTest' --tests '*CoalescingOutboxTest' --tests '*SidecarSupervisorTest*' --tests '*ProductionBridgeWiringTest'`: BUILD SUCCESSFUL.
- `./gradlew :squaremap-api:compileJava :squaremap-common:compileJava :squaremap-paper:compileJava :squaremap-fabric:compileJava :squaremap-neoforge:compileJava :squaremap-sponge:compileJava`: BUILD SUCCESSFUL.
- `cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test view_contract`: 8 passed, including all checked-in fixtures, complete icon JSON/PNG fixture linkage, out-of-order baseline convergence, deterministic sorting, stale revision/epoch rejection, root reopen reconciliation, reload settings preservation with stale marker removal, confined removal, and invalid payload coverage.
- `SQUAREMAP_OUTPUT_ROOT=/tmp/squaremap-rust-bootstrap cargo test --manifest-path rust/Cargo.toml -p squaremap-server bootstrap::protocol_tests`: 3 passed.
- `git diff --check`: passed.

## Concerns

- None.
