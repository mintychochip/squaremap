# Task 6 report: Rust HTTP/output/dev frontend contract

## RED

The required command was run before production modules existed:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test http_contract --test dev_frontend
```

It failed during test compilation because `squaremap_server` had no library crate and therefore `HttpServer`/`OutputRoot` were absent. This was the genuine missing-module RED.

## GREEN

At the recorded review base `4c7b93b`, the required focused command had the exact baseline result:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test http_contract --test dev_frontend
cargo test: 13 passed (2 suites, 15 warnings)
```

That baseline contained exactly two Task 6 unreachable-statement warnings; the remaining warnings were pre-existing protocol/session warnings. The follow-up regressions now pass after the third review:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test http_contract --test dev_frontend
cargo test: 21 passed (2 suites, 13 warnings)
```

The tests cover static index/JSON/PNG responses, GET/HEAD, explicit MIME and length headers, quoted/weak/wildcard ETags and 304, tile cache headers and missing-PNG behavior, non-tile 404, method/path rejection, root-confined writes, symlink rejection, disabled mode, single-owner locking, dev URL rejection/readiness timeout, delayed readiness and multiple candidates, immediate exit, bounded multibyte/sustained logs after readiness, HTTP POST body/query/status forwarding, retained infinite-response cancellation, barrier-controlled stalled WebSocket-handshake cancellation, WebSocket header/subprotocol forwarding, successful and startup-failure descendant process-group shutdown, concurrent atomic writes, and local tile exclusions.

## Smoke

`--help` remained valid and printed both `bridge` and `serve-fixture` usage. The latest fixture smoke launched `serve-fixture --root web --bind 127.0.0.1:0`; it printed exactly one readiness line:

```text
READY http_addr=127.0.0.1:46337
```

Fetching `/` returned HTTP 200; the combined header/body capture was 2100 bytes. The process was terminated with SIGTERM and exited 0 after the graceful signal path.

## Security review

- URI percent decoding occurs once; malformed encodings, NUL, backslash, encoded separators, absolute/prefix/current/parent components, and symlink components are rejected.
- HTTP reads and output writes share `validate_relative` and component checks; reserved owner/temp internals are rejected.
- Unix output reads/writes traverse directory handles with `openat(..., O_NOFOLLOW)`; Windows uses `cap_std::fs::Dir` capability-relative traversal and same-parent rename; temporary files are exclusive and cleaned on errors.
- Output roots acquire a cross-platform exclusive owner lock before cleanup, so stale cleanup cannot remove another live writer's temporary file.
- Opened file handles supply metadata used for strong quoted ETags; matching `If-None-Match` returns 304 before body reading.
- Missing tiles are only synthesized for the exact `tiles` first component and `.png` extension; `/tiles2` is not an exclusion.
- Proxy bodies use bounded streaming adapters rather than whole-body copies; request/response hop-by-hop filtering parses `Connection` extension tokens and suppresses Host forwarding.


The Windows-target check passed with the required cross-compilation environment:

```text
CC_x86_64_pc_windows_gnu=gcc AR_x86_64_pc_windows_gnu=ar cargo check --manifest-path rust/Cargo.toml -p squaremap-server --target x86_64-pc-windows-gnu
Finished `dev` profile [unoptimized + debuginfo] target(s)
```
## Lifecycle review

Dev startup runs the injected executable exactly as `bun run dev` in the configured frontend directory, uses bounded byte log decoding and drains/reaps readers after readiness, accepts only loopback URLs before the configured timeout, tracks and cancels WebSocket tunnels, and terminates the Unix process group or Windows Job Object on readiness, bind, and shutdown failures. HTTP shutdown signals the listener first, cancels in-flight proxy/tunnel work, terminates the frontend, then awaits the listener; timeout aborts return an error. Disabled mode never binds.

## Final review-fix verification

Against `fc0591f`, the final focused command passed 21 tests, the Windows target check passed with the required compiler environment, CLI help remained valid, and fixture smoke printed exactly one readiness line and exited 0 after SIGTERM.
