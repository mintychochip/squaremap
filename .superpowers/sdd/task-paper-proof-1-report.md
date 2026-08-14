
## Review fix

- Placeholder `hello.bin`/`fill.invalid` artifact entries now fail closed as `blocked_fixture_artifacts` with three explicit prerequisites; they are never returned as executable artifacts.
- Added `world-fixture.json` deterministic contract without pretending it is a Paper executable.
- Added runtime metadata (Java/JVM/OS/arch/server/command protocol), canonical/physical root overlap checks, exact ordered unique lifecycle validation, and focused coverage for hash/missing artifacts, unsupported version, root overlap, normalization, lifecycle order/duplicate/missing, and invalid port values.

Exact verification command:

```bash
./gradlew :squaremap-common:test --tests '*ProofFixtureTest'
```

Exact output:

```text
BUILD SUCCESSFUL in 1s
16 actionable tasks: 3 executed, 13 up-to-date
```

Fix commit:

```text
dbca115 Fix Paper proof fixture validation
```

Concerns:

- Real Paper/plugin/sidecar binaries are still unavailable in this worktree, so the checked-in manifest intentionally remains blocked and cannot launch Paper.
- The focused test suite uses the checked-in blocked manifest to assert fail-closed behavior; temporary valid executable bytes should be supplied by future tests when exercising a non-blocked load path.
- The prior `squaremap-compare` baseline was not modified.
