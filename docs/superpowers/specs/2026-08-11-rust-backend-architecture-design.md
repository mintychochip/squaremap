# Rust Backend Architecture Design

Date: 2026-08-11
Status: Planning baseline for review

## Intent

Reduce squaremap's Java runtime to a Minecraft integration bridge and move the map-serving backend to Rust.

The target data flow is:

```text
Minecraft platform plugin/mod
  -> versioned local event and snapshot protocol
  -> managed Rust sidecar
  -> existing HTTP/file contract
  -> existing web UI
```

“Majority of the backend” has a concrete meaning in this design. Rust owns HTTP serving, backend state, render scheduling, tile rendering and image encoding, dirty-chunk persistence, player/world/marker JSON production, cache policy, and operational metrics. Java owns only work that requires a live Minecraft or loader API: lifecycle hooks, event capture, safe chunk snapshot extraction, player privacy/visibility decisions, the existing Java addon API, platform commands, and native-process supervision.

The initial migration preserves the current web UI and its URLs. It is a backend replacement, not a simultaneous UI rewrite.

## Current architecture

The current request path is static-file oriented:

- `common/httpd/IntegratedServer.java` runs Undertow and serves the extracted UI, JSON, marker icons, and tiles.
- `common/httpd/JsonCache.java` keeps generated JSON in memory and optionally mirrors it to disk.
- `UpdatePlayers`, `UpdateWorldData`, and `UpdateMarkers` publish the UI's current JSON contract.
- `RenderManager` and `task/render/*` own render queues and execute background, radius, and full renders.
- `AbstractRender` converts Minecraft `ChunkSnapshot` values to pixels.
- `Image` creates the 512-pixel PNG tile pyramid.
- Platform listeners such as Paper's `MapUpdateListeners` and Fabric's `FabricMapUpdates` mark chunks dirty.
- The public Java API owns marker/layer/icon registries used by addons.
- The UI polls `tiles/settings.json`, `tiles/players.json`, per-world `settings.json`, `markers.json`, and PNG tiles.

The important seam already exists conceptually: platform code detects Minecraft changes, while common code transforms those changes into web artifacts. The migration makes that seam an explicit, versioned process boundary.

## Goals

1. Keep every supported loader integration in Java, where the Minecraft APIs already exist.
2. Move the non-Minecraft backend to safe Rust with no JNI and no Rust code loaded into the JVM.
3. Preserve the public Java addon API and the current UI/file/URL contract through the cutover.
4. Keep all event handling off the Minecraft tick thread except the minimum safe snapshot copy.
5. Make disconnects, backpressure, replay, upgrades, and rollback explicit protocol behavior.
6. Run Java and Rust implementations against equivalent inputs long enough to prove parity and measure cost.
7. Remove the Java backend after the Rust-default observation release; do not retain a permanent dual implementation.

## Non-goals

- Rewriting the web UI during the backend migration.
- Exposing the bridge protocol as a public remote API.
- Parsing live Minecraft region files directly in Rust in the first cutover.
- Replacing the Java addon API with a Rust API.
- Moving loader-specific commands, packet channels, mixins, or event registration to Rust.
- Adding WebSockets or server-sent events to an interface that currently works by polling.
- Removing unrelated telemetry or update-check functionality solely because the backend is changing.

Direct Anvil-region parsing is a later optimization. It would reduce snapshot traffic, but cross-version NBT details, loaded-versus-saved state, modded registries, and concurrent region writes make it a separate project.

## Architecture options

### Option A: managed Rust sidecar — selected

The plugin launches a version-matched Rust executable. Java and Rust communicate through an authenticated loopback stream. Rust binds the public HTTP port and owns the output directory.

Advantages:

- A Rust panic or memory fault cannot corrupt the Minecraft JVM.
- Rust can be restarted, profiled, constrained, and A/B tested independently.
- The ownership boundary matches the desired plugin -> backend -> UI pipeline.
- The Rust server can later run externally without changing the domain protocol.

Costs:

- Native binaries require an OS/architecture release matrix.
- Startup, supervision, logging, and upgrade compatibility must be designed.
- Chunk snapshots cross a serialization boundary.

### Option B: Rust native library through JNI

Java would load a Rust `cdylib` and call it directly.

Advantages:

- One process and lower-copy calls.
- No child-process lifecycle or loopback transport.

Costs:

- Unsafe JNI code becomes a load-bearing boundary.
- A native crash takes down the Minecraft server.
- Class-loader reloads, native-library extraction, and multi-loader packaging are fragile.
- It does not create the operational separation requested.

This option is rejected.

