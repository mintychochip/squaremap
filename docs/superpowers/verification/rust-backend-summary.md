# Rust backend migration verification

Current branch: `rust-backend-migration`

## Verified gates

- `cargo test --manifest-path rust/Cargo.toml --workspace`: 211 tests passed.
- `./gradlew :squaremap-common:test --no-daemon`: passed.
- `./gradlew build --no-daemon`: passed.
- `cd web && bun run lint && bun run build`: build passed; lint reports three warnings in existing web files.
- `cargo test --manifest-path rust/Cargo.toml -p squaremap-compare`: 4 tests passed.

## Not yet passed

- Java/Rust recorded-output parity fixture with a zero-mismatch report.
- Live Java-primary/Rust-shadow evidence.
- Sidecar fault/restart/recovery evidence.
- Isolated performance benchmark and threshold report.
- Build-generated backend binary manifest and resolver integration.
- Rust observation-release default and clean deletion of legacy Java backend paths.

The migration is not complete until every item above has direct current-commit evidence.
