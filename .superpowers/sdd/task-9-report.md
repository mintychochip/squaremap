# Task 9 — Export registry descriptors and compact chunk snapshots

## RED evidence

- `./gradlew :squaremap-common:test --tests xyz.jpenilla.squaremap.common.bridge.snapshot.ChunkSnapshotEncoderTest.encodesNegativeSectionsAndMatchesSharedFixture --no-daemon` failed after the fixture was expanded, proving the immutable Java fixture comparison caught drift before regeneration.
- `cargo test -p squaremap-render --test snapshot_decode overwidth_top_level_scalar_is_rejected_before_prost_cast` failed before top-level scalar preflight with the expected assertion mismatch.
- `cargo test -p squaremap-server snapshot_client::tests::registry_replace_requires_matching_pending_correlation` failed before correlation binding because a foreign registry replaced the active empty registry and snapshot decoding reported `UnknownDescriptor`.
- `cargo test -p squaremap-server snapshot_client::tests::registry_generation_is_reset_for_new_world_epoch` failed before epoch-qualified registry keys with `Snapshot(RegistryMismatch)`.

## GREEN evidence

- `./gradlew :squaremap-common:compileJava :squaremap-common:compileTestJava --no-daemon` — BUILD SUCCESSFUL.
- `./gradlew :squaremap-common:test --tests xyz.jpenilla.squaremap.common.bridge.snapshot.ChunkSnapshotEncoderTest --no-daemon` — BUILD SUCCESSFUL.
- `cargo test -p squaremap-render --test snapshot_decode --test snapshot_render_hardening` — 14 passed, including the enabled unknown-descriptor malformed case and explicit air fixture assertions.
- `cargo test -p squaremap-server snapshot_client` — 6 passed, including same-name new-epoch registry replacement.
- `git diff --check` — no whitespace errors.

## Changed behavior

- Added deterministic session-local registry descriptors with world/revision binding, descriptor-count limits, explicit clear/invisible AIR, water, and glass fixture descriptors/states, and no fallback IDs.
- Added `SnapshotRequestService.Work` upstream/encoded-stage tracking so cancellation holds permits until both stages terminate; default admission remains 96.
- Added production request handling with admission before export, loader-safe provider stages, a bounded bridge worker, typed malformed/missing outcomes, duplicate rejection without clobbering valid responses, epoch invalidation, failure abort, and shutdown/reload/unload lifecycle closure.
- Added a separate restart-abort path so reload leaves the singleton controller, scheduler, and handler reusable; terminal shutdown closes controller/handler before bridge connection and supervisor.
- Added acknowledgement-wired per-world registry gating: a pending registry generation must be acknowledged before dependent snapshots or the next generation publish, and failure/restart resets the gate.
- Added authenticated session gating on Java inbound frames and restored production controller injection.
- Added a callable Rust `SnapshotClient` request/response seam with deterministic bounded correlation/request IDs, matching pending correlation/world/revision registry acceptance, epoch-qualified registry generations, strict snapshot decode routing, bootstrap snapshot payload consumption before views, and no-mutation handling for unsolicited responses. Task 13 owns the scheduler invocation that will call `request_once`; this task does not claim live runtime request generation.
- Updated immutable Java/Rust fixture comparison with an air-only section, semantic air/water/glass descriptors, and a width-3 palette crossing a 64-bit boundary plus malformed cross-language tests.

## Commit

Single atomic commit after squashing the migration range onto the approved Task 9 base: `Send portable chunk snapshots to Rust`.
