# Squaremap Rust Backend — Parity Review Findings (workflowz audit)

Date: 2026-08-18. Fork point: upstream `ad21b0c` (jpenilla/squaremap, zero downstream drift).
Method: 8 parallel dimension scouts → adversarial source verification of 7 HIGH/MED claims → fixes developed and unit-tested.

## Fixed and unit-tested (Rust)

1. **Web frontend serving in plugin mode** (`rust/crates/squaremap-server/src/http/{mod,static_files,output}.rs`, `tests/http_contract.rs`; `common/.../bridge/process/BridgeBootstrapConfig.java`, `SquaremapCommon.java`, `SidecarSupervisor.java`)
   - Divergence: the user-facing Rust HTTP server served only the output root; the built web frontend was extracted to a disjoint `webDirectory()` and never delivered (index.html/JS/CSS/images absent → the UI couldn't load).
   - Fix: `HttpConfig.web_root` + `serve_web` serve non-tile/non-registered-icon paths from a read-only web root, confined with the same O_NOFOLLOW component walking as the output root on unix (non-unix uses canonical-path containment, which is a different mechanism and is not claimed equivalent). Verified end-to-end with the real release binary (`cargo build --release -p squaremap-server`, `serve-fixture` with `SQUAREMAP_WEB_ROOT=common/build/web`): `/` → 200 text/html from web root; `/favicon.ico` → 200 image/x-icon; `/assets/index-DnYrALrj.js` (real built asset) → 200 application/javascript; `/tiles/settings.json` → 200 application/json from the output root; `/nope.js` → 404. Verified against the actual built frontend (`common/build/web`): `index.html`, `assets/index-*.js` (application/javascript), `assets/index-*.css` (text/css), `assets/index-*.js.map` (application/json), `favicon.ico` (image/x-icon), and `images/*.png` are all served with correct types; `tiles/*` and `images/icon/registered/*` still route to the output root. Every static path the frontend requests is covered: `images/icon/{world.icon}.png` + `*-cube-smol.png` + `player.png` → web root; `images/icon/registered/{icon}.png` (markers) → output root; `images/health|armor/*.png`, `images/clear.png`, sky backgrounds → web root. Java passes the extracted web directory via `squaremap.backendWebRoot` (a global system property set in `SquaremapCommon.start()`; it is overwritten on each start but not explicitly cleared on shutdown, so it remains set process-wide — a known lifecycle limitation, not a correctness issue for a single running plugin). → `SQUAREMAP_WEB_ROOT`. Tests: `serves_frontend_from_web_root_and_tiles_from_output_root` (index/JS/ico from web root; tiles/registered icons from output root; missing 404), `web_root_rejects_symlink_escape` (confinement), and content-type cases using the real built asset filenames; 15/15 http_contract tests pass. Java `:squaremap-common:compileJava` passes. Live Paper/plugin end-to-end serving remains unproven (no plugin runtime here), but the source gap is closed.

2. **MultiPolygon marker wire tag** (`rust/crates/squaremap-state/src/view.rs`, `tests/view_contract.rs`)
   - Divergence: `MarkerGeometryView::MultiPolygon` serialized `"type":"multipolygon"`, but the unchanged web frontend (`web/src/js/util/World.js`) only dispatches on `rectangle|polyline|polygon|circle|ellipse|icon`. MultiPolygon addon markers were silently dropped.
   - Fix: manual Serialize/Deserialize emits `"type":"polygon"` (matching upstream and the fork's own Java reference serializer) and infers multi-polygon from point nesting depth on round-trip. Test `multipolygon_geometry_uses_polygon_type_tag` updated; 10/10 view_contract tests pass.

3. **Players in disabled worlds rejected whole update** (`rust/crates/squaremap-server/src/views.rs`, `tests/view_contract.rs`)
   - Divergence: `PlayerStateExporter` emits players from ALL loaded levels (including `MAP_ENABLED=false`), but `validate_player_epoch` required each player's world to exist in canonical `world_epochs` (enabled worlds only). A player in a disabled world failed the entire players.json replacement.
   - Fix: `validate_player_epoch` now rejects only **stale** epochs for worlds that ARE canonical; a player whose world is not canonical is published (matching upstream, which includes players from all levels). Added test `players_from_disabled_worlds_are_published_without_canonical_epoch`; 10/10 view_contract tests pass.

4. **Replay-pending dirty rows never re-rendered after reconnect** (`rust/crates/squaremap-state/src/repository.rs`, `tests/owner_lease.rs`)
   - Divergence: on sidecar reconnect, `reassign_bridge_lease` sets every owned dirty row `replay_pending=1, lease_expires=0`, and `dirty_page_for_owner` filtered `replay_pending=0` only — so chunks dirty at sidecar-crash time were excluded from the only production render tick and never re-rendered (data-loss window). Java's `onDirtyReplayItem` validated then dropped the chunk.
   - Fix: `dirty_page_for_owner` now includes `replay_pending=1 AND lease_expires_epoch_seconds=0` rows (the exact post-reconnect state), so replayed rows are re-rendered by the owner page and completed via `complete_dirty_for_owner`. Test `reassignment_on_reconnect_moves_rows_to_new_session_and_marks_replay` updated to assert the re-render; 9/9 owner_lease tests pass.

5. **HTTP content-type map narrower than upstream** (`rust/crates/squaremap-server/src/http/static_files.rs`, `tests/http_contract.rs`)
   - Divergence: the fork mapped only json/png/html/css/js and returned `application/octet-stream` for everything else; upstream's Undertow MimeMappings serve `.ico` (image/x-icon), `.svg`, fonts, etc. with correct types. The web frontend ships a `favicon.ico` served wrong.
   - Fix: expanded the map to ico/svg/webp/gif/jpeg/woff/woff2/ttf/otf/txt/xml/map/webmanifest plus htm/mjs aliases. Added `serves_web_asset_content_types_and_missing_tile_headers`; 12/12 http_contract tests pass.

6. **Missing-tile 200 carried an explicit Content-Type** (`rust/crates/squaremap-server/src/http/static_files.rs`, `tests/http_contract.rs`)
   - Divergence: for a missing `tiles/*.png` the fork returned 200 with `Content-Type: image/png` + `Content-Length: 0`; upstream's welcome handler sets only status 200 with an empty body.
   - Fix: the fork no longer sets an explicit Content-Type for the missing-tile 200. Content-Length is framework-managed; only hyper was observed emitting `0` for the empty body (Undertow's wire behavior was not captured), and the parity contract is an empty 200 that Leaflet skips rather than a specific header set. Covered by the same new test.

7. **MAP_BIOMES_BLEND out-of-range rejected instead of clamped** (`rust/crates/squaremap-server/src/config.rs`, `tests/config_compatibility.rs`)
   - Divergence: upstream loads `MAP_BIOMES_BLEND` via `Mth.clamp(value, 0, 15)`; the fork rejected `> 15` as an invalid config, failing configs upstream tolerates.
   - Fix: `validate_and_stage` now clamps `map_biomes_blend` to `[0,15]` for the default world and every world, exactly mirroring `Mth.clamp`; the `> 15` rejection was removed. Added `biome_blend_out_of_range_is_clamped_like_upstream`; 3/3 config_compatibility tests pass.

8. **HTTP bind accepted only IP literals, not hostnames** (`rust/crates/squaremap-server/src/bootstrap.rs`, `tests/http_contract.rs`)
   - Divergence: upstream `Undertow.addHttpListener(host, port)` accepts hostnames ("localhost") as well as IP literals; the fork's `format!("{}:{}", bind, port).parse::<SocketAddr>()` rejected hostnames and refused to start HTTP for such configs.
   - Fix: the bootstrap now resolves the configured bind via `tokio::net::lookup_host` before binding, accepting both hostnames and IP literals. Added `hostname_bind_resolves_like_upstream_add_http_listener`; 13/13 http_contract tests pass.

9. **Render progress logging absent** (`rust/crates/squaremap-server/src/scheduler/{mod,jobs}.rs`, `src/bootstrap.rs`, `tests` unit module; `ConfigBridgeExporter.java` already exported the fields)
   - Divergence: upstream schedules a `RenderProgress` TimerTask that logs % complete, chunk/region counts, elapsed, ETA, and rate per second during active renders (gated by `PROGRESS_LOGGING` + `PROGRESS_LOGGING_INTERVAL`, defaults true/1s). The fork validated `progress_logging_interval_seconds` but never consumed it — no progress was ever logged, and `/squaremap progresslogging` had no observable effect.
   - Fix: `SchedulerConfig` now carries `progress_logging_enabled`/`progress_logging_interval_seconds` (threaded from the proto `RenderSettings` that `ConfigBridgeExporter` already exports: Java `Config.PROGRESS_LOGGING` default true, `PROGRESS_LOGGING_INTERVAL` default 1); `jobs::run` logs render progress at the configured interval with upstream's exact message format `(<percent>) World: <world> Chunks: <current>/<total> Elapsed: <HH:MM:SS> ETA: <HH:MM:SS> Rate: <N.N> cps` (matching `LOG_RENDER_PROGRESS`), and skips logging while the world's renders are paused (matching upstream's `RenderProgress.run()` early return). Known minor difference: elapsed/ETA reset on a resumed job run, whereas upstream preserves them across `restartRenderProgressLogging` config changes (resume-after-crash restarts the timer in both). Tests: `hhmmss_formats_duration`, `progress_logger_gates_on_interval_and_disabled`, `interval_zero_is_clamped_to_one`; 8/8 scheduler unit tests pass. Full workspace 42 suites.

Full Rust workspace: 42 suites, 0 failures. `git diff --check` clean.

## Real-binary HTTP smoke (2026-08-19)

`cargo build --release -p squaremap-server`; `serve-fixture` with `SQUAREMAP_WEB_ROOT=/.../common/build/web` and an output root containing `tiles/settings.json` + `images/icon/registered/spawn.png`. Verified over real HTTP against the release binary:

| Request | Result | Root |
|---|---|---|
| `/` | 200 `text/html; charset=utf-8` | web root |
| `/favicon.ico` | 200 `image/x-icon` | web root |
| `/assets/index-C42OGjer.css` | 200 `text/css; charset=utf-8` | web root |
| `/assets/index-DnYrALrj.js` | 200 `application/javascript` | web root |
| `/assets/index-DnYrALrj.js.map` | 200 `application/json` | web root |
| `/images/icon/player.png` | 200 `image/png` | web root |
| `/images/icon/registered/spawn.png` | 200 `image/png` | output root |
| `/tiles/settings.json` | 200 `application/json` + `cache-control: max-age=0,...` | output root |
| `/tiles/missing.png` | 200, empty body, **no content-type**, `content-length: 0` | — |
| `/nope.js` | 404 | — |
| asset + `If-None-Match: <etag>` | 304 | — |

This binary-level evidence confirms fixes 1, 5, and 6 end-to-end beyond unit tests; the hostname-bind fix (8) is unit-verified via `localhost` lookup.

## Remaining unresolved parity gaps (source-confirmed)

- **Live Paper/plugin serving of the web frontend unproven end-to-end.** The Rust web-root source/env wiring is implemented and unit-tested (15/15 http_contract); actual browser delivery through a running Paper plugin with the extracted web bundle is not verified here (no plugin runtime).
- **HIGH — Grass/tinted biome chunks fail to render.** The live scheduler always builds `SnapshotBundle` with `grass_resolutions: Vec::new()`; `SnapshotBiomeSource.resolved_grass` then returns `UnsupportedSelector` for category-1 (grass) blocks, and with default `MAP_BIOMES=true, MAP_BIOMES_BLEND=3` such chunks error out (map holes). Mechanism exists only in fixtures. Fix requires wiring per-position grass resolution from Java through the bridge protocol. Unresolved.
- **HIGH — Non-self-contained packaging / dead download channels.** The jar embeds only a manifest; the backend binary must be obtained separately. `common/build.gradle.kts` bakes `https://github.com/jpenilla/squaremap/releases/download/v${version}/` (upstream repo, `-SNAPSHOT` tag that never resolves); `scripts/sidecar-binary-lib.sh` hardcodes `jpenilla/squaremap`. `BinaryResolver` auto-download, `install.sh`/`update.sh` resolve to dead URLs. Partial-fix scope crosses build + scripts + runtime; unresolved.
- **MED — Java build requires all 5 backend targets staged.** `:squaremap-common:test`/`build` fail unless all 5 native binaries exist locally (only x86_64 present). Blocks running the Java test suite in this environment. Release/packaging gate, unresolved here.
- **MED — Vite dev-frontend proxy unreachable.** `dev_frontend.rs` exists and is tested, but both production `HttpConfig` constructions pass `dev_frontend: None`; `squaremap.devFrontend`/`frontendPath` properties are never read. Developer-facing only.

## Lower-severity confirmations (LOW, mostly header/internal differences)HTTP: players/markers.json Cache-Control + disk freshness differ (Rust always writes JSON to disk and serves with `max-age=0`; upstream serves players/markers from an in-memory JsonCache with no Cache-Control — the fork's stance is a defensible correctness improvement for browsers); no h2c; encoded-separator 400 (stricter traversal hardening than upstream's decode-and-serve); non-GET/HEAD 405 on cached JSON (upstream's JsonCache serves any method). Config: additive bridge.* options only; remaining zoom/interval/thread validations are stricter-than-upstream guards justified by frontend behavior — the web frontend derives its projection scale from `zoom.max` (`S.setScale(max)`, `maxNativeZoom: max`, `setMaxZoom(max+extra)`) and centers via `zoom.def`, so `zoom_default > zoom_max` or bad intervals produce a silently broken/blank map upstream while the fork rejects with a clear invalid-config error (both degrade; the fork fails loudly); PauseRender/ProgressLogging command messaging couples to sidecar. State: players/markers array ordering sorts by uuid/key (cosmetic and verified order-independent — the frontend keys players by uuid into a Map, re-sorts the sidebar by display name via `localeCompare`, sorts layer controls by `order`, sorts worlds by `order`, and iterates markers per-layer, so array order has no effect). Lifecycle: `onEnable` blocks until sidecar handshake; IconRegistry unregister deletes stale PNG (upstream retained until restart); cosmetic task-null cleanup. Rendering: biome down-fall fix preserved by delegation (Java derives colors, Rust stores result); quart-grid biome selection is internal reimplementation, fixture-verified.

## Parity matrix status update
Deterministic rows (transport, config, state, render, json, http) remain `verified_component`. Fixed rows now additionally covered by new/adjusted unit tests. The three `blocked_infrastructure` live rows and the `unproven` rows (addon-api, loader, packaging, rollback, cutover) are unchanged: they still require real Paper/plugin/sidecar artifacts, a live runtime, remote release publication, and canary/observation evidence — none obtainable in this environment.