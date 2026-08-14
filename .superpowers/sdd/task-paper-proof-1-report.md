
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
## Follow-up review fix

- Checked-in world fixture now has a real SHA-256 and is parsed and cross-checked for version, seed, name, timezone, locale, and nonempty mutation coordinates.
- Artifact metadata cross-checks the top-level named SHA when present against the canonical `artifacts` entry; blocked placeholder artifacts remain fail-closed.
- Artifact/world paths resolve the real existing ancestor, rejecting symlink-parent escapes even when the requested child is absent. Root overlap checks independently resolve each existing ancestor, including symlink aliases.
- Exact runtime metadata remains recorded in the manifest; executable artifact bytes remain intentionally blocked.

Verification:

```bash
./gradlew :squaremap-common:test --tests '*ProofFixtureTest'
```

```text
BUILD SUCCESSFUL in 3s
16 actionable tasks: 2 executed, 14 up-to-date
```

Fix commit: `c55a187 Harden Paper proof fixture validation`
