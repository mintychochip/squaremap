package xyz.jpenilla.squaremap.common.bridge.process;

import static org.junit.jupiter.api.Assertions.assertEquals;
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
            final Path data = Files.createTempDirectory("squaremap-wiring");
            Config.BRIDGE_BACKEND_MODE = "SHADOW";
            Config.BRIDGE_SIDECAR_COMMAND = java.util.List.of("fixture-sidecar");
            Config.BRIDGE_RUST_OUTPUT_ROOT = data.resolve("rust").toString();
            final Injector injector = Guice.createInjector(new AbstractModule() {
                @Override protected void configure() {
                    bind(BridgeBootstrapConfig.class).toProvider(BridgeBootstrapConfig::configured).in(Singleton.class);
                    bind(SidecarSupervisor.class).in(Singleton.class);
                }
            });
            final BridgeBootstrapConfig config = injector.getInstance(BridgeBootstrapConfig.class);
            assertEquals(BackendMode.SHADOW, config.backendMode());
            assertSame(config, injector.getInstance(BridgeBootstrapConfig.class));
            assertSame(injector.getInstance(SidecarSupervisor.class), injector.getInstance(SidecarSupervisor.class));
            assertTrue(IconRegistry.class.isAnnotationPresent(Singleton.class));
        } finally {
            Config.BRIDGE_BACKEND_MODE = oldMode;
            Config.BRIDGE_SIDECAR_COMMAND = oldCommand;
            Config.BRIDGE_RUST_OUTPUT_ROOT = oldRoot;
        }
    }

    @Test
    void configuredSupervisorCarriesFramedReplacementToStartedSidecar() throws Exception {
        final String oldMode = Config.BRIDGE_BACKEND_MODE;
        final var oldCommand = Config.BRIDGE_SIDECAR_COMMAND;
        final String oldRoot = Config.BRIDGE_RUST_OUTPUT_ROOT;
        final Path data = Files.createTempDirectory("squaremap-wiring-sidecar");
        try {
            Config.BRIDGE_BACKEND_MODE = "SHADOW";
            Config.BRIDGE_RUST_OUTPUT_ROOT = data.resolve("rust").toString();
            Config.BRIDGE_SIDECAR_COMMAND = java.util.List.of(
                System.getProperty("java.home") + "/bin/java", "-cp", System.getProperty("java.class.path"),
                FakeSidecar.class.getName(), "--behavior=ack-publish", "--expected-root=" + Config.BRIDGE_RUST_OUTPUT_ROOT
            );
            final Injector injector = Guice.createInjector(new AbstractModule() {
                @Override protected void configure() {
                    bind(BridgeBootstrapConfig.class).toProvider(BridgeBootstrapConfig::configured).in(Singleton.class);
                    bind(SidecarSupervisor.class).in(Singleton.class);
                }
            });
            final SidecarSupervisor supervisor = injector.getInstance(SidecarSupervisor.class);
            final BridgeConnection connection = supervisor.start(injector.getInstance(BridgeBootstrapConfig.class)).toCompletableFuture().get();
            supervisor.publish(new xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent.ReplaceState(
                "players", xyz.jpenilla.squaremap.bridge.v1.Envelope.newBuilder()
                    .setPlayersReplace(xyz.jpenilla.squaremap.bridge.v1.PlayersReplace.getDefaultInstance()).build()
            ));
            connection.close();
            supervisor.close();
        } finally {
            Config.BRIDGE_BACKEND_MODE = oldMode;
            Config.BRIDGE_SIDECAR_COMMAND = oldCommand;
            Config.BRIDGE_RUST_OUTPUT_ROOT = oldRoot;
        }
    }
}
