package xyz.jpenilla.squaremap.common.inject.module;

import com.google.inject.Inject;
import com.google.inject.Provider;
import xyz.jpenilla.squaremap.common.SquaremapPlatform;
import xyz.jpenilla.squaremap.common.bridge.process.BridgeBootstrapConfig;

final class PlatformBootstrapConfigProvider implements Provider<BridgeBootstrapConfig> {
    private final SquaremapPlatform platform;

    @Inject
    PlatformBootstrapConfigProvider(final SquaremapPlatform platform) {
        this.platform = platform;
    }

    @Override
    public BridgeBootstrapConfig get() {
        return BridgeBootstrapConfig.configured(this.platform.version());
    }
}
