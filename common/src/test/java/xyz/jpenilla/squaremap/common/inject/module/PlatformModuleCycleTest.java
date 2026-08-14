package xyz.jpenilla.squaremap.common.inject.module;

import static org.junit.jupiter.api.Assertions.assertEquals;

import com.google.inject.Guice;
import com.google.inject.Inject;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.common.SquaremapPlatform;
import xyz.jpenilla.squaremap.common.bridge.process.BridgeBootstrapConfig;

final class PlatformModuleCycleTest {
    @Test
    void classBasedPlatformDoesNotResolvePlatformToBuildBridgeConfig() throws Exception {
        final String version = PlatformBootstrapConfigProvider.version(new java.util.jar.Manifest(
            PlatformModuleCycleTest.class.getResourceAsStream("/META-INF/MANIFEST.MF")
        ));
        final var injector = Guice.createInjector(new com.google.inject.AbstractModule() {
            @Override
            protected void configure() {
                bind(SquaremapPlatform.class).to(ConstructingPlatform.class);
                bind(BridgeBootstrapConfig.class).toProvider(new PlatformBootstrapConfigProvider(version));
            }
        });

        assertEquals("injector-cycle-test", injector.getInstance(ConstructingPlatform.class).config.pluginVersion());
    }

    static final class ConstructingPlatform implements SquaremapPlatform {
        private final BridgeBootstrapConfig config;

        @Inject
        ConstructingPlatform(final BridgeBootstrapConfig config) {
            this.config = config;
        }

        @Override public void startCallback() {
        }

        @Override public void stopCallback() {
        }

        @Override public String version() {
            throw new AssertionError("version must not be called during injector construction");
        }
    }
}
