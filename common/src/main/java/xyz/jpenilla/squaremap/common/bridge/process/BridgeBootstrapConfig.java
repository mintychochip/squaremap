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
    static String targetTriple(final String operatingSystem, final String architecture) {
        final String os = Objects.requireNonNull(operatingSystem, "operatingSystem").toLowerCase(java.util.Locale.ROOT);
        final String arch = Objects.requireNonNull(architecture, "architecture").toLowerCase(java.util.Locale.ROOT);
        if (os.contains("windows")) {
            if (arch.equals("amd64") || arch.equals("x86_64")) return "x86_64-pc-windows-msvc";
            throw new IllegalArgumentException("unsupported Windows architecture: " + architecture);
        }
        if (os.contains("mac") || os.contains("darwin")) {
            if (arch.equals("aarch64") || arch.equals("arm64")) return "aarch64-apple-darwin";
            if (arch.equals("amd64") || arch.equals("x86_64")) return "x86_64-apple-darwin";
            throw new IllegalArgumentException("unsupported macOS architecture: " + architecture);
        }
        if (os.contains("linux")) {
            if (arch.equals("aarch64") || arch.equals("arm64")) return "aarch64-unknown-linux-gnu";
            if (arch.equals("amd64") || arch.equals("x86_64")) return "x86_64-unknown-linux-gnu";
            throw new IllegalArgumentException("unsupported Linux architecture: " + architecture);
        }
        throw new IllegalArgumentException("unsupported operating system: " + operatingSystem);
    }
    public static BridgeBootstrapConfig configured(final String pluginVersion) {
        final BackendMode mode = BackendMode.valueOf(Config.BRIDGE_BACKEND_MODE.toUpperCase(java.util.Locale.ROOT));
        if (mode == BackendMode.JAVA) return java(pluginVersion);
        if (!Config.BRIDGE_SIDECAR_COMMAND.isEmpty() && !Config.BRIDGE_RUST_OUTPUT_ROOT.isBlank()) {
            return new BridgeBootstrapConfig(mode, pluginVersion, new SidecarCommand(Config.BRIDGE_SIDECAR_COMMAND),
                Duration.ofSeconds(Config.BRIDGE_STARTUP_TIMEOUT_SECONDS), DEFAULT_SHUTDOWN_GRACE, Path.of(Config.BRIDGE_RUST_OUTPUT_ROOT));
        }
        final Path configuredBinary = Path.of(System.getProperty("squaremap.backendBinary", ""));
        if (!Files.isRegularFile(configuredBinary)) {
            throw new IllegalStateException("non-Java backend requires settings.bridge.sidecar-command and settings.bridge.rust-output-root, or squaremap.backendBinary");
        }
        try {
            final BackendManifest manifest = BackendManifest.load().requireVersion(pluginVersion);
            final String triple = targetTriple(
                System.getProperty("os.name", ""),
                System.getProperty("os.arch", "")
            );
            final BackendManifest.BackendBinary expected = manifest.forTarget(triple);
            if (expected == null) throw new IllegalStateException("native backend manifest has no target " + triple);
            final Path verified = BinaryResolver.verifyConfiguredPath(configuredBinary, expected);
            return new BridgeBootstrapConfig(mode, pluginVersion, new SidecarCommand(verified),
                Duration.ofSeconds(Config.BRIDGE_STARTUP_TIMEOUT_SECONDS), DEFAULT_SHUTDOWN_GRACE,
                Path.of(System.getProperty("squaremap.backendOutputRoot", Path.of("rust-backend").toAbsolutePath().toString())));
        } catch (final IOException error) {
            throw new IllegalStateException("could not resolve configured Rust backend", error);
        }
    }

    public static BridgeBootstrapConfig configured() {
        return configured("unknown");
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