### Option C: operator-managed external Rust service

The plugin would connect to a separately installed service over TCP.

Advantages:

- Strongest process and host separation.
- One service could theoretically serve multiple Minecraft instances.
- Independent deploy cadence.

Costs:

- Adds installation, authentication, firewall, discovery, and upgrade work for every server owner.
- Remote snapshot transport can be large.
- Failure and compatibility modes are more complex than a local child process.

This is not the default. The protocol will avoid assumptions that prevent an advanced external mode later, but remote transport is not part of the first migration.

## Selected component boundary

### Java bridge retains

- Paper, Fabric, NeoForge, and Sponge bootstrap/lifecycle code.
- Loader events, mixins, plugin channels, player hide/show behavior, and safe scheduler access.
- World discovery and the mapping from Minecraft registry objects to stable session-local numeric IDs.
- Immutable chunk snapshot capture from live Minecraft state.
- Player visibility/privacy evaluation before any player data leaves the JVM.
- The public `squaremap-api` types and Java addon registries.
- A thin marker/icon exporter that converts addon state to protocol values.
- Loader command registration and permission checks; command execution becomes a request to Rust.
- Sidecar acquisition, launch, handshake, health reporting, and shutdown.

### Rust backend gains

- Public HTTP server and static UI serving.
- Existing ETag, cache-control, missing-tile, and disabled-internal-server semantics.
- Canonical world, player, marker, and UI state.
- JSON serialization for all existing UI endpoints.
- Dirty-chunk coalescing, render job scheduling, pause/cancel/resume, and durable progress.
- Visibility-limit evaluation.
- Block/biome/fluid color calculation from registry descriptors.
- Chunk-to-pixel rendering, neighboring-column shading, image pyramid composition, and PNG encoding.
- Atomic output writes and registered-icon storage.
- Health, queue-depth, render, HTTP, process, and comparison metrics.
- A replay/comparison CLI used by CI and live A/B runs.

### Java code removed after cutover

The clean-cutover release removes Undertow, `IntegratedServer`, `JsonCache`, Java render executors and render implementations, Java tile image encoding, and Java ownership of dirty/render progress files. `MapWorldInternal`, configuration classes, and update tasks shrink to bridge/API adapters or are deleted where no Java caller remains. The Undertow dependency leaves `common/build.gradle.kts`.

## Process and packaging model

The default deployment is a managed child process:

1. The plugin resolves an executable in this order: configured absolute path, versioned local cache, verified release download.
2. Release CI produces Linux x86_64, Linux aarch64, Windows x86_64, macOS x86_64, and macOS aarch64 binaries.
3. The plugin JAR contains a build-generated manifest of exact binary SHA-256 hashes. A downloaded binary must match the plugin version and the manifest embedded in the already-installed JAR before execution.
4. Offline installations can set the executable path or pre-populate the cache. No unverified executable is run.
5. Java opens an ephemeral loopback listener, starts the child with the address and version, and sends a one-use 256-bit bootstrap token through child stdin rather than command-line arguments.
6. Rust connects back, proves the token, negotiates an exact protocol major and compatible minor version, then closes the bootstrap stdin.
7. Rust reports backend readiness only after state recovery, bridge bootstrap, and HTTP bind succeed.

In `shadow` mode, binary acquisition or child failure never interrupts the Java primary. In `rust` mode, a missing or incompatible backend fails loudly and does not expose a half-initialized map.

## Bridge protocol

### Encoding and transport

Use one bidirectional, length-delimited Protobuf stream over loopback TCP. This is deliberately not gRPC: a small framing layer avoids pulling a second HTTP/2 stack into every plugin while retaining generated, versioned schemas for Java and Rust.

Each frame contains:

- protocol major/minor;
- server session UUID;
- monotonically increasing sequence number;
- optional request/correlation ID;
- payload type;
- payload bytes;
- maximum-size enforcement before allocation.

Large chunk payloads use zstd inside the Protobuf message and include uncompressed length and CRC32C. Control messages are not compressed. Schema evolution follows additive Protobuf rules; field numbers are never reused. Major incompatibility rejects startup before Rust serves traffic.

### Messages

The first protocol version includes:

- `Hello`, `HelloAck`, `Ready`, `Heartbeat`, and `Shutdown`.
- `ConfigReplace`, `BridgePolicyReplace`, and `RegistryReplace`.
- `WorldUpsert`, `WorldRemove`, and `WorldStateReplace`.
- `PlayersReplace`, `MarkerLayersReplace`, and `IconsReplace`.
- `ChunkDirty` and `WorldResyncRequired`.
- `ChunkSnapshotRequest`, `ChunkSnapshot`, and `ChunkMissing`.
- `RenderCommand`, `RenderProgress`, and `RenderResult`.
- `Ack`, structured `ProtocolError`, and `BackendHealth`.

