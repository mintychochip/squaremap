# Rust backend migration verification

Current branch: `rust-backend-migration`

## Verified gates

- `cargo test --manifest-path rust/Cargo.toml --workspace`: 211 tests passed.
- `./gradlew :squaremap-common:test --no-daemon`: passed.
- `./gradlew build --no-daemon`: passed.
- `cd web && bun run lint && bun run build`: build passed; lint reports three warnings in existing web files.
- `cargo test --manifest-path rust/Cargo.toml -p squaremap-compare`: 6 tests passed.
- The comparison CLI produced `mismatch_count: 0` for the checked-in bridge view fixture roots. Both roots currently use the checked-in documents, so this validates comparator behavior, not independent backend generation.
- The benchmark CLI requires explicit iterations and a positive threshold and emits a JSON verdict. It currently measures only a bounded comparison-tool workload.

## Not yet passed

- Independently generated Java/Rust recorded-output parity fixture.
- Live Java-primary/Rust-shadow evidence.
- Sidecar fault/restart/recovery evidence.
- Real backend rendering/bridge performance benchmark and threshold report.
- Build-generated backend binary manifest and resolver integration.
- Rust observation-release default and clean deletion of legacy Java backend paths.

The migration is not complete until every item above has direct current-commit evidence.
