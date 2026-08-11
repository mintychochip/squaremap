# Task 5 report: Rust durable state and legacy import

## Contract tests and verification

The required test-first RED was run before the crate existed:

```text
$ cargo test --manifest-path rust/Cargo.toml -p squaremap-state --test recovery
error: package ID specification `squaremap-state` did not match any packages
Process exited with code 101
```

After implementation, the required recovery command passed:

```text
$ cargo test --manifest-path rust/Cargo.toml -p squaremap-state --test recovery
running 15 tests
...............
test result: ok. 15 passed; 0 failed
```

The session integration was exercised with the specifically permitted focused suite:

```text
$ cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test session_ordering
running 12 tests
............
test result: ok. 12 passed; 0 failed
```

No formatter, linter, or project-wide test suite was run.

## Evidence covered

- `squaremap-state` is a Rust 2024/MSRV 1.88 workspace member and uses bundled rusqlite; Cargo.lock is committed.
- The checked-in migration creates exactly the required six state tables and schema version 1. Repository opening configures and validates WAL, `synchronous=NORMAL`, foreign keys, 5-second busy timeout, and application ID `SQMP` (`0x53514D50`). Newer, malformed, wrong-ID, unrelated, nonempty, and multiple-version databases are rejected.
- Recovery tests prove reopen persistence, deterministic ordering, current worlds/dirty/checkpoints, running-job conversion to resumable recovery, revision-7 duplicate coalescing, stale revision-6 ignore, revision-8 replacement, completion protection against newer revisions, epoch purge and stale epoch guards, and checked u64 overflow without mutation.
- SQLite work runs behind a crate-owned semaphore acquired before each `spawn_blocking`; transactions are created and committed wholly inside blocking closures.
- Legacy parsing reads exact bytes, hashes exact bytes with SHA-256, validates Gson dirty arrays and complex-map-key resume entry arrays with bounds and unknown-shape rejection, parses both files before opening the import transaction, deduplicates dirty coordinates with bounded hashing, rejects duplicate resume coordinates, rejects symlink candidates, hard-limits bytes actually read, preserves resume entry order in a typed deterministic payload, uses length-prefixed world namespace/value plus epoch and fixed filename import keys, and never modifies files. Valid repeat imports are hash-idempotent; changed valid content updates only an untouched legacy-owned resumable zero-progress job whose payload hash matches its companion ownership marker; live updates clear ownership, unowned jobs remain unchanged, malformed marker hashes fail before mutation, and terminal/progress guards prevent resurrection/regression.
- `Session::process_with_repository` handles only New `ChunkDirty` payloads through `Repository::mark_dirty`; the Ack is emitted only after the repository transaction returns success. Duplicate sequences use the existing cursor path and do not call persistence. Stale/repository/overflow errors and unrelated New payloads produce no Ack or checkpoint. Existing panic, cancellation, and gap behavior remains covered by `session_ordering`.

## Self-review

- Review regressions were first run against commit `064c207`: 13 tests ran and 3 failed; after those fixes recovery passed 13/13 and session passed 12/12.
- Final follow-up RED was run against commit `4061bc5` with the corrected unrelated-Full regression: 15 tests ran and 1 failed before implementation. After the fixes, recovery passed 15/15 and `session_ordering` passed 12/12.
- Final self-review: render ownership is cleared only for the deterministic Resume job, marker lookups read only SQLite length plus a 33-byte prefix into a fixed array, and all prior/ownership marker paths use the bounded helper.

Atomic task commit: this commit; SHA recorded externally