World messages carry a world epoch. Reusing a world identifier after unload/load cannot apply stale dirty events or snapshot responses.

### Chunk snapshot representation

The bridge sends Minecraft-independent data, not Java class names or serialized Java objects:

- world ID and epoch, chunk coordinates, min/max Y, dimension ceiling flag, and revision;
- section-local block-state palettes plus packed palette indices;
- section-local biome palettes plus packed indices;
- the world-surface heightmap;
- session-local block-state and biome IDs defined by `RegistryReplace`;
- only rendering-relevant flags not derivable from those registries.

Registry descriptors provide map color, transparency/invisibility class, glass behavior, fluid class, biome tint inputs, and mod/loader-specific overrides. The bridge computes descriptors while Minecraft registries are available; Rust owns the per-pixel algorithm and blending.

Rust requests snapshots with a credit limit. Java captures them on the loader-safe scheduler, copies immutable values, and performs Protobuf/zstd encoding on a bridge worker. Neither socket writes nor compression run on the tick thread.

## Data flow

### Startup

1. Java starts and authenticates Rust.
2. Rust opens its SQLite state store and validates its schema.
3. Java sends full configuration, registries, worlds, icons, marker layers, and current players.
4. Rust reconciles that bootstrap against persisted worlds and dirty/render jobs.
5. Rust binds or activates HTTP and reports `Ready`.

### Live chunk updates

1. A loader event marks a world/chunk revision dirty in Java.
2. Java enqueues a tiny `ChunkDirty`; repeated coordinates coalesce.
3. Rust persists the dirty coordinate before acknowledging it.
4. The Rust scheduler requests snapshots subject to explicit credits.
5. Java captures and sends a snapshot or `ChunkMissing`.
6. Rust renders affected pixels and neighboring shading dependencies, atomically writes tiles, and clears the durable dirty entry.

### Full/radius render

The Java command facade validates permissions and sends `RenderCommand`. Rust creates a durable job, enumerates coordinates, requests snapshots in bounded batches, reports progress, and owns cancel/pause/resume semantics. Command text remains localized in Java from structured result codes.

### Players, worlds, markers, and icons

These are replaceable snapshots rather than an unbounded mutation log. Java publishes only when the canonical value changes. Rust atomically swaps current state and regenerates the same JSON and asset paths the UI already consumes. A reconnect sends the complete current snapshot, so missed transient updates do not require replay.

## State and filesystem ownership

Rust uses one SQLite database in WAL mode beneath squaremap's data directory for worlds, render jobs, dirty coordinates, protocol checkpoints, and schema version. Ephemeral player state remains in memory. Chunk snapshots are bounded work items, not a permanent duplicate world database.

Rust remains the sole writer for the web output tree while active. It preserves the existing paths so the web UI and external reverse-proxy setups do not change. Output writes use temporary files plus atomic replacement. Existing `dirty_chunks.json` and `resume_render.json` are imported idempotently but left untouched while Java rollback exists. After the observation release and final successful Rust checkpoint, the clean-cutover migration renames them as `.migrated-v1` backups before removing the legacy importer. The migration never mutates Minecraft world files.

When the integrated HTTP server is disabled, the Rust backend still renders and writes JSON/tiles but does not bind the public HTTP listener, preserving external-web-server installations.

## Backpressure, recovery, and failure handling

- Every queue is bounded and exposes depth/high-water metrics.
- Dirty chunks coalesce by `(world, epoch, x, z)`; player/marker/world state uses latest-value replacement.
- If the Java outbound dirty set reaches its cap while Rust is unavailable, Java records `WorldResyncRequired` instead of allocating without bound.
- Snapshot requests are credit-based. Rust cannot make Java hold more than the configured number of immutable snapshots.
- Rust acknowledges durable messages only after the SQLite transaction commits.
- Duplicate events and responses are idempotent by session, sequence, world epoch, coordinate, and revision.
- On reconnect, Java sends a full baseline. Rust discards stale session messages and reconciles durable jobs.
- Java supervises the child with capped exponential restart backoff and a restart budget. It never spins indefinitely.
- In shadow mode, Java remains primary if Rust fails. In Rust-primary mode during the observation release, an operator can select Java on restart; automatic mid-request failover is intentionally avoided because two writers must never share an output tree.
- Logs identify component, session, protocol version, world, and correlation ID without logging bootstrap secrets or player addresses.

