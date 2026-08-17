package xyz.jpenilla.squaremap.common.bridge.process;

import java.nio.file.Files;
import java.nio.file.Path;
import java.security.SecureRandom;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Base64;
import java.util.List;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.PlayersReplace;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent;

import static org.junit.jupiter.api.Assertions.*;

class SidecarSupervisorTest {
    @Test
    void closeAfterFailurePreservesOriginalThrowableAndLateListenerReplaysOnce() throws Exception {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final BridgeConnection connection = supervisor.startDetachedForTests();
        final var failures = new ArrayList<Throwable>();
        final RuntimeException original = new RuntimeException("original");
        connection.setFailureListener(failures::add);
        final var signal = connection.getClass().getDeclaredMethod("signalFailure", Throwable.class);
        signal.setAccessible(true);
        signal.invoke(connection, original);
        connection.close();
        connection.setFailureListener(failures::add);
        assertEquals(1, failures.size());
        assertSame(original, failures.get(0));
        supervisor.close();
    }
    @Test
    void validHandshakeReturnsConnectionAndCloseIsIdempotent() throws Exception {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final BridgeConnection connection = supervisor.start(config("valid", Duration.ofSeconds(5)))
            .toCompletableFuture().get(6, TimeUnit.SECONDS);
        assertFalse(connection.isClosed());
        connection.close();
        assertDoesNotThrow(connection::close);
        assertTrue(connection.isClosed());
        assertDoesNotThrow(supervisor::close);
    }

    @Test
    void publishesFramedReplacementAndConsumesAcknowledgement() throws Exception {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final BridgeConnection connection = supervisor.start(config("ack-publish", Duration.ofSeconds(5)))
            .toCompletableFuture().get(6, TimeUnit.SECONDS);

        final Envelope payload = Envelope.newBuilder()
            .setPlayersReplace(PlayersReplace.newBuilder().setMaxPlayers(20))
            .build();
        final BridgeEvent event = new BridgeEvent.ReplaceState("players", payload);
        assertTrue(connection.publish(event) == xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher.PublishResult.ACCEPTED);
        Thread.sleep(100L);
        assertFalse(connection.isClosed());
        connection.close();
        supervisor.close();
    }
    @Test
    void routesWorldEnumerationRequestsToSnapshotListenerOnly() throws Exception {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final BridgeConnection connection = supervisor.start(config("world-enumeration", Duration.ofSeconds(5)))
            .toCompletableFuture().get(6, TimeUnit.SECONDS);
        final CountDownLatch snapshot = new CountDownLatch(1);
        final CountDownLatch response = new CountDownLatch(1);
        connection.setSnapshotRequestListener(envelope -> {
            if (envelope.hasWorldEnumerationRequest()) snapshot.countDown();
        });
        connection.setResponseListener(envelope -> response.countDown());
        assertTrue(snapshot.await(3, TimeUnit.SECONDS));
        assertFalse(response.await(200, TimeUnit.MILLISECONDS));
        connection.close();
        supervisor.close();
    }
    @Test
    void closeStopsOwnedSidecarThreads() throws Exception {
        final int before = sidecarThreadCount();
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final BridgeConnection connection = supervisor.start(config("valid", Duration.ofSeconds(5)))
            .toCompletableFuture().get(6, TimeUnit.SECONDS);
        assertTrue(sidecarThreadCount() > before);
        connection.close();
        final long deadline = System.nanoTime() + Duration.ofSeconds(2).toNanos();
        while (System.nanoTime() < deadline && sidecarThreadCount() > before) {
            Thread.sleep(10L);
        }
        assertTrue(sidecarThreadCount() <= before);
    }

