# Baseline Fix Report: Linked-Worktree Commit Resolution

## Scope

The fix restores commit-hash resolution for normal checkouts and linked Git worktrees. Only build-logic sources, build-logic tests, build-logic test setup, and this required report were changed. No migration Task 1 files were touched.

## Files changed

- `build-logic/src/main/kotlin/ext.kt`
  - Keeps Indra's commit as the first-choice source.
  - Adds an isolated, injectable native-Git resolver.
  - Uses `providers.exec` for the production fallback so Gradle configuration cache remains compatible.
  - Runs `git rev-parse --verify HEAD` from `rootProject.projectDir`, requires exit code 0, trims output, validates a 40- or 64-character hexadecimal object ID, and shortens only after validation to seven characters.
  - Retains `Could not determine commit hash` when neither source resolves.
- `build-logic/src/test/kotlin/ExtTest.kt`
  - Focused tests for command arguments and working directory, trimming and 40-character IDs, 64-character IDs, nonzero exit codes, and malformed output.
- `build-logic/build.gradle.kts`
  - Adds the Kotlin JUnit 5 test dependency and test platform configuration required by the focused tests.
- `.superpowers/sdd/baseline-fix-report.md`
  - This report.

## TDD evidence

### RED

After adding the focused test but before production changes, the following command was run:

```text
./gradlew -p build-logic test
```

Result: `BUILD FAILED` during `:compileTestKotlin`. After the test dependency setup was added (still before production changes), the command was run again to isolate the intended failure. The result was:

```text
> Task :compileTestKotlin FAILED
e: .../build-logic/src/test/kotlin/ExtTest.kt:11:18 Unresolved reference 'resolveNativeGitCommit'.
e: .../build-logic/src/test/kotlin/ExtTest.kt:14:7 Unresolved reference 'GitCommandResult'.
e: .../build-logic/src/test/kotlin/ExtTest.kt:23:41 Unresolved reference 'GitCommandResult'.
e: .../build-logic/src/test/kotlin/ExtTest.kt:26:16 Unresolved reference 'resolveNativeGitCommit'.
e: .../build-logic/src/test/kotlin/ExtTest.kt:29:7 Unresolved reference 'GitCommandResult'.
e: .../build-logic/src/test/kotlin/ExtTest.kt:31:16 Unresolved reference 'resolveNativeGitCommit'.
BUILD FAILED
```

This RED result was caused by the missing resolver API and result type, not by a test typo.

### GREEN

After implementing the fallback, the focused test module was run:

```text
./gradlew -p build-logic test
```

Result:

```text
BUILD SUCCESSFUL in 2s
12 actionable tasks: 4 executed, 8 up-to-date
```

The final run included all tests in the build-logic test module; the `:test` task completed successfully.

## Linked-worktree verification

From `/home/jlo/dev/squaremap/.worktrees/rust-backend-migration`, the required reproduction was run:

```text
./gradlew help
```

Result:

```text
BUILD SUCCESSFUL in 2s
10 actionable tasks: 1 executed, 9 up-to-date
```

The first implementation attempt used `ProcessBuilder` directly during configuration and was rejected by Gradle configuration-cache checks. The final implementation uses Gradle's `providers.exec`, and the required help reproduction succeeds with configuration cache enabled.

The linked worktree's native Git metadata was also checked:

```text
git rev-parse --verify HEAD | tr -d '\\n' | wc -c
```

Result: `40`

```text
git rev-parse --verify HEAD | cut -c1-7
```

Result: `f1505a4`

Version decoration remains wired through `squaremap.platform.gradle.kts` calling `decorateVersion()`, and `lastCommitHash()` still appends `substring(0, 7)` to snapshot versions. The successful linked-worktree configuration confirms the decorator can now resolve the commit instead of failing during plugin application.

## Self-review

- Indra remains first: `lastCommitHash()` reads `IndraGitExtension.commit().orNull` and only invokes native Git when that value is absent.
- The native command is passed as separate arguments (`git`, `rev-parse`, `--verify`, `HEAD`); no shell command string is constructed.
- The production fallback executes from `rootProject.projectDir`, which is the linked-worktree root.
- Nonzero process results and process exceptions become an unresolved fallback; malformed, empty, short, long, or non-hex output is rejected before shortening.
- A valid native object ID is shortened exactly once to the first seven characters. Indra-provided commit behavior remains the existing seven-character substring path.
- The resolver and process result are isolated from `Project.lastCommitHash()` and can be tested with an injected command runner.
- Focused tests cover 40-character and 64-character IDs, trimming, exact command arguments, working directory, nonzero exit status, and malformed output.
- Changed-file status was checked before reporting. Only build-logic paths and this required report were changed; no migration Task 1 files are present in the diff.

## Concerns

None identified. Per the brief, formatters, linters, and the project-wide build/test suite were not run; verification was limited to the focused build-logic test module and the linked-worktree `./gradlew help` reproduction.
