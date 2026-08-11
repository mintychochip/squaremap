package xyz.jpenilla.squaremap.common.bridge.process;

import com.google.inject.Inject;
import java.time.Duration;
import java.util.Objects;

/** Immutable, validated sidecar bootstrap settings. */
public final class BridgeBootstrapConfig {
    public static final Duration DEFAULT_READINESS_TIMEOUT = Duration.ofSeconds(30);
    public static final Duration DEFAULT_SHUTDOWN_GRACE = Duration.ofSeconds(10);

    private final BackendMode backendMode;
    private final String pluginVersion;
    private final SidecarCommand sidecarCommand;
    private final Duration readinessTimeout;
    private final Duration shutdownGrace;
    @Inject
    public BridgeBootstrapConfig() {
        this(BackendMode.JAVA, "unknown", null);
    }


    public BridgeBootstrapConfig(
        final BackendMode backendMode,
        final String pluginVersion,
        final SidecarCommand sidecarCommand
    ) {
        this(backendMode, pluginVersion, sidecarCommand, DEFAULT_READINESS_TIMEOUT, DEFAULT_SHUTDOWN_GRACE);
    }

    public BridgeBootstrapConfig(
        final BackendMode backendMode,
        final String pluginVersion,
        final SidecarCommand sidecarCommand,
        final Duration readinessTimeout,
        final Duration shutdownGrace
    ) {
        this.backendMode = Objects.requireNonNull(backendMode, "backendMode");
        if (pluginVersion == null || pluginVersion.isBlank()) {
            throw new IllegalArgumentException("plugin version must not be blank");
        }
        this.pluginVersion = pluginVersion;
        if (backendMode != BackendMode.JAVA && sidecarCommand == null) {
            throw new IllegalArgumentException("non-Java backend requires a sidecar command");
        }
        this.sidecarCommand = sidecarCommand;
        this.readinessTimeout = positive(readinessTimeout, "readinessTimeout");
        this.shutdownGrace = positive(shutdownGrace, "shutdownGrace");
    }

    public static BridgeBootstrapConfig java(final String pluginVersion) {
        return new BridgeBootstrapConfig(BackendMode.JAVA, pluginVersion, null);
    }

    public BackendMode backendMode() {
        return this.backendMode;
    }

    public String pluginVersion() {
        return this.pluginVersion;
    }

    public SidecarCommand sidecarCommand() {
        return this.sidecarCommand;
    }

    public Duration readinessTimeout() {
        return this.readinessTimeout;
    }

    public Duration shutdownGrace() {
        return this.shutdownGrace;
    }

    private static Duration positive(final Duration duration, final String name) {
        Objects.requireNonNull(duration, name);
        if (duration.isZero() || duration.isNegative()) {
            throw new IllegalArgumentException(name + " must be positive");
        }
        return duration;
    }
}
