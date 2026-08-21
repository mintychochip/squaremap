# Rust backend migration verification

## Current evidence (2026-08-20)

- Rust workspace: `cargo test --manifest-path rust/Cargo.toml --workspace` passed with exit 0.
- Snapshot client regression `cached_registry_is_bound_to_later_request` now passes: later requests bind to the already-cached registry generation, matching Java's `RegistryGate` (commit 7b9195e; 16/16 snapshot-client tests green).
- Grass parity: Rust now ports vanilla `BiomeSpecialEffects.GrassColorModifier` (NONE/DARK_FOREST/SWAMP) plus the `Biome.BIOME_INFO_NOISE` simplex chain bit-exactly (`squaremap-render/src/vanilla.rs`). The bridge carries the modifier per biome (`BiomeDescriptor.grass_color_modifier`), the live renderer computes resolved grass from descriptors instead of requiring injected resolutions, and the regenerated Java corpus (`testdata/bridge/v2/render/registry/registry.bin`) asserts Rust-computed grass equals the Java oracle on every sample with full pixel equality across all 26 cases (commit 43ec941). Previously every grass-tinted block errored under default `MAP_BIOMES=true`, producing map holes.
- Java common suite: `./gradlew :squaremap-common:test` BUILD SUCCESSFUL (exit 0), including the regenerated-corpus oracle test, `FrameLimitsTest` (`fromPeerPolicy` clamp implemented and adopted by `SidecarSupervisor`, commit 029b207), and `HtmlComponentSerializerImplTest` (component text is now HTML-escaped before sanitization, commit 0be07bd).
- Java/Rust fixture comparator: 0 mismatches across 6 paths (deterministic fixture parity, not live shadow parity).
- Fresh Paper run (prior session): Paper 26.2 and the Rust sidecar reached readiness; live HTTP served the frontend root, favicon, JavaScript, CSS, source map, tile JSON, and icons.

## Authoritative live failure

A clean isolated Paper run using the current plugin and Rust sidecar executed:

```text
squaremap radiusrender minecraft:overworld 1 0 0
```

The sidecar then disconnected with:

```text
RegistryUnavailable("correlation=2, world=WorldIdentity { namespace: \"minecraft\", value: \"overworld\", epoch: 1 }, revision=3, cached_revision=Some(3), registry_issue=None")
```

The Java supervisor reported a fatal bridge protocol error and the render job was cancelled with zero completed chunks. Evidence is preserved at `/tmp/squaremap-parity-live-repro-20260820.log`.

Root cause: Rust `SnapshotClient::request_once` did not bind a later request to an already cached registry generation; Java's `SnapshotRequestHandler` uses `RegistryGate` to ensure and reuse the registry before encoding the snapshot. **Fixed** at component level with a regression test (commit 7b9195e); a fresh live Paper render probe is still required to confirm end-to-end.

## Current verdict

Deterministic protocol, state, JSON, HTTP, and fixture-render components are green within their declared scopes. The two known render-path divergences (cached-registry binding; grass color modifiers) are fixed with component-level regression evidence. Complete Java parity is **not verified**: a fresh live Paper render/control probe is required to confirm the fixes end-to-end, and addon API parity, all loader lifecycle parity, release publication/remote resolution, rollback, observation/cutover, and full Java-primary/Rust-shadow coverage also remain unproven.

Do not enable Rust as an observation-release default or delete legacy Java paths based on this evidence.
