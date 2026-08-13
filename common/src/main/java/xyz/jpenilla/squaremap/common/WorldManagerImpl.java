package xyz.jpenilla.squaremap.common;

import com.google.inject.Inject;
import com.google.inject.Provider;
import com.google.inject.Singleton;
import java.util.Collection;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicBoolean;
import net.minecraft.server.level.ServerLevel;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.checker.nullness.qual.Nullable;
import org.checkerframework.framework.qual.DefaultQualifier;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.common.backend.BackendResult;
import xyz.jpenilla.squaremap.common.backend.BackendControllerSupport;
import xyz.jpenilla.squaremap.common.config.ConfigManager;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;
import xyz.jpenilla.squaremap.common.util.Util;
import xyz.jpenilla.squaremap.common.util.chunksnapshot.EmptySectionHolder;

@DefaultQualifier(NonNull.class)
@Singleton
public class WorldManagerImpl implements WorldManager {
    private final Map<WorldIdentifier, MapWorldInternal> worlds = new ConcurrentHashMap<>();
    private final MapWorldInternal.Factory factory;
    protected final ServerAccess serverAccess;
    private final ConfigManager configManager;
    private final @Nullable Provider<BackendControllerSupport> backendSupport;
    private final AtomicBoolean bridgeConfigPending = new AtomicBoolean();
    private final AtomicBoolean bridgeConfigRepeat = new AtomicBoolean();
    private volatile boolean started;
    private final Provider<xyz.jpenilla.squaremap.common.bridge.snapshot.SnapshotRequestHandler> snapshotHandler;
    @Inject
    protected WorldManagerImpl(
        final MapWorldInternal.Factory factory,
        final ServerAccess serverAccess,
        final ConfigManager configManager,
        final Provider<BackendControllerSupport> backendSupport,
        final Provider<xyz.jpenilla.squaremap.common.bridge.snapshot.SnapshotRequestHandler> snapshotHandler
    ) {
        this.factory = factory;
        this.serverAccess = serverAccess;
        this.configManager = configManager;
        this.backendSupport = backendSupport;
        this.snapshotHandler = snapshotHandler;
    }

    protected WorldManagerImpl(final MapWorldInternal.Factory factory, final ServerAccess serverAccess, final ConfigManager configManager) {
        this(factory, serverAccess, configManager, null, null);
    }

    @Override
    public Collection<MapWorldInternal> worlds() {
        return Collections.unmodifiableCollection(this.worlds.values());
    }

    @Override
    public Optional<MapWorldInternal> getWorldIfEnabled(final WorldIdentifier worldIdentifier) {
        return Optional.ofNullable(this.worlds.get(worldIdentifier));
    }

    @Override
    public Optional<MapWorldInternal> getWorldIfEnabled(final ServerLevel level) {
        return this.getWorldIfEnabled(Util.worldIdentifier(level));
    }

    public void initWorld(final ServerLevel level) {
        EmptySectionHolder.init(level.palettedContainerFactory());
        final WorldIdentifier identifier = Util.worldIdentifier(level);
        if (this.worlds.containsKey(identifier)) {
            throw new IllegalStateException("MapWorld already exists for '" + identifier.asString() + "'");
        }
        if (this.configManager.worldConfig(level).MAP_ENABLED) {
            this.worlds.put(identifier, this.factory.create(level));
            if (this.started) {
                this.publishBridgeConfig();
            }
        }
    }
    public void start() {
        for (final ServerLevel level : this.serverAccess.levels()) {
            this.initWorld(level);
        }
        this.started = true;
        if (!this.worlds.isEmpty()) {
            this.publishBridgeConfig();
        }
    }

    private void publishBridgeConfig() {
        if (this.backendSupport == null) {
            return;
        }
        if (!this.bridgeConfigPending.compareAndSet(false, true)) {
            this.bridgeConfigRepeat.set(true);
            return;
        }
        try {
            this.backendSupport.get().publishConfig().whenComplete((result, failure) -> {
                this.bridgeConfigPending.set(false);
                if (failure != null) {
                    Logging.logger().warn("Bridge configuration after world change failed", failure);
                } else if (result.code() != BackendResult.Code.HEALTHY) {
                    Logging.logger().warn("Bridge configuration after world change was not accepted: {}", result.code());
                }
                if (this.bridgeConfigRepeat.getAndSet(false)) {
                    this.publishBridgeConfig();
                }
            });
        } catch (final RuntimeException failure) {
            this.bridgeConfigPending.set(false);
            Logging.logger().warn("Bridge configuration after world change could not be published", failure);
            if (this.bridgeConfigRepeat.getAndSet(false)) {
                this.publishBridgeConfig();
            }
        }
    }

    public void worldUnloaded(final ServerLevel world) {
        final WorldIdentifier identifier = Util.worldIdentifier(world);
        final @Nullable MapWorldInternal removed = this.worlds.remove(identifier);
        if (this.snapshotHandler != null) this.snapshotHandler.get().invalidateWorld(identifier, Long.MAX_VALUE);
        if (removed != null) {
            tryShutdown(removed);
            if (this.started) {
                this.publishBridgeConfig();
            }
        }
    }

    public void shutdown() {
        this.started = false;
        final List<MapWorldInternal> worlds = List.copyOf(this.worlds.values());
        this.worlds.clear();
        for (final MapWorldInternal world : worlds) {
            tryShutdown(world);
        }
    }
    private static void tryShutdown(final MapWorldInternal mapWorld) {
        try {
            mapWorld.shutdown();
        } catch (final Exception ex) {
            Logging.logger().error("Exception shutting down map world '{}'", mapWorld.identifier().asString(), ex);
        }
    }
}
