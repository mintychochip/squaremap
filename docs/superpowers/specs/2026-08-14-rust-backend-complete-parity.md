# Rust Backend Complete Parity Specification

## Intent

Replace the Java backend with the managed Rust sidecar without changing Squaremap’s public web, addon, loader, command, configuration, rendering, lifecycle, recovery, or operational behavior.

## Ownership boundary

Java remains responsible for loader lifecycle, Minecraft event capture, safe immutable snapshot extraction, privacy/visibility decisions, public addon API objects, command permissions/localization, and sidecar supervision. Rust owns canonical backend state, render scheduling and persistence, dirty recovery, rendering and PNG pyramid production, JSON/assets, HTTP, output locking/atomic writes, backend metrics, and replay/comparison tooling.

No mode permits Java and Rust to write the same output root or own the same HTTP listener. Java remains default until every pre-cutover gate passes.

## Observable parity domains

1. **Transport and security:** loopback-only authenticated framing, one-use bootstrap token, major/minor compatibility, bounds and decompression validation, path confinement, secret-free logs, typed failures.
2. **Recovery:** stable bridge identity, reconnect session identity, durable checkpoint, accepted-but-unacknowledged replay, bounded queues, saturation resync, owner/world/epoch isolation, leases and retry state.
3. **Controls:** full/radius render, pause/resume, cancel, reset, reload/config sync, progress, health, structured results, timeouts, restart recovery.
4. **Rendering:** exact pixels, north/south shading, visibility, block/fluid/glass/biome behavior, PNG pyramid, negative and boundary coordinates, modded descriptors.
5. **State and assets:** worlds, players, privacy-filtered fields, settings, markers/geometries, icons, addon updates, epochs and volatile fields.
6. **HTTP/output:** paths, GET/HEAD, ETag/304, cache-control, existing/missing tiles, static/dev frontend, disabled HTTP, external web server, atomic replacement and sole-writer locking.
7. **Configuration and API:** YAML semantics, locale/messages, invalid-config atomicity, public addon API immutability and update propagation, loader commands and permissions.
8. **Platforms:** Paper, Fabric, NeoForge and Sponge lifecycle/events, safe scheduling, thread/process/port cleanup.
9. **Operations:** startup/readiness, shutdown, crash loops, reconnect, rollback, performance/resource thresholds, native distribution and verified execution.
10. **Cutover:** canary, observation release, state migration/backups, deletion of Java backend implementation, final sole ownership.

## Evidence semantics

Every behavior has one parity-matrix row with Java oracle, Rust implementation, test scenario, denominator, dependencies, allowed predeclared volatility, evidence kind, artifact hashes and current status. Missing observations fail closed. Fixture evidence cannot satisfy live Paper, loader, remote release, performance, canary or observation gates. No post-hoc mismatch explanation or ignored mismatch list is permitted.

Evidence fields are scoped by kind. All records include commit, dirty-tree state, timestamp, toolchain, exact command/exit status, input/config hashes and artifact hashes. Live evidence additionally includes process generations/PIDs, roots/ports, scenario and quiescence hashes and metric sources. Remote evidence includes immutable URL/digest, request transcript, target-host attestation and execution result. Canary and observation runs bind each process start to the verified digest.

## Correctness invariants

- Rust acknowledges durable messages only after commit.
- Replay ordering and paging are bounded, contiguous, idempotent and owner/world/epoch isolated.
- Legacy rows never receive inferred authenticated identities.
- Reconnect restores configuration before readiness, controls and state traffic.
- Full render enumerates the enabled world, not merely current dirty rows.
- Cancel/reset/shutdown cannot race a later tile publication.
- Output and HTTP ownership transfer has no overlap.
- Public Java addon and privacy behavior remains observable through Rust output.
- Deterministic comparison uses decoded RGBA and semantic JSON.
- Quiescence requires two complete equal production snapshots separated by a quiet interval.
- Automatic mid-request Java/Rust failover is forbidden.

## Performance contract

Matched Java-primary and Rust-primary Paper twins use the same pinned world, configuration, action transcript, output medium and quiescence boundaries. After fixed warmup, at least three measured runs produce confidence intervals. Rust render throughput must not regress; HTTP and dirty-to-visible p95 may regress at most 10%; tick p95 at most 5%; combined Paper-plus-sidecar steady RSS at most 15%; every queue must converge.

## Release and cutover contract

Five immutable native target assets must be remotely published, independently fetched and executed on matching target hosts. Verification-to-execution must bind the verified bytes or immutable handoff. The exact digest is used in canary and observation runs.

Rust becomes opt-in primary only after live parity, loader, recovery, performance and release gates pass. It becomes default only after the predefined canary window. Java rollback remains for one observation release. Only then are migrated-state backups created and Java HTTP/cache/render/scheduler/tile/persistence implementations deleted. The entire parity matrix and remote execution proof rerun on the deletion commit.

## Completion

Complete parity means every matrix row has current-commit evidence, deterministic and live comparison has zero disallowed mismatch, all loader/recovery/performance/release/canary/observation gates pass, rollback has been exercised, the Java backend implementation is removed, and the post-delete matrix passes.
