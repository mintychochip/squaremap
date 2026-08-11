# Task 6 report: Rust HTTP/output/dev frontend contract

## RED

The required command was run before production modules existed:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test http_contract --test dev_frontend
```

It failed during test compilation because `squaremap_server` had no library crate and therefore `HttpServer`/`OutputRoot` were absent. This was the genuine missing-module RED.

## GREEN

Focused contract tests now pass:

```text
cargo test --manifest-path rust/Cargo.toml -p squaremap-server --test http_contract --test dev_frontend
cargo test: 9 passed (2 suites, 13 warnings, 0.00s)
```

The tests cover static index/JSON/PNG responses, GET/HEAD, explicit MIME and length headers, quoted ETags and 304, tile cache headers and missing-PNG behavior, non-tile 404, method/path rejection, root-confined writes, symlink rejection, disabled mode, dev URL rejection/readiness timeout, HTTP and WebSocket forwarding, and local tile exclusions.

## Smoke

`--help` remained valid and printed both `bridge` and `serve-fixture` usage. The fixture command was launched with `--root web --bind 127.0.0.1:0`; it printed exactly one readiness line:

```text
READY http_addr=127.0.0.1:33597
```

Fetching `/` returned HTTP 200 and 1952 bytes. Fetching `/tiles/missing.png` returned HTTP 200 with `Content-Length: 0`; the saved body was 0 bytes. The process was terminated with SIGTERM and exited 0 after the graceful signal path.

## Security review

- URI percent decoding occurs once; malformed encodings, NUL, backslash, encoded separators, absolute/prefix/current/parent components, and symlink components are rejected.
- HTTP reads and output writes share `validate_relative` and component checks.
- Output writes use a per-root shared mutex, exclusive sibling temporary files, write/flush/sync, atomic rename, parent-directory sync on Unix, and cleanup on errors.
- Opened file handles supply metadata used for strong quoted ETags; matching `If-None-Match` returns 304 before body reading.
- Missing tiles are only synthesized for the exact `tiles` first component and `.png` extension; `/tiles2` is not an exclusion.
- Proxy request/response handling strips hop-by-hop headers and bounds body capture at 16 MiB.

## Lifecycle review

Dev startup runs the injected executable exactly as `bun run dev` in the configured frontend directory, merges bounded stdout/stderr line capture, accepts only loopback URLs before the configured timeout, and terminates the process group on readiness, bind, and shutdown failures. HTTP shutdown is idempotent, stops the listener before the frontend child, and disabled mode never binds.

Atomic task commit: this commit; SHA recorded externally.
