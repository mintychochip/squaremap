package xyz.jpenilla.squaremap.common.bridge.process;

import java.nio.file.Path;
import java.time.Duration;
import java.util.Base64;
import java.util.List;
import java.util.concurrent.CompletionException;
import java.util.concurrent.TimeUnit;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertDoesNotThrow;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

class SidecarSupervisorTest {
    @Test
    void validHandshakeReturnsConnectionAndCloseIsIdempotent() throws Exception {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final BridgeConnection connection = supervisor.start(config("valid", Duration.ofSeconds(5)))
            .toCompletableFuture().get(6, TimeUnit.SECONDS);
        assertFalse(connection.isClosed());
        connection.close();
        assertDoesNotThrow(connection::close);
        assertDoesNotThrow(supervisor::close);
    }

    @Test
    void tokenMismatchRejectsAndTerminatesChild() {
        assertRejected("token-mismatch");
    }

    @Test
    void protocolMajorMismatchRejectsAndTerminatesChild() {
        assertRejected("major-mismatch");
    }

    @Test
    void readinessTimeoutCleansUpChildAndListener() {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        assertThrows(CompletionException.class, () -> supervisor.start(config("timeout", Duration.ofMillis(150)))
            .toCompletableFuture().join());
        assertTrue(supervisor.isClosed());
        assertDoesNotThrow(supervisor::close);
    }

    @Test
    void stderrCaptureIsBounded() throws Exception {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final BridgeConnection connection = supervisor.start(config("stderr", Duration.ofSeconds(5)))
            .toCompletableFuture().get(6, TimeUnit.SECONDS);
        assertTrue(supervisor.stderrSnapshot().length <= SidecarSupervisor.MAX_STDERR_BYTES);
        connection.close();
    }

    @Test
    void childArgumentsContainAddressAndVersionButNotA32ByteToken() throws Exception {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final BridgeConnection connection = supervisor.start(config("valid", Duration.ofSeconds(5)))
            .toCompletableFuture().get(6, TimeUnit.SECONDS);
        final List<String> command = supervisor.launchedCommandSnapshot();
        assertTrue(command.contains("bridge"));
        assertTrue(command.contains("--connect"));
        assertTrue(command.contains("--plugin-version"));
        assertTrue(command.contains("fixture"));
        assertFalse(command.stream().anyMatch(SidecarSupervisorTest::is32ByteBase64));
        connection.close();
    }

    private static boolean is32ByteBase64(final String argument) {
        try {
            return Base64.getDecoder().decode(argument).length == 32;
        } catch (final IllegalArgumentException ignored) {
            return false;
        }
    }

    @Test
    void startIsSingleUseAndDoesNotRestartAfterClose() throws Exception {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final BridgeBootstrapConfig config = config("valid", Duration.ofSeconds(5));
        final var first = supervisor.start(config);
        final BridgeConnection connection = first.toCompletableFuture().get(6, TimeUnit.SECONDS);
        connection.close();
        assertTrue(first == supervisor.start(config));
        assertDoesNotThrow(supervisor::close);
    }

    private static void assertRejected(final String behavior) {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        assertThrows(CompletionException.class, () -> supervisor.start(config(behavior, Duration.ofSeconds(5)))
            .toCompletableFuture().join());
        assertTrue(supervisor.isClosed());
        supervisor.close();
    }

    private static BridgeBootstrapConfig config(final String behavior, final Duration timeout) {
        return new BridgeBootstrapConfig(
            BackendMode.RUST,
            "fixture",
            new SidecarCommand(List.of(
                javaExecutable().toString(),
                "-cp",
                System.getProperty("java.class.path"),
                FakeSidecar.class.getName(),
                "--behavior=" + behavior
            )),
            timeout,
            Duration.ofMillis(200)
        );
    }

    private static Path javaExecutable() {
        final String executable = System.getProperty("java.home") + "/bin/java";
        return Path.of(executable);
    }
}
