package xyz.jpenilla.squaremap.fabric.data;

import com.google.inject.assistedinject.Assisted;
import com.google.inject.assistedinject.AssistedInject;
import net.minecraft.server.level.ServerLevel;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.framework.qual.DefaultQualifier;
import xyz.jpenilla.squaremap.common.config.ConfigManager;
import xyz.jpenilla.squaremap.common.data.DirectoryProvider;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;
import xyz.jpenilla.squaremap.common.bridge.state.BridgeStatePublisher;

@DefaultQualifier(NonNull.class)
public final class FabricMapWorld extends MapWorldInternal {
    private final BridgeStatePublisher statePublisher;
    @AssistedInject
    private FabricMapWorld(
        @Assisted final ServerLevel level,
        final DirectoryProvider directoryProvider,
        final ConfigManager configManager,
        final BridgeStatePublisher statePublisher
    ) {
        super(level, directoryProvider, configManager);
        this.statePublisher = statePublisher;
    }

    public void tickEachSecond(final long tick) {
        if (tick % (this.config().MARKER_API_UPDATE_INTERVAL_SECONDS * 20L) == 0) {
            this.statePublisher.publishMarkers(this);
        }
    }
}
