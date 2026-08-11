package xyz.jpenilla.squaremap.common.bridge.process;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.Objects;
import xyz.jpenilla.squaremap.common.config.Config;
/** Immutable, validated sidecar bootstrap settings. */
public final class BridgeBootstrapConfig {
    public static final Duration DEFAULT_READINESS_TIMEOUT = Duration.ofSeconds(30);
    public static final Duration DEFAULT_SHUTDOWN_GRACE = Duration.ofSeconds(10);

    private final BackendMode backendMode;
    private final String pluginVersion;
    private final SidecarCommand sidecarCommand;
    private final Duration readinessTimeout;
    private final Duration shutdownGrace;
    private final Path rustOutputRoot;

    public BridgeBootstrapConfig(
        final BackendMode backendMode,
        final String pluginVersion,
        final SidecarCommand sidecarCommand
    ) {
        this(backendMode, pluginVersion, sidecarCommand, DEFAULT_READINESS_TIMEOUT, DEFAULT_SHUTDOWN_GRACE, null);
    }

    public BridgeBootstrapConfig(
        final BackendMode backendMode,
        final String pluginVersion,
        final SidecarCommand sidecarCommand,
        final Duration readinessTimeout,
        final Duration shutdownGrace
    ) {
        this(backendMode, pluginVersion, sidecarCommand, readinessTimeout, shutdownGrace, null);
    }

    public BridgeBootstrapConfig(
        final BackendMode backendMode,
        final String pluginVersion,
        final SidecarCommand sidecarCommand,
        final Duration readinessTimeout,
        final Duration shutdownGrace,
        final Path rustOutputRoot
    ) {
        this.backendMode = Objects.requireNonNull(backendMode, "backendMode");
        if (pluginVersion == null || pluginVersion.isBlank()) {
            throw new IllegalArgumentException("plugin version must not be blank");
        }
        this.pluginVersion = pluginVersion;
        if (backendMode != BackendMode.JAVA && sidecarCommand == null) {
            throw new IllegalArgumentException("non-Java backend requires a sidecar command");
        }
        if (backendMode != BackendMode.JAVA && rustOutputRoot == null) {
            throw new IllegalArgumentException("non-Java backend requires a Rust output root");
        }
        if (rustOutputRoot != null && !rustOutputRoot.isAbsolute()) {
            throw new IllegalArgumentException("Rust output root must be absolute");
        }
        this.sidecarCommand = sidecarCommand;

        this.readinessTimeout = positive(readinessTimeout, "readinessTimeout");
        this.shutdownGrace = positive(shutdownGrace, "shutdownGrace");
        this.rustOutputRoot = rustOutputRoot == null ? null : rustOutputRoot.normalize();
    }
    public static void validateIsolatedRoots(final Path javaRoot, final Path rustRoot) {
        final Path first = canonical(javaRoot);
        final Path second = canonical(rustRoot);
        if (first.startsWith(second) || second.startsWith(first)) {
            throw new IllegalArgumentException("Java and Rust output roots overlap");
        }
    }

    private static Path canonical(final Path path) {
        Path current = Objects.requireNonNull(path, "path").toAbsolutePath().normalize();
        final List<Path> suffix = new ArrayList<>();
        while (!Files.exists(current)) {
            suffix.add(0, current.getFileName());
            current = current.getParent();
            if (current == null) throw new IllegalArgumentException("output root has no existing ancestor");
        }
        try {
            Path result = current.toRealPath();
            for (final Path part : suffix) result = result.resolve(part);
            return result.normalize();
        } catch (final IOException error) {
            throw new IllegalArgumentException("could not canonicalize output root", error);
        }
    }
    public static BridgeBootstrapConfig configured() {
        final BackendMode mode = BackendMode.valueOf(Config.BRIDGE_BACKEND_MODE.toUpperCase(java.util.Locale.ROOT));
        if (mode == BackendMode.JAVA) return java("unknown");
        if (Config.BRIDGE_SIDECAR_COMMAND.isEmpty() || Config.BRIDGE_RUST_OUTPUT_ROOT.isBlank()) {
            throw new IllegalStateException("non-Java backend requires settings.bridge.sidecar-command and settings.bridge.rust-output-root");
        }
        return new BridgeBootstrapConfig(mode, "unknown", new SidecarCommand(Config.BRIDGE_SIDECAR_COMMAND),
            Duration.ofSeconds(Config.BRIDGE_STARTUP_TIMEOUT_SECONDS), DEFAULT_SHUTDOWN_GRACE, Path.of(Config.BRIDGE_RUST_OUTPUT_ROOT));
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

    public Path rustOutputRoot() {
        return this.rustOutputRoot;
    }

    private static Duration positive(final Duration duration, final String name) {
        Objects.requireNonNull(duration, name);
        if (duration.isZero() || duration.isNegative()) {
            throw new IllegalArgumentException(name + " must be positive");
        }
        return duration;
    }
}
