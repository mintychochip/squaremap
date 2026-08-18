# Sidecar Binary Install/Update Scripts Design

**Date:** 2026-08-17

## Goal

Provide `install.sh` and `update.sh` at the repository root that download the official squaremap Rust sidecar binaries from GitHub releases and stage them in the layout the Gradle build expects.

## Background

- The CI `rust-backend` job cross-compiles `squaremap-server` for five targets.
- The `publish-rust-backends` job uploads `squaremap-server-<target>(.exe)` assets to the GitHub release.
- `common/build.gradle.kts` reads binaries from `rust/backend/rust-backend-<target>/squaremap-server-<target>(.exe)` and uses them to generate the packaged `squaremap-backends.json` manifest.
- Locally obtaining these binaries currently requires re-running CI or manually downloading. These scripts replace that manual work.

## Design principle

- Verification must not require `--insecure` for existing releases that lack a manifest.
- The default verifier is the GitHub Releases API: each script run fetches the release metadata once and verifies every downloaded file's size against the API-reported `size`.
- If a release also publishes `squaremap-backends.json` as an asset, the scripts additionally verify each binary's SHA-256 and length.
- Local manifests can be supplied with `--manifest <path>`.
- All verification can be skipped only by explicitly passing `--insecure`.

## Scripts

### `install.sh`

```
./install.sh [OPTIONS] [VERSION]
```

- `VERSION` defaults to the `version` value in `gradle.properties` and can be overridden by the `VERSION` environment variable.
- If the resolved version ends with `-SNAPSHOT`, the suffix is stripped when forming the GitHub release tag (e.g., `1.3.16-SNAPSHOT` → `v1.3.16`). If that release does not exist, the script fails and the user must provide an explicit release version.
- Default target is the host triple.
- Options:
  - `--all-targets`: download all five supported targets.
  - `--target <triple>`: download a specific target (can be repeated).
  - `--force`: overwrite an existing `rust/backend/rust-backend-<target>/` directory.
  - `--insecure`: skip GitHub API and manifest verification.
  - `--manifest <path>`: use a local `squaremap-backends.json` file for SHA-256/length verification.
  - `--github-token <token>`: pass a token for higher GitHub API rate limits.
- Fetches release metadata from:
  `https://api.github.com/repos/jpenilla/squaremap/releases/tags/v${VERSION}`
- Downloads assets from the `browser_download_url` in the release metadata.
- Creates:
  `rust/backend/rust-backend-<target>/squaremap-server-<target>(.exe)`
- Fails if the target directory already exists and `--force` is not given.
- Does not delete or touch other existing target directories.

### `update.sh`

```
./update.sh [OPTIONS] [VERSION]
```

- Same version, target, and verification resolution as `install.sh`.
- Downloads all selected binaries into a staging directory named `rust/backend/.update-<pid>/`.
- Verifies every selected binary before any replacement:
  1. Size matches the GitHub Releases API `size` field.
  2. If `squaremap-backends.json` is present in the release's asset list, download it and verify the binary's SHA-256 and `length`. The manifest's `pluginVersion` must match the requested version.
  3. If `--manifest <path>` is given, verify against that file.
  4. If `--insecure` is given, skip both API and manifest verification.
- Transactional replacement across all selected targets:
  - For each selected target, move the existing `rust/backend/rust-backend-<target>/` to `rust/backend/rust-backend-<target>.old-<pid>/` and then move the staged `rust/backend/.update-<pid>/rust-backend-<target>/` into place.
  - An `EXIT`/`ERR` trap records every target whose old directory was moved. If the script exits with an error, the trap removes any partial new directory and restores every recorded `rust-backend-<target>.old-<pid>/` to `rust-backend-<target>/`.
  - If a staged-to-target move itself fails, the recorded old directory is restored before exit.
  - After all replacements succeed, the script removes every `rust-backend-<target>.old-<pid>/`.
- On any failure, the staging directory is removed and the existing `rust/backend/` tree is restored to the state it had before the script ran.


## Supported targets

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`

## Host target detection

1. If `rustc` is on `PATH`, use the `host:` value from `rustc -vV`.
2. Otherwise map `uname -s` and `uname -m` to the supported triple, with sensible defaults. Error on unsupported combinations.

## GitHub API parsing

- Prefer `jq` for JSON extraction. Fall back to `python3` (or `python`) if `jq` is not available.
- If neither `jq` nor `python` is available, the script fails with a message telling the user to install one of them or pass `--insecure`.
- Cache the release metadata in a temp file for the duration of the script so only one API call is made.
- Respect `GITHUB_TOKEN` env var / `--github-token` for authenticated requests.

## Error handling

- `set -euo pipefail`.
- Use `curl --fail --location` and fall back to `wget` if `curl` is unavailable.
- Remove temporary/staging directories on error.
- Progress and error messages go to `stderr`.

## Integration with build

- After running `install.sh` or `update.sh`, the normal `./gradlew build` continues as before.
- `common/build.gradle.kts:stageBackendBinary` copies from `rust/backend/rust-backend-<target>/` into `common/build/backend/`.
- `common/build.gradle.kts:generateBackendManifest` builds the packaged manifest from the staged binaries.
