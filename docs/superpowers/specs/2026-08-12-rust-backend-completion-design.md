# Rust Backend Migration Completion Design

Date: 2026-08-12
Status: Approved for implementation

## Intent

Complete the existing managed Rust sidecar migration rather than stopping at the current unit-level ports. Rust must own backend HTTP serving, canonical state, rendering, tile output, replay/comparison, and operational evidence; Java must remain the thin Minecraft/loader and public-addon bridge.

## Current evidence and scope

The isolated `rust-backend-migration` branch already contains protocol/framing, sidecar supervision, outbox/session ordering, state recovery, HTTP contract, state exporters, snapshot encoding, render primitives/chunk rendering, tile pyramids, and scheduler work. The workspace currently fails because `squaremap-compare` declares missing `compare`, `recording`, and `report` modules. Java still starts `IntegratedServer` regardless of backend mode, so Rust mode cannot yet own HTTP/output.

Completion therefore covers the remaining plan tasks and their acceptance gates:

1. Implement comparison/replay tooling and deterministic contract tests.
2. Make HTTP/output lifecycle ownership mode-specific and test Rust-mode isolation.
3. Add fault/recovery evidence, metrics/benchmark gates, binary manifest/resolution, and configuration compatibility ownership.
4. Run replay, live-shadow, fault, and isolated-performance gates; publish machine-readable evidence.
5. Make Rust default for one observation release only after those gates pass.
6. Delete Java backend and legacy migration paths only after the observation-release evidence is recorded, preserving Java loader integrations, public API, and existing web URLs/files.

No shortcut removes the comparison crate, leaves Java HTTP active in Rust mode, or claims completion from unit tests alone.

## Architecture

The plugin launches a version-matched Rust child process over an authenticated loopback TCP stream using the existing length-delimited Protobuf protocol. Java continues to capture Minecraft-safe state and snapshots and publish bounded events. Rust owns HTTP, durable state, render scheduling, PNG output, JSON/assets, and comparison tooling.

Backend modes have exclusive output roots:

- `java`: Java backend owns HTTP and Java output.
- `shadow`: Java remains authoritative; Rust receives a mirror in a separate root and never binds the public port.
- `rust`: Rust owns HTTP and Rust output; Java `IntegratedServer`, `JsonCache`, and Java render output do not start.

Readiness is authoritative: Rust mode is considered started only after state recovery, bridge bootstrap, HTTP bind, and `Ready`; shutdown only invokes the owner’s lifecycle. Rust failures are fatal in `rust` mode and isolated in `shadow` mode.

## Comparison and evidence

`squaremap-compare` provides:

- bounded recording read/write with manifest and protocol-version validation;
- deterministic replay at speed zero or a positive multiplier;
- semantic JSON comparison preserving array order and numeric semantics;
- PNG comparison after RGBA decode with first mismatch coordinate and count;
- path-set comparison with missing/extra reporting;
- immutable JSON reports with no-overwrite semantics;
- CLI exit codes 0 equal, 1 mismatch, 2 invalid input/tool failure.

Evidence reports include schema version, commits, environment, compared paths, mismatch details, timing, and gate verdicts. Fixture recordings and Java/Rust output trees are deterministic and checked in.

## Configuration, distribution, and gates

Rust receives versioned configuration and bridge policy with explicit compatibility validation. Java remains the source of loader-specific values and public API state until the protocol transfer is complete. Binary resolution accepts only configured absolute paths or versioned cache/release artifacts whose SHA-256 matches the embedded manifest; no unverified executable runs.

The completion gate requires:

- Rust workspace tests and focused compare tests pass.
- Java bridge and lifecycle tests pass.
- Gradle build passes for all supported loaders.
- Web lint/build passes.
- Replay parity has zero semantic mismatches.
- Live shadow has no forbidden output overlap or public-port ownership conflict.
- Fault/recovery proves bounded restart and no half-initialized HTTP exposure.
- Isolated performance stays within the plan’s configured thresholds.

Only after all gates are directly evidenced may the branch perform the observation-release default and clean cutover. The clean cutover removes obsolete Java backend classes, Undertow dependency, legacy mode, and transitional aliases; it does not remove loader integration or the public Java addon API.

## Testing strategy

Every production behavior follows test-first development. New tests defend observable contracts: comparison semantics and limits, ownership/lifecycle transitions, recovery behavior, manifest verification, config compatibility, and cutover boundaries. Existing protocol/render/state tests remain regression coverage. Final verification exercises the actual Rust CLI/server, Gradle build, and web build rather than relying only on isolated unit tests.

## Risks and decisions

- The final deletion is intentionally last; deleting Java backend paths before replay/live-shadow evidence would remove rollback capability and violate the staged migration contract.
- The existing uncommitted Java/render edits on `master` are unrelated user work and remain untouched. Work proceeds only in the existing isolated branch.
- The comparison crate is implemented, not bypassed, because parity evidence is a required acceptance criterion.
