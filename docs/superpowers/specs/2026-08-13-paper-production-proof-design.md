# Paper Production Proof Design

Date: 2026-08-13
Status: Approved for local implementation; remote gate is `blocked_infrastructure`

## Intent

Prove the Rust backend through a real, pinned local Paper lifecycle rather than infer production behavior from Java or Rust component tests. The proof runs the production plugin and managed Rust child, drives actual Minecraft state changes through supported Paper surfaces, waits for strict quiescence, and emits a fail-closed evidence bundle.

The gate must establish all six claims:

1. Rust owns the public HTTP listener and output tree in `rust` mode.
2. Actual block changes produce Java/Rust tile parity, including dependent neighboring tiles.
3. World reload, player churn, render cancellation/resumption, and shutdown converge without stale, lost, duplicate, or late state.
4. Killing the sidecar while Paper remains alive causes bounded restart, authenticated reconnect, replay, and convergence.
5. Equivalent Java/Rust scenarios produce complete timing, throughput, queue, and RSS measurements.
6. Every compared output matches: no missing paths, extra paths, semantic JSON differences, or decoded RGBA pixel differences.

## Current baseline

The migration worktree already contains:

- Java bridge supervision, transport, state export, snapshot, and lifecycle code under `common/src/main/java/xyz/jpenilla/squaremap/common/bridge`;
- Rust HTTP, scheduler, rendering, recovery, and metrics code under `rust/crates/squaremap-server`;
- `rust/crates/squaremap-compare` with recording, replay, comparison, report, and benchmark code;
- deterministic comparison fixtures under `testdata/bridge`;
- component and integration tests for HTTP contracts, rendering, recovery, ordering, and comparison.

Those assets are component evidence. They do not prove a real Paper lifecycle. Missing proof infrastructure is a transparent Java bridge recorder, a production-shaped local lifecycle driver/corpus, and an executed Paper scenario covering mutations, churn, reload, cancellation, restart, quiescence, HTTP ownership, performance, RSS, and zero mismatches.

## Architecture

### Pinned local Paper fixture

A repository manifest pins the Minecraft and Paper versions, Paper artifact SHA-256, plugin JAR SHA-256, Rust sidecar SHA-256, Java and Rust versions, deterministic seed/world, JVM flags, timezone/locale, view and simulation distances, ports, roots, scenario version, timeouts, quiet interval, and RSS sampling interval.

Downloaded binaries may remain outside Git, but every executable input is hash-verified before startup. Java-primary and Rust-primary runs use fresh server data, distinct ports, distinct output roots, and distinct diagnostics roots. Path aliasing or symlink overlap fails before Paper starts.

### Transparent Java BridgeRecorder

`common/src/main/java/xyz/jpenilla/squaremap/common/bridge/recording/BridgeRecorder.java` records frames at the production post-coalescing transport boundary. Each record includes direction, monotonic time, session/sequence/correlation metadata where available, payload length, and exact frame bytes. Bootstrap secrets are excluded.

Recording is opt-in and disabled in normal production. The writer is bounded and asynchronous. It cannot alter payloads, ordering, acknowledgements, error propagation, or backpressure. A full writer, dropped frame, flush failure, close failure, path-confinement failure, or hash failure terminates the proof as invalid.

### Black-box Paper lifecycle driver

A repository verification task launches the pinned Paper process and observes it externally through process state, logs, console/RCON or a documented fixture control command, HTTP, and filesystem output. It must not call Squaremap internals to manufacture success.

The lifecycle is:

1. start Paper and await plugin readiness;
2. await authenticated sidecar and Rust HTTP readiness;
3. capture the baseline endpoint/output inventory;
4. place, break, and mutate blocks at positive, negative, chunk-edge, and region-edge coordinates;
5. await affected tiles and neighboring shading dependencies;
6. join, move, hide/show, and disconnect fixture players;
7. unload and reload the mapped world and observe an epoch replacement;
8. start a full render, cancel it, prove terminal cancellation, resume/restart, and await exactly one completion;
9. reach strict quiescence;
10. kill only the supervised Rust child while Paper remains responsive;
11. await bounded restart, authenticated reconnect, baseline replay, dirty/render recovery, and a new child generation;
12. repeat a real block mutation and reach final quiescence;
13. stop Paper and prove no listener, child, temporary file, or late write remains.

Every step records start/end, command acknowledgement, relevant epochs/revisions, process generations, expected and observed hashes, timeout, and failure reason. A skipped step is a failure.

### Twin execution and comparison

Run A uses the Java backend as reference. Run B uses Rust as the authoritative backend. Both consume the same fixture manifest, seed, action sequence, requested paths, and convergence barriers. Only fields explicitly named as volatile in the manifest may be normalized; map content, paths, status codes, headers, pixels, lifecycle outcomes, queue loss, and ownership are never normalized.

`squaremap-compare` is the comparison authority:

- JSON is compared semantically while preserving array order and numeric semantics.
- PNG files are decoded to RGBA; reports include the first mismatching coordinate and total mismatched pixels.
- Other files are compared byte-for-byte.
- The complete path set is compared, so missing and extra outputs fail.
- Replay consumes recorded bridge traffic and must regenerate equivalent outputs.
- Reports are immutable and refuse overwrite.

