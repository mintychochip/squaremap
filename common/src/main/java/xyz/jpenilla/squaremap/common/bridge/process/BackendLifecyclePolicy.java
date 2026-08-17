package xyz.jpenilla.squaremap.common.bridge.process;

/** Ownership decisions after Java map-backend removal. Rust always owns HTTP and render. */
public final class BackendLifecyclePolicy {
    private BackendLifecyclePolicy() {
    }

    public static BackendMode configuredMode() {
        return BackendMode.RUST;
    }

    public static boolean rustHttpOwner(final BackendMode mode) {
        return mode == BackendMode.RUST;
    }
}
