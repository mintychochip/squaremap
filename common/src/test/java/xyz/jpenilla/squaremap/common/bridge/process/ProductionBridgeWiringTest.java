package xyz.jpenilla.squaremap.common.bridge.process;

import java.time.Duration;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.google.inject.AbstractModule;
import com.google.inject.Guice;
import com.google.inject.Injector;
import com.google.inject.Singleton;
import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.common.IconRegistry;
import xyz.jpenilla.squaremap.common.config.Config;

final class ProductionBridgeWiringTest {
    @Test
    void productionEquivalentGraphLazilyLoadsConfiguredModeAndScopesBridgeObjects() throws Exception {
        final String oldMode = Config.BRIDGE_BACKEND_MODE;
        final var oldCommand = Config.BRIDGE_SIDECAR_COMMAND;
        final String oldRoot = Config.BRIDGE_RUST_OUTPUT_ROOT;
        try {
            Files.createTempDirectory("squaremap-wiring");
            final Injector injector = Guice.createInjector(new AbstractModule() {
                @Override protected void configure() {
                    bind(SupervisorHolder.class);
                    bind(SecondSupervisorHolder.class);
                }
            });
            final SupervisorHolder first = injector.getInstance(SupervisorHolder.class);
            final SecondSupervisorHolder second = injector.getInstance(SecondSupervisorHolder.class);
            assertSame(first.supervisor, second.supervisor);
            assertSame(first.supervisor, injector.getInstance(SidecarSupervisor.class));
            assertTrue(SidecarSupervisor.class.isAnnotationPresent(Singleton.class));
            assertTrue(IconRegistry.class.isAnnotationPresent(Singleton.class));
        } finally {
            Config.BRIDGE_BACKEND_MODE = oldMode;
            Config.BRIDGE_SIDECAR_COMMAND = oldCommand;
            Config.BRIDGE_RUST_OUTPUT_ROOT = oldRoot;
        }
    }

    private static final class SupervisorHolder {
        private final SidecarSupervisor supervisor;

        @com.google.inject.Inject
        private SupervisorHolder(final SidecarSupervisor supervisor) {
            this.supervisor = supervisor;
        }
    }

    private static final class SecondSupervisorHolder {
        private final SidecarSupervisor supervisor;

        @com.google.inject.Inject
        private SecondSupervisorHolder(final SidecarSupervisor supervisor) {
            this.supervisor = supervisor;
        }
    }

    @Test
    void configuredSupervisorCarriesFramedReplacementToStartedSidecar() throws Exception {
        final String oldMode = Config.BRIDGE_BACKEND_MODE;
        final var oldCommand = Config.BRIDGE_SIDECAR_COMMAND;
        final String oldRoot = Config.BRIDGE_RUST_OUTPUT_ROOT;
        final Path data = Files.createTempDirectory("squaremap-wiring-sidecar");
        SidecarSupervisor supervisor = null;
        try {
            Config.BRIDGE_BACKEND_MODE = "SHADOW";
            final Path rustRoot = data.resolve("rust").toAbsolutePath();
            Config.BRIDGE_RUST_OUTPUT_ROOT = rustRoot.toString();
            final var command = java.util.List.of(
                System.getProperty("java.home") + "/bin/java", "-cp", System.getProperty("java.class.path"),
                FakeSidecar.class.getName(), "--behavior=ack-publish", "--expected-root=" + rustRoot
            );
            Config.BRIDGE_SIDECAR_COMMAND = command;
            final BridgeBootstrapConfig config = new BridgeBootstrapConfig(
                BackendMode.SHADOW,
                "fixture-version",
                new SidecarCommand(command),
                Duration.ofSeconds(10),
                Duration.ofSeconds(2),
                rustRoot
            );
            final Injector injector = Guice.createInjector(new AbstractModule() {
                @Override protected void configure() {
                    bind(BridgeBootstrapConfig.class).toInstance(config);
                }
            });
            supervisor = injector.getInstance(SidecarSupervisor.class);
            final BridgeConnection connection = supervisor.start(config).toCompletableFuture().get();
            supervisor.publish(new xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent.ReplaceState(
                "players", xyz.jpenilla.squaremap.bridge.v1.Envelope.newBuilder()
                    .setPlayersReplace(xyz.jpenilla.squaremap.bridge.v1.PlayersReplace.getDefaultInstance()).build()
            ));
            connection.close();
        } finally {
            if (supervisor != null) {
                supervisor.close();
            }
            Config.BRIDGE_BACKEND_MODE = oldMode;
            Config.BRIDGE_SIDECAR_COMMAND = oldCommand;
            Config.BRIDGE_RUST_OUTPUT_ROOT = oldRoot;
        }
    }
    @Test
    void rustIsTheDefaultWebBackend() {
        assertEquals("RUST", Config.BRIDGE_BACKEND_MODE);
        assertTrue(BackendLifecyclePolicy.rustHttpOwner(BackendLifecyclePolicy.configuredMode()));
        assertFalse(BackendLifecyclePolicy.javaHttpOwner(BackendLifecyclePolicy.configuredMode()));
        assertFalse(BackendLifecyclePolicy.javaCacheOwner(BackendLifecyclePolicy.configuredMode()));
        assertFalse(BackendLifecyclePolicy.javaDirtyOwner(BackendLifecyclePolicy.configuredMode()));
        assertFalse(BackendLifecyclePolicy.javaRenderOwner(BackendLifecyclePolicy.configuredMode()));
    }
}
