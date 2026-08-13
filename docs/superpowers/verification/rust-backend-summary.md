# Rust backend migration verification

Current branch: `rust-backend-migration`

## Verified gates

- `cargo test --manifest-path rust/Cargo.toml --workspace`: **222 tests passed across 32 suites**.
- `./gradlew :squaremap-common:test --no-daemon`: historical pass; current full Gradle verification is blocked at manifest generation.
- `./gradlew build --no-daemon`: **currently blocked** by the intentional duplicate-artifact SHA-256 guard in `squaremap-common:generateBackendManifest`; the earlier pass is historical, not current evidence.
- `cargo test --manifest-path rust/Cargo.toml -p squaremap-compare`: 11 tests passed.
- Rust bridge HTTP lifecycle tests: 2 passed.
- Rust configuration compatibility tests: 2 passed, including atomic rejection retaining the prior active revision.
- Web verification: `cd web && bun run lint && bun run build`; build passed and lint completed with three warnings.
- Comparator CLI produced `mismatch_count: 0` for the checked-in bridge view fixture roots. Both roots currently use checked-in documents, so this validates comparator behavior, not independent backend generation.
- Renderer workload benchmark: passed at 188.02 items/s for 26 cases through `RenderTileInstaller` with `MemoryTileStore`, threshold 100 items/s; excludes Scheduler, bridge, repository, and HTTP.
- Real Rust sidecar restart smoke: passed with supervisor-owned child termination, a distinct replacement connection, and SQLite repository state persisted.
- Backend manifest packaging: **blocked**; the current local five-target inputs are duplicate placeholder content, and generation now rejects duplicate SHA-256 artifacts.

## Not yet passed

- Full independent Java-primary/Rust-shadow comparison across block/chunk churn, player/marker changes, reload, quiescence, and sidecar fault while Paper remains alive.
- Remote five-target release publication and asset URL resolution.
- Rust observation-release default and clean deletion of legacy Java backend paths.

The migration is not complete until every item above has direct current-commit evidence.
