package xyz.jpenilla.squaremap.common.bridge.process;

/** Centralizes mode ownership decisions used by the common lifecycle. */
public final class BackendLifecyclePolicy {
    private BackendLifecyclePolicy() {
    }

    public static boolean javaHttpOwner(final BackendMode mode) {
        return mode != BackendMode.RUST;
    }

    public static boolean javaCacheOwner(final BackendMode mode) {
        return mode != BackendMode.RUST;
    }
}