## Proof contracts

### HTTP and output ownership

The evidence records listener address, owning PID, executable, child generation, socket ownership, access transcript, and filesystem writer identity where supported. Rust mode exercises `GET` and `HEAD` for settings, players, world settings, markers, icons, existing tiles, and missing tiles, including cache headers and ETags.

Pass requires the Rust child to own the public listener and Rust output tree. Any Java public listener, Java response for the Rust public endpoint, Java write into the Rust root, or Rust write into the Java root fails.

### Actual block-change parity

The driver performs real Paper block commands/events, records coordinates and before/after block state, waits for bridge acknowledgement and tile regeneration, and compares every affected tile plus tiles whose shading dependency changed. A scenario that only compares prebuilt fixtures or produces no block-state transition is invalid.

Pass requires zero path, JSON, and decoded RGBA mismatches after quiescence.

### Lifecycle convergence

World reload must advance the world epoch and reject stale prior-epoch work. Player churn must preserve visibility/privacy semantics. Cancellation must stop the accepted render without a later duplicate completion. Resumption must preserve durable progress and finish once. Shutdown must prevent new HTTP, bridge, child-process, and filesystem activity.

Any stale epoch output, missing dirty coordinate, duplicate completion, leaked job, or late write fails.

### Sidecar recovery

The driver derives the sidecar PID from the supervised production process record and kills only that process. Paper must remain alive and responsive. Recovery evidence includes kill time, restart attempts/backoff, process generation, authenticated handshake, replay boundaries, sequence/session handling, dirty/render recovery, and final output hashes.

Pass requires bounded restart under the configured policy, authenticated reconnect, complete baseline replay, and exact final convergence. A Java fallback public listener is forbidden in Rust mode.

### Strict quiescence

A checkpoint is quiescent only when all conditions hold across two samples separated by the configured quiet interval:

- Paper is alive and responsive;
- the required sidecar generation is ready;
- bridge queues are empty and accepted messages are acknowledged or terminally cancelled;
- no active render, pending cancellation, dirty coordinate, or durable job remains;
- output inventory, hashes, and mtimes are unchanged;
- no temporary output exists;
- recorder buffers are flushed;
- no lifecycle step is outstanding.

Missing health or metric data means non-quiescent, not unknown.

### Performance and memory

After correctness passes, repeated paired runs collect warm-up and measured samples for event-to-frame latency, frame-to-output latency, scenario wall time, render throughput, queue high-water, frame bytes, process CPU where available, Paper RSS, and sidecar peak/steady RSS. Sampling uses `/proc/<pid>/status` on Linux and records its interval and collector metadata.

Thresholds are declared in the fixture manifest before measurement. Reports retain raw samples, medians, percentiles where sample count supports them, maxima, machine metadata, and verdict. A faster run with any mismatch or incomplete lifecycle fails.

## Fail-closed evidence bundle

Each run produces a unique diagnostics root containing:

- `manifest.json`: inputs, hashes, commits, environment, commands, thresholds, scenario, and normalization rules;
- `lifecycle.jsonl`: steps, acknowledgements, epochs/revisions, process generations, barriers, and quiescence samples;
- `recordings/`: Java bridge recordings and hashes;
- `java/` and `rust/`: output inventories, HTTP transcripts, logs, metrics, and canonical hashes;
- `comparisons/`: path, JSON, PNG/RGBA, replay, and ownership reports;
- `performance.json`: paired timing, throughput, queue, frame, CPU, and RSS measurements;
- `checksums.sha256`: every accepted evidence artifact;
- `verdict.json`: each required gate and the aggregate result.

The aggregate verdict passes only if every required artifact exists and hashes correctly, every lifecycle step completed, every quiescence checkpoint passed, Rust ownership is proven, replay passed, and `mismatch_count` is exactly zero. Partial bundles are `incomplete`, never passed.

## Remote production-like gate

The same scenario and evidence schema apply to a supplied production-like Paper environment. Until a host, access credentials, pinned artifacts, safe mutation scope, isolated roots, and process/RSS collection permission are supplied, the remote gate result is `blocked_infrastructure`. Local success does not imply remote success.

## Acceptance matrix

| Claim | Authoritative evidence | Pass condition |
|---|---|---|
| Rust HTTP ownership | socket/PID ownership, endpoint transcript, write-root inventory | Rust owns listener/output; Java owns neither |
| Tile parity | real block event, bridge records, affected/dependent RGBA and JSON comparison | zero mismatches after quiescence |
| Lifecycle convergence | epoch, player, render, shutdown, and quiescence records | no stale/lost/duplicate/late work |
| Sidecar recovery | kill/restart generations, authenticated replay, final outputs | Paper survives and Rust reconverges within policy |
| Performance/RSS | paired raw measurements and declared thresholds | complete measurements within thresholds after correctness |
| Zero mismatch evidence | hash-valid bundle and immutable compare/replay reports | complete bundle and `mismatch_count: 0` |
