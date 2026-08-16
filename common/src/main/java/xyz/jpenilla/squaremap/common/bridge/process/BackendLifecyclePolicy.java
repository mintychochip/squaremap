package xyz.jpenilla.squaremap.common.bridge.process;

import java.util.Locale;
import xyz.jpenilla.squaremap.common.config.Config;

/** Centralizes mode ownership decisions used by the common lifecycle. */
public final class BackendLifecyclePolicy {
    private BackendLifecyclePolicy() {
    }

    public static BackendMode configuredMode() {
        return BackendMode.valueOf(Config.BRIDGE_BACKEND_MODE.toUpperCase(Locale.ROOT));
    }

    public static boolean javaHttpOwner(final BackendMode mode) {
        return mode != BackendMode.RUST;
    }

    public static boolean javaCacheOwner(final BackendMode mode) {
        return mode != BackendMode.RUST;
    }

    public static boolean javaDirtyOwner(final BackendMode mode) {
        return mode != BackendMode.RUST;
    }

    public static boolean javaRenderOwner(final BackendMode mode) {
        return mode != BackendMode.RUST;
    }

    public static boolean rustHttpOwner(final BackendMode mode) {
        return mode == BackendMode.RUST;
    }
}