    private static int sidecarThreadCount() {
        return (int) Thread.getAllStackTraces().keySet().stream()
            .filter(thread -> thread.getName().equals("squaremap-sidecar"))
            .count();
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
    void protocolMinorMismatchRejectsAndTerminatesChild() {
        assertRejected("minor-mismatch");
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
    void sidecarDisconnectSchedulesBoundedRecovery() throws Exception {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final BridgeConnection first = supervisor.start(config("crash-after-handshake", Duration.ofSeconds(5)))
            .toCompletableFuture().get(6, TimeUnit.SECONDS);
        final java.util.concurrent.CountDownLatch reconnected = new java.util.concurrent.CountDownLatch(1);
        supervisor.setReconnectListener(connection -> reconnected.countDown());
        final java.util.concurrent.CountDownLatch failure = new java.util.concurrent.CountDownLatch(1);
        first.setFailureListener(ignored -> failure.countDown());
        assertTrue(failure.await(3, TimeUnit.SECONDS));
        assertTrue(reconnected.await(2, TimeUnit.SECONDS));
        final BridgeConnection replacement = supervisor.currentConnection();
        assertNotNull(replacement);
        assertNotSame(first, replacement);
        assertFalse(supervisor.isClosed());
        supervisor.close();
        assertTrue(supervisor.isClosed());
    }
    @Test
    void reconnectPublishesOneStableBridgeIdentity() throws Exception {
        final Path data = Files.createTempDirectory("bridge-identity");
        final SidecarSupervisor first = new SidecarSupervisor(new SecureRandom(), data);
        try {
            final byte[] expected = Files.readAllBytes(data.resolve("bridge-identity.bin"));
            assertEquals(SidecarSupervisor.SESSION_ID_BYTES, expected.length);
            final SidecarSupervisor second = new SidecarSupervisor(new SecureRandom(), data);
            try {
                assertArrayEquals(expected, Files.readAllBytes(data.resolve("bridge-identity.bin")));
            } finally {
                second.close();
            }
        } finally {
            first.close();
        }
    }
    @Test
    void authenticatedFailureDuringRestartStopsAfterFailedReauthentication() throws Exception {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final CountDownLatch failure = new CountDownLatch(1);
        supervisor.setReconnectListener(connection -> connection.setFailureListener(ignored -> failure.countDown()));
        final Path state = Files.createTempFile("squaremap-fault-sequence", ".txt");
        Files.deleteIfExists(state);
        try {
            final BridgeBootstrapConfig config = config("auth-fails-after-restart", Duration.ofSeconds(5), Duration.ofMillis(200), state);
            supervisor.start(config).toCompletableFuture().get(6, TimeUnit.SECONDS);
            assertTrue(failure.await(3, TimeUnit.SECONDS));
            assertTimeoutPreemptively(Duration.ofSeconds(20), () -> {
                while (Files.readString(state).trim().equals("1")) {
                    Thread.sleep(25L);
                }
                assertEquals("2", Files.readString(state).trim());
                while (!supervisor.isClosed()) {
                    Thread.sleep(25L);
                }
            });
            assertTrue(supervisor.currentConnection() == null);
        } finally {
            supervisor.close();
            Files.deleteIfExists(state);
        }
    }
    @Test
    void duplicateInboundSequenceFailsAuthenticatedConnection() throws Exception {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final BridgeConnection connection = supervisor.start(config("duplicate-inbound-sequence", Duration.ofSeconds(5)))
            .toCompletableFuture().get(6, TimeUnit.SECONDS);
        final CountDownLatch failure = new CountDownLatch(1);
        connection.setFailureListener(ignored -> failure.countDown());
        assertTrue(failure.await(3, TimeUnit.SECONDS));
        assertTimeoutPreemptively(Duration.ofSeconds(3), () -> {
            while (!connection.isClosed()) {
                Thread.sleep(10L);
            }
        });
        supervisor.close();
    }

    @Test
    void gapInboundSequenceFailsAuthenticatedConnection() throws Exception {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final BridgeConnection connection = supervisor.start(config("gap-inbound-sequence", Duration.ofSeconds(5)))
            .toCompletableFuture().get(6, TimeUnit.SECONDS);
        final CountDownLatch failure = new CountDownLatch(1);
        connection.setFailureListener(ignored -> failure.countDown());
        assertTrue(failure.await(3, TimeUnit.SECONDS));
        assertTimeoutPreemptively(Duration.ofSeconds(3), () -> {
            while (!connection.isClosed()) {
                Thread.sleep(10L);
            }
        });
        supervisor.close();
    }


    @Test
    void failedSupervisorIsTerminalBeforeAuthentication() {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        assertThrows(CompletionException.class, () -> supervisor.start(config("timeout", Duration.ofMillis(150)))
            .toCompletableFuture().join());
        assertTrue(supervisor.isClosed());
        supervisor.close();
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
    void configuredShutdownGraceBoundsForcedTermination() throws Exception {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final BridgeConnection connection = supervisor.start(config(
                "ignore-shutdown",
                Duration.ofSeconds(5),
                Duration.ofMillis(100)
            ))
            .toCompletableFuture().get(6, TimeUnit.SECONDS);
        assertTimeout(Duration.ofSeconds(3), connection::close);
        assertTrue(connection.isClosed());
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
    @Test
    void childReceivesConfiguredRustOutputRoot() throws Exception {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        final BridgeConnection connection = supervisor.start(config("root-check", Duration.ofSeconds(5)))
            .toCompletableFuture().get(6, TimeUnit.SECONDS);
        connection.close();
        supervisor.close();
    }
    @Test
    void rustBackendRejectsMissingRustOutputRoot() {
        assertThrows(IllegalArgumentException.class, () -> new BridgeBootstrapConfig(
            "fixture",
            new SidecarCommand(List.of("fixture")),
            Duration.ofSeconds(1),
            Duration.ofSeconds(1)
        ));
    }
    @Test
    void rejectsNestedAndSymlinkAliasedOutputRoots() throws Exception {
        final Path root = java.nio.file.Files.createTempDirectory("squaremap-roots");
        final Path javaRoot = root.resolve("java");
        java.nio.file.Files.createDirectories(javaRoot);
        assertThrows(IllegalArgumentException.class, () -> BridgeBootstrapConfig.validateIsolatedRoots(javaRoot, javaRoot.resolve("rust")));
        assertThrows(IllegalArgumentException.class, () -> BridgeBootstrapConfig.validateIsolatedRoots(javaRoot, root));
        final Path alias = root.resolve("alias");
        java.nio.file.Files.createSymbolicLink(alias, javaRoot);
        assertThrows(IllegalArgumentException.class, () -> BridgeBootstrapConfig.validateIsolatedRoots(javaRoot, alias.resolve("rust")));
    }

    private static void assertRejected(final String behavior) {
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        assertThrows(CompletionException.class, () -> supervisor.start(config(behavior, Duration.ofSeconds(5)))
            .toCompletableFuture().join());
        assertTrue(supervisor.isClosed());
        supervisor.close();
    }

    private static BridgeBootstrapConfig config(final String behavior, final Duration timeout) {
        return config(behavior, timeout, Duration.ofMillis(200), null);
    }

    private static BridgeBootstrapConfig config(
        final String behavior,
        final Duration timeout,
        final Duration shutdownGrace
    ) {
        return config(behavior, timeout, shutdownGrace, null);
    }

    private static BridgeBootstrapConfig config(
        final String behavior,
        final Duration timeout,
        final Duration shutdownGrace,
        final Path stateFile
    ) {
        final List<String> command = new ArrayList<>(List.of(
            javaExecutable().toString(),
            "-cp",
            System.getProperty("java.class.path"),
            FakeSidecar.class.getName(),
            "--behavior=" + behavior,
            "--expected-root=" + rustOutputRoot()
        ));
        if (stateFile != null) command.add("--state-file=" + stateFile);
        return new BridgeBootstrapConfig(
            "fixture",
            new SidecarCommand(command),
            timeout,
            shutdownGrace,
            Path.of(System.getProperty("java.io.tmpdir"), "squaremap-rust-fixture")
        );
    }
    private static String rustOutputRoot() {
        return Path.of(System.getProperty("java.io.tmpdir"), "squaremap-rust-fixture").toAbsolutePath().normalize().toString();
    }

    private static Path javaExecutable() {
        return Path.of(System.getProperty("java.home"), "bin", "java");
    }
}