## Security model

The first protocol is local-only. Both peers reject non-loopback addresses. A random one-use token prevents an unrelated local process from winning the startup race. Frames have strict size, decompression-ratio, enum, coordinate, palette, and section-count limits before expensive work. Paths are derived from validated world identifiers and configured roots; protocol messages never supply arbitrary filesystem paths. The HTTP server retains the current public surface and adds no unauthenticated control endpoints.

An external/remote daemon would require mutual authentication and a separate threat model; it is not enabled by this design.

## Java-versus-Rust A/B validation

This is a stateful renderer, so random per-request traffic splitting would produce misleading results and cache inconsistency. A/B testing uses equivalent inputs, isolated output roots, isolated HTTP ports, and explicit cohorts.

### Modes

- `java`: current implementation is the only writer and server.
- `shadow`: Java remains primary. The bridge mirrors the same captured snapshots and state to Rust, which writes to an isolated Rust output root and serves a loopback-only comparison port.
- `rust`: Rust is primary. During one observation release, Java remains available only as restart-selected rollback and writes to a separate root when invoked.

No mode allows Java and Rust to write the same output tree.

### Stage 1: deterministic replay corpus

Add a recorder behind a development flag that stores protocol frames plus the Java renderer's decoded 512x512 RGBA tile results. The committed fixture corpus covers:

- positive and negative coordinates and region boundaries;
- overworld, ceiling dimension, and End-like dimensions;
- empty/missing chunks and min/max build heights;
- water/lava depth, glass and stained glass, transparent/invisible blocks;
- biome tinting and blend radii;
- neighboring north/south height shading;
- visibility limits and zoom pyramid edges;
- representative modded block/fluid descriptors;
- players with privacy filtering, marker geometry, icons, and world reload epochs.

The Rust replay CLI consumes the exact recorded messages. Tile comparison decodes PNGs and hashes RGBA pixels, not compressed PNG bytes. JSON comparison validates schemas and compares canonical semantic values.

Release gate: 100% exact RGBA equality and semantic JSON equality for the committed corpus, with no ignored mismatch list.

### Stage 2: live shadow correctness

Run both backends on representative Paper and Fabric servers, plus a modded NeoForge scenario and a Sponge smoke scenario. After each test reaches quiescence, compare every emitted JSON document, icon hash, existing tile's decoded RGBA hash, missing-tile behavior, and render-job result. Inject rapid block updates, world unload/reload, player churn, marker changes, sidecar disconnect/restart, queue saturation, full render cancel/resume, and plugin shutdown.

Release gate: zero unexplained mismatches, zero lost dirty coordinates after recovery, bounded queues throughout, and no writes outside each backend's assigned root.

### Stage 3: isolated performance comparison

Correctness shadow runs are not performance evidence because dual rendering creates contention. Replay the same recorded workload separately in `java` and `rust` modes on the same host/JVM/server settings. Collect warm-up and at least three measured runs for:

- Minecraft tick p50/p95/p99 and snapshot-copy time;
- chunks and regions rendered per second;
- dirty-event-to-tile-visible p50/p95/p99;
- HTTP static/JSON/tile throughput and p50/p95/p99 latency;
- Java heap, GC pause/time, Rust RSS, combined RSS, CPU time, and disk bytes;
- startup-to-ready, shutdown, and recovery time;
- queue high-water marks and snapshot compression ratio.

The comparison CLI emits machine-readable JSON and a human-readable report with environment, commit, config, sample counts, and confidence intervals.

Performance gate: Rust must be at least as fast in render throughput; p95 HTTP and dirty-to-visible latency may not regress by more than 10%; Minecraft p95 tick time may not regress by more than 5%; combined steady-state RSS may not regress by more than 15%; and no queue may grow without converging after input stops. Any threshold waiver requires a documented cause and explicit review, not a hidden tolerance.

### Stage 4: canary and rollback

Expose the Rust shadow server only on loopback or a separately protected reverse-proxy route. Testers use an explicit cohort URL; ordinary users remain on Java. After parity and performance gates pass, make Rust opt-in primary, then default primary in the next observation release. Preserve the Java output root and additive state migration so rollback is a configuration change plus restart. After the observation release has no unresolved parity or reliability issue, delete the legacy Java backend and retain only offline fixture comparison for regression testing.

## Testing strategy beyond A/B

