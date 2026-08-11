# Task 6 report: Rust HTTP/output/dev frontend contract

## RED

The required command was run before production modules existed:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test http_contract --test dev_frontend
```

It failed during test compilation because `squaremap_server` had no library crate and therefore `HttpServer`/`OutputRoot` were absent. This was the genuine missing-module RED.

## GREEN

The focused contract command passes all current HTTP/output and development-frontend tests:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test http_contract --test dev_frontend
cargo test: 24 passed (2 suites, 13 warnings, 0.00s)
```

The tests cover static index/JSON/PNG responses, GET/HEAD, explicit MIME and length headers, quoted/weak/wildcard ETags and 304, tile cache headers and missing-PNG behavior, non-tile 404, method/path rejection, root-confined writes, symlink rejection, root/owner-lock symlink rejection, disabled mode, single-owner locking, dev URL rejection/readiness timeout, delayed readiness and multiple candidates, immediate exit, bounded multibyte/sustained logs after readiness, HTTP POST body/query/status forwarding, retained infinite-response cancellation, barrier-controlled stalled WebSocket-handshake cancellation, WebSocket header/subprotocol/origin forwarding, successful and startup-failure descendant process-group shutdown, concurrent atomic writes, reserved/malformed temp namespace handling, and local tile exclusions.

The Windows-target check passed with the required cross-compilation environment:

```text
CC_x86_64_pc_windows_gnu=gcc AR_x86_64_pc_windows_gnu=ar cargo check --manifest-path rust/Cargo.toml -p squaremap-server --target x86_64-pc-windows-gnu
```

## Smoke

`--help` remained valid and printed both `bridge` and `serve-fixture` usage. The latest fixture smoke launched `serve-fixture --root web --bind 127.0.0.1:0`; it printed exactly one readiness line:

```text
READY http_addr=127.0.0.1:41921
```

Fetching `/` returned HTTP 200 with `Content-Length: 1952`; the process was terminated with SIGTERM and exited 0 after the graceful signal path.

## Security review

- URI percent decoding occurs once; malformed encodings, NUL, backslash, encoded separators, absolute/prefix/current/parent components, and symlink components are rejected.
- HTTP reads and output writes share `validate_relative` and component checks; reserved owner/temp internals are rejected.
- Unix output roots open stable `/` or `.` anchors and traverse/create each component with `openat`/`mkdirat` and `O_NOFOLLOW`; Windows anchors at the volume/current-directory capability and opens each component with `OPEN_REPARSE_POINT`, validating the opened handle before advancing. HTTP reads and output writes then traverse capability-relative handles; temporary files are exclusive and cleaned on errors.
- Output roots acquire a cross-platform exclusive owner lock before cleanup; Windows validates the opened lock handle as a non-reparse regular file, so stale cleanup cannot remove another live writer's temporary file.
- Opened file handles supply metadata used for strong quoted ETags; matching `If-None-Match` returns 304 before body reading.
- Missing tiles are only synthesized for the exact `tiles` first component and `.png` extension; `/tiles2` is not an exclusion.
- Proxy bodies use bounded streaming adapters rather than whole-body copies; request/response hop-by-hop filtering parses `Connection` extension tokens and suppresses Host forwarding.


## Lifecycle review

Dev startup runs the injected executable exactly as `bun run dev` in the configured frontend directory, uses bounded byte log decoding and drains/reaps readers after readiness, accepts only loopback URLs before the configured timeout, tracks and cancels WebSocket tunnels, and terminates the Unix process group or Windows Job Object on readiness, bind, and shutdown failures. HTTP shutdown signals the listener first, cancels in-flight proxy/tunnel work, terminates the frontend, then awaits the listener; timeout aborts return an error. Disabled mode never binds.

## Final review-fix verification

The final focused command passed 24 tests, including live-owner reserved-temp rejection and missing-component symlink rejection; the Windows target check and Windows-gated test compilation passed with the required compiler environment; CLI help remained valid; and fixture smoke printed exactly one readiness line and exited 0 after SIGTERM. The final changes also create the Windows Job Object before spawning the suspended process, assign the Tokio Child raw process handle before resume, validate opened root and owner-lock attributes, and eliminate ambient root creation.
