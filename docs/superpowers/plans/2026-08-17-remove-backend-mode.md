# Remove BackendMode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the `BackendMode` enum and all mode-selection plumbing, hard-coding the Rust backend as the sole backend.

**Architecture:** `BackendMode` currently contains only `RUST` and `BackendLifecyclePolicy` is a trivial `mode == RUST` check. Removing them lets us drop the `settings.bridge.backend-mode` config key, simplify constructors, and delete always-true validation and tests.

**Tech Stack:** Java 25, Gradle Kotlin DSL, Guice, JUnit.

## Global Constraints

- The backend is always Rust; no user-selectable mode remains.
- Preserve existing Rust backend behavior: HTTP enabled flag, sidecar bootstrap, state publishing.
- Delete or update tests that assert mode selection or non-Rust rejection.
- Delete `BackendMode.java` and `BackendLifecyclePolicy.java` entirely.
- Commit each coherent unit and run `:squaremap-common:test` at the end.

---

### Task 1: Delete BackendMode enum and mode plumbing

**Files:**
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BackendMode.java`
- Delete: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BackendLifecyclePolicy.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/config/Config.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeBootstrapConfig.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/config/ConfigBridgeExporter.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/BackendController.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/BridgeBackendController.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/bridge/state/BridgeStatePublisher.java`
- Modify: `common/src/main/java/xyz/jpenilla/squaremap/common/backend/BackendPaths.java`
- Delete: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/process/BackendModeLifecycleTest.java`
- Delete: `common/src/test/java/xyz/jpenilla/squaremap/common/httpd/HttpOwnershipContractTest.java`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/backend/ExclusiveRustOwnershipTest.java`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/process/BridgeBootstrapConfigTargetTest.java`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/process/ProductionBridgeWiringTest.java`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/process/RustSidecarSmokeTest.java`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/process/SidecarSupervisorTest.java`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/backend/BackendControllerTest.java`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/bridge/state/BridgeStatePublisherTest.java`
- Modify: `common/src/test/java/xyz/jpenilla/squaremap/common/inject/module/PlatformModuleCycleTest.java`

**Interfaces:**
- `BridgeBootstrapConfig` no longer exposes `backendMode()` or accept a `BackendMode` constructor parameter.
- `ConfigBridgeExporter.export()` becomes no-arg.
- `BridgeBackendController` and `BridgeStatePublisher` constructors no longer take `BackendMode`.
- `BackendPaths.resolve()` no longer takes `BackendMode`.

**Steps:**
- [ ] **Step 1: Delete `BackendMode.java` and `BackendLifecyclePolicy.java`.** Remove both source files; nothing in production should reference them after the rest of the steps.
- [ ] **Step 2: Remove `BRIDGE_BACKEND_MODE` from `Config.java`.** Delete the constant and the `settings.bridge.backend-mode` read in `bridgeSettings()`.
- [ ] **Step 3: Simplify `BridgeBootstrapConfig.java`.** Drop the `backendMode` field, all constructor params, the `!= RUST` validation, the `BackendMode.parse(...)` call, the `backendMode()` accessor, and the `BackendMode` import.
- [ ] **Step 4: Simplify `ConfigBridgeExporter.java`.** Make `export()` no-arg (delete the throwing no-arg overload), replace `BackendLifecyclePolicy.rustHttpOwner(mode) && Config.HTTPD_ENABLED` with `Config.HTTPD_ENABLED`, and remove `BackendMode` / `BackendLifecyclePolicy` imports.
- [ ] **Step 5: Update `BackendController.java`.** Call `this.configExporter.export()` instead of `this.configExporter.export(BackendMode.RUST)`.
- [ ] **Step 6: Update `BridgeBackendController.java`.** Remove the `mode` field and all `BackendMode.RUST` constructor arguments; drop the private ctor `mode` parameter and `this.mode` assignment.
- [ ] **Step 7: Update `BridgeStatePublisher.java`.** Remove the `mode` field, the `mode` constructor parameter, and all `BackendMode.RUST` arguments (including in `@Inject` and `forTesting`); remove the `BackendMode` import.
- [ ] **Step 8: Update `BackendPaths.java`.** Remove the `mode` parameter from `resolve()`, the `mode != RUST` guard, the `BackendMode` import, and any mode-related javadoc.
- [ ] **Step 9: Update tests.** Delete `BackendModeLifecycleTest` and `HttpOwnershipContractTest` entirely. In the other listed test files, remove `BackendMode` imports, `BackendMode.RUST` constructor arguments, `Config.BRIDGE_BACKEND_MODE` mutations, and assertions that non-Rust modes are rejected (now irrelevant). See the detailed scout report `local://remove-backend-mode-scout.json` for exact line numbers and snippets.
- [ ] **Step 10: Verify.** Run `./gradlew :squaremap-common:test --no-daemon` and fix any compilation or test failures until it is `BUILD SUCCESSFUL`.

**Run:** `./gradlew :squaremap-common:test --no-daemon`
**Expected:** `BUILD SUCCESSFUL`; no `BackendMode` references remain in production or test source (allow only documentation/historical references in `docs/superpowers`).

---

## Risk notes

- `BridgeBackendController.mode` and `BridgeStatePublisher.mode` are assigned but never read in production; removal is behavior-neutral.
- Removing `Config.BRIDGE_BACKEND_MODE` / `settings.bridge.backend-mode` changes the config surface. Existing configs with a `backend-mode: RUST` key will silently ignore it (benign).
- Two tests currently assert that `JAVA`/`SHADOW` config values are rejected. Removing them is correct because the mode concept no longer exists; if a guard against stale `backend-mode` keys is desired, that should be a separate, smaller task.
- `BackendPaths` is independently slated for deletion in the migration docs; strip only its `BackendMode` coupling here, not the whole class.