- Protocol compatibility tests generated from shared golden frames in Java and Rust.
- Property tests for malformed lengths, palettes, coordinates, decompression limits, duplicate sequences, and world epochs.
- Rust unit tests for color, shading, visibility, tile coordinate, and zoom invariants.
- Rust integration tests for SQLite recovery, atomic writes, HTTP headers/status, missing tiles, and disabled HTTP mode.
- Java tests for non-blocking event coalescing, scheduler-safe snapshot capture, privacy filtering, process supervision, and structured command responses.
- End-to-end harness that launches a fake bridge and real Rust server, replays fixtures, fetches the current UI paths, kills/restarts Rust, and verifies convergence.
- Loader smoke tests for Paper, Fabric, NeoForge, and Sponge before changing the default.

## Migration sequence

1. **Baseline and contracts** — capture current file/HTTP behavior, add metrics, define golden fixtures, and commit the Protobuf schema without changing the default backend.
2. **Bridge and sidecar shell** — add the managed process, authenticated framing, handshake, lifecycle, CI target matrix, and fake-backend tests behind `backend: java`.
3. **Rust HTTP/state backend** — serve the existing web tree and reproduce current JSON/cache behavior while Java still produces tiles.
4. **State producers** — move world/player/marker/icon JSON ownership and durable job/dirty state to Rust; Java exports canonical snapshots.
5. **Rust renderer** — send compact chunk snapshots and registry descriptors; implement pixel rendering, tile pyramid, PNG output, and render commands.
6. **Shadow and comparison tooling** — run deterministic replay, live shadow correctness, fault injection, and isolated performance suites.
7. **Rust opt-in and canary** — publish platform binaries, documentation, diagnostics, and restart-selected rollback.
8. **Rust default observation release** — keep Java fallback isolated for one release while Rust is the default.
9. **Clean cutover** — remove Java HTTP/render/state code, Undertow, legacy mode, duplicate config, migrated-file readers, and unused dependencies after gates pass.

Each phase is independently revertible and keeps the default behavior working. The implementation plan will split these into testable commits rather than one long-lived rewrite branch.

## Configuration transition

The migration begins with the existing YAML files as the user-facing source of truth. Java parses them and sends a complete `ConfigReplace`, avoiding an immediate format migration. Once Rust owns every map/render/HTTP/UI consumer, Rust becomes the parser for those settings and sends Java a narrow, validated `BridgePolicyReplace` containing only privacy/event-capture policy and snapshot limits. Through the observation release, Java locally parses backend mode, executable resolution, startup timeout, and process launch because those values are needed before Rust exists. Clean cutover removes backend mode; Java then parses only executable/bootstrap settings for the always-authoritative sidecar. Existing keys retain their meaning; unsupported keys fail validation rather than being silently ignored.

During shadow mode, backend-specific output root and comparison port settings are explicitly development/diagnostic options. They are removed with legacy mode after clean cutover.

## Acceptance criteria

The migration is complete only when all of the following are true:

- The Java runtime contains no HTTP listener, JSON cache/publisher, render scheduler, pixel renderer, tile image encoder, or dirty/render persistence implementation.
- Rust serves the current UI and endpoint/file contract without requiring a UI change.
- Existing Java addons can obtain worlds and register layers, markers, and icons through the public API and see equivalent UI output.
- Paper, Fabric, NeoForge, and Sponge bridge lifecycle and event smoke tests pass.
- Disconnect/restart, backlog saturation, world reload, render cancellation/resumption, and disabled-HTTP scenarios converge without data loss or unbounded allocation.
- The deterministic and live A/B correctness gates pass with no ignored mismatch list.
- The isolated performance gates pass and their report is attached to the cutover change.
- Rust is default for an observation release with a tested restart rollback.
- The following clean-cutover release deletes the Java backend and duplicate legacy path.

## Principal risks and mitigations

- **Snapshot bandwidth and copy cost:** use palette-packed sections, zstd, credits, coalescing, and measured copy-time budgets. Direct region parsing remains a later optimization only if measurements justify it.
- **Modded rendering parity:** derive session registry descriptors inside Minecraft, retain fixture coverage for representative modded blocks/fluids, and reject unknown descriptor versions.
- **Native distribution:** build a fixed target matrix, embed hashes in the plugin, support explicit offline paths, and fail before execution on mismatch.
- **Two-writer corruption:** isolate roots and ports by mode; never implement automatic live failover against one output directory.
- **Protocol drift:** share one schema, golden cross-language frames, exact startup negotiation, additive minor changes, and no field-number reuse.
- **Tick-thread impact:** events enqueue only small values; safe snapshot copy is credit-limited; encoding and socket I/O run on bridge workers.
- **Permanent dual-backend maintenance:** time-box Java fallback to one observation release and make legacy deletion an explicit completion criterion.
