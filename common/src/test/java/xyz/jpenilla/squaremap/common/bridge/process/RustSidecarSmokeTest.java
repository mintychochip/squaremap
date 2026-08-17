package xyz.jpenilla.squaremap.common.bridge.process;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.google.protobuf.ByteString;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import org.junit.jupiter.api.Assumptions;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.bridge.v1.AckStatus;
import xyz.jpenilla.squaremap.bridge.v1.AdvancedSettings;
import xyz.jpenilla.squaremap.bridge.v1.ConfigReplace;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.GlobalSettings;
import xyz.jpenilla.squaremap.bridge.v1.LocaleSettings;
import xyz.jpenilla.squaremap.bridge.v1.RenderSettings;
import xyz.jpenilla.squaremap.bridge.v1.UiSettings;
import xyz.jpenilla.squaremap.bridge.v1.WorldConfig;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;
import xyz.jpenilla.squaremap.bridge.v1.WorldSettings;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent;

final class RustSidecarSmokeTest {
    @Test
    void realRustSidecarAcceptsConfigAndEmitsReady() throws Exception {
        final Path binary = rustBinary();
        Assumptions.assumeTrue(Files.isRegularFile(binary), "build squaremap-server before running the real sidecar smoke");
        final Path root = Files.createTempDirectory("squaremap-rust-shadow");
        final BridgeBootstrapConfig config = new BridgeBootstrapConfig(
            BackendMode.RUST,
            "1.3.16-SNAPSHOT",
            new SidecarCommand(List.of(binary.toString())),
            Duration.ofSeconds(20),
            Duration.ofSeconds(2),
            root
        );
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        try {
            final BridgeConnection connection = supervisor.start(config).toCompletableFuture().get(25, TimeUnit.SECONDS);
            final CountDownLatch ready = new CountDownLatch(1);
            final CountDownLatch ack = new CountDownLatch(1);
            connection.setReadyListener(envelope -> {
                if (envelope.hasReady() && envelope.getReady().getStateRevision() == 1) ready.countDown();
            });
            connection.setAcknowledgementListener(sent -> {
                if (sent.envelope().hasConfigReplace()) ack.countDown();
            });
            final ConfigReplace candidate = validConfig();
            assertEquals(xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher.PublishResult.ACCEPTED,
                connection.publish(new BridgeEvent.ReplaceState("config", Envelope.newBuilder().setConfigReplace(candidate).build())));
            assertTrue(ack.await(10, TimeUnit.SECONDS));
            assertTrue(ready.await(10, TimeUnit.SECONDS));
            assertTrue(Files.isRegularFile(root.resolve(".squaremap-state.sqlite")));
            connection.close();
        } finally {
            supervisor.close();
        }
    }
    @Test
    void realSidecarRestartProducesReplacementAndRetainsState() throws Exception {
        final Path binary = rustBinary();
        Assumptions.assumeTrue(Files.isRegularFile(binary), "build squaremap-server before running the restart smoke");
        final Path root = Files.createTempDirectory("squaremap-rust-restart");
        final BridgeBootstrapConfig config = new BridgeBootstrapConfig(
            BackendMode.RUST, "1.3.16-SNAPSHOT", new SidecarCommand(List.of(binary.toString())),
            Duration.ofSeconds(20), Duration.ofSeconds(2), root
        );
        final SidecarSupervisor supervisor = new SidecarSupervisor();
        try {
            final BridgeConnection first = supervisor.start(config).toCompletableFuture().get(25, TimeUnit.SECONDS);
            final long firstPid = supervisor.currentProcessPidForTests();
            final CountDownLatch replacement = new CountDownLatch(1);
            final java.util.concurrent.atomic.AtomicReference<BridgeConnection> replacementConnection =
                new java.util.concurrent.atomic.AtomicReference<>();
            supervisor.setReconnectListener(connection -> {
                if (connection != first) {
                    replacementConnection.set(connection);
                    replacement.countDown();
                }
            });
            first.publish(new BridgeEvent.ReplaceState("config", Envelope.newBuilder().setConfigReplace(validConfig()).build()));
            assertTrue(Files.isRegularFile(root.resolve(".squaremap-state.sqlite")));
            ProcessHandle.of(firstPid).ifPresent(ProcessHandle::destroyForcibly);
            assertTrue(replacement.await(25, TimeUnit.SECONDS));
            assertTrue(replacementConnection.get() != null && replacementConnection.get() != first);
            assertTrue(Files.isRegularFile(root.resolve(".squaremap-state.sqlite")));
        } finally {
            supervisor.close();
        }
    }

    private static ConfigReplace validConfig() {
        final WorldSettings world = WorldSettings.newBuilder()
            .setZoomMax(3).setZoomDefault(3)
            .setBackgroundRenderIntervalSeconds(1).setBackgroundRenderMaxChunksPerInterval(1)
            .setPlayerTrackerUpdateInterval(1).setMarkerApiUpdateIntervalSeconds(1).build();
        return ConfigReplace.newBuilder().setRevision(1)
            .setGlobal(GlobalSettings.newBuilder().setHttpPort(8080).setCompressionRatio(1.0f).setHttpEnabled(false))
            .setAdvanced(AdvancedSettings.getDefaultInstance())
            .setWorld(world)
            .setLocale(LocaleSettings.newBuilder().setLanguage("lang-en.yml"))
            .setRender(RenderSettings.newBuilder().setProgressLoggingIntervalSeconds(1).setBackgroundIntervalSeconds(1).setBackgroundMaxChunksPerInterval(1))
            .setUi(UiSettings.newBuilder().setSidebarPinned("unpinned"))
            .addWorlds(WorldConfig.newBuilder().setIdentity(WorldIdentity.newBuilder().setNamespace("minecraft").setValue("overworld").setEpoch(1)).setSettings(world))
            .setPlayerPrivacyEnabled(false).setEventCaptureEnabled(true).build();
    }

    private static Path rustBinary() {
        final String configured = System.getProperty("squaremap.rustBinary", "rust/target/debug/squaremap-server");
        return Path.of(configured).toAbsolutePath().normalize();
    }
}
