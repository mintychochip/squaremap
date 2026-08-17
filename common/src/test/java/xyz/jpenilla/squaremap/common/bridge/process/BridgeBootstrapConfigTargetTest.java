package xyz.jpenilla.squaremap.common.bridge.process;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.common.config.Config;

final class BridgeBootstrapConfigTargetTest {
    @Test
    void mapsSupportedTargets() {
        assertEquals("x86_64-pc-windows-msvc", BridgeBootstrapConfig.targetTriple("Windows 11", "amd64"));
        assertEquals("aarch64-unknown-linux-gnu", BridgeBootstrapConfig.targetTriple("Linux", "arm64"));
        assertEquals("x86_64-apple-darwin", BridgeBootstrapConfig.targetTriple("Mac OS X", "x86_64"));
        assertEquals("aarch64-apple-darwin", BridgeBootstrapConfig.targetTriple("Darwin", "aarch64"));
    }

    @Test
    void rejectsUnsupportedTargets() {
        assertThrows(IllegalArgumentException.class, () -> BridgeBootstrapConfig.targetTriple("Solaris", "x86_64"));
        assertThrows(IllegalArgumentException.class, () -> BridgeBootstrapConfig.targetTriple("Linux", "ppc64le"));
    }

    @Test
    void configuredModeRejectsJavaAndShadow() {
        final String oldMode = Config.BRIDGE_BACKEND_MODE;
        try {
            Config.BRIDGE_BACKEND_MODE = "JAVA";
            assertThrows(IllegalArgumentException.class, () -> BridgeBootstrapConfig.configured("test-version"));
            Config.BRIDGE_BACKEND_MODE = "SHADOW";
            assertThrows(IllegalArgumentException.class, () -> BridgeBootstrapConfig.configured("test-version"));
        } finally {
            Config.BRIDGE_BACKEND_MODE = oldMode;
        }
    }
}
