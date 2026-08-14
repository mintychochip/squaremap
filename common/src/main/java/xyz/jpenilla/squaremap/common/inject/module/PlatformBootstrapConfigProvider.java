package xyz.jpenilla.squaremap.common.inject.module;

import com.google.inject.Provider;
import java.util.Objects;
import java.util.jar.Manifest;
import xyz.jpenilla.squaremap.common.bridge.process.BridgeBootstrapConfig;
import xyz.jpenilla.squaremap.common.util.Util;

final class PlatformBootstrapConfigProvider implements Provider<BridgeBootstrapConfig> {
    private final String version;

    PlatformBootstrapConfigProvider(final String version) {
        this.version = Objects.requireNonNull(version, "version");
    }

    @Override
    public BridgeBootstrapConfig get() {
        return BridgeBootstrapConfig.configured(this.version);
    }

    static String packagedVersion(final Class<?> anchor) {
        return version(Objects.requireNonNull(
            Util.manifest(anchor),
            "Missing squaremap manifest"
        ));
    }

    static String version(final Manifest manifest) {
        return Objects.requireNonNull(
            manifest.getMainAttributes().getValue("squaremap-version"),
            "squaremap manifest missing 'squaremap-version' attribute"
        );
    }
}
