package xyz.jpenilla.squaremap.common.backend;

import com.google.inject.Inject;
import com.google.inject.Singleton;
import java.util.Objects;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import org.checkerframework.checker.nullness.qual.Nullable;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.common.config.ConfigBridgeExporter;

@Singleton
public final class BackendController implements AutoCloseable {
    private final BackendExecutor bridge;
    private final AutoCloseable bridgeLifecycle;
    private final ConfigBridgeExporter configExporter;

    @Inject
    public BackendController(
        final BridgeBackendController bridge,
        final ConfigBridgeExporter configExporter
    ) {
        this(bridge::execute, bridge, configExporter);
    }

    BackendController(final BackendExecutor bridge) {
        this(bridge, null, null);
    }

    private BackendController(
        final BackendExecutor bridge,
        final AutoCloseable bridgeLifecycle,
        final ConfigBridgeExporter configExporter
    ) {
        this.bridge = Objects.requireNonNull(bridge);
        this.bridgeLifecycle = bridgeLifecycle;
        this.configExporter = configExporter;
    }

    public CompletionStage<BackendResult> fullRender(final WorldIdentifier world) { return route(new FullRender(world)); }
    public CompletionStage<BackendResult> radiusRender(final WorldIdentifier world, int x, int z, int radius) { return route(new RadiusRender(world, x, z, radius)); }
    public CompletionStage<BackendResult> cancelRender(final WorldIdentifier world) { return route(new CancelRender(world)); }
    public CompletionStage<BackendResult> pauseRenders(final WorldIdentifier world) { return route(new PauseRenders(world)); }
    public CompletionStage<BackendResult> resetMap(final WorldIdentifier world) { return route(new ResetMap(world)); }
    public CompletionStage<BackendResult> reload() { return route(new Reload()); }
    public CompletionStage<BackendResult> health() { return route(new Health()); }

    CompletionStage<BackendResult> publishConfig() {
        return this.configExporter == null
            ? CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.INVALID_CONFIG))
            : route(new ConfigSync(this.configExporter.export(xyz.jpenilla.squaremap.common.bridge.process.BackendMode.RUST)));
    }

    CompletionStage<BackendResult> restartProgressLogging() { return route(new RestartProgressLogging()); }

    private CompletionStage<BackendResult> route(final BackendRequest request) {
        try {
            return this.bridge.execute(request);
        } catch (final Throwable failure) {
            return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.FAILED));
        }
    }

    @FunctionalInterface
    interface BackendExecutor {
        CompletionStage<BackendResult> execute(BackendRequest request);
    }

    sealed interface BackendRequest permits FullRender, RadiusRender, CancelRender, PauseRenders, ResetMap, Reload, Health, RestartProgressLogging, ConfigSync {
        @Nullable WorldIdentifier world();
    }

    record FullRender(WorldIdentifier world) implements BackendRequest {
        public FullRender { Objects.requireNonNull(world); }
    }
    record RadiusRender(WorldIdentifier world, int centerX, int centerZ, int radius) implements BackendRequest {
        public RadiusRender { Objects.requireNonNull(world); }
    }
    record CancelRender(WorldIdentifier world) implements BackendRequest {
        public CancelRender { Objects.requireNonNull(world); }
    }
    record PauseRenders(WorldIdentifier world) implements BackendRequest {
        public PauseRenders { Objects.requireNonNull(world); }
    }
    record ResetMap(WorldIdentifier world) implements BackendRequest {
        public ResetMap { Objects.requireNonNull(world); }
    }
    record Reload() implements BackendRequest {
        public WorldIdentifier world() { return null; }
    }
    record Health() implements BackendRequest {
        public WorldIdentifier world() { return null; }
    }
    record RestartProgressLogging() implements BackendRequest {
        public WorldIdentifier world() { return null; }
    }
    record ConfigSync(xyz.jpenilla.squaremap.bridge.v1.ConfigReplace config) implements BackendRequest {
        public ConfigSync { Objects.requireNonNull(config); }
        public WorldIdentifier world() { return null; }
    }

    @Override
    public void close() {
        if (this.bridgeLifecycle != null) {
            try {
                this.bridgeLifecycle.close();
            } catch (final Exception failure) {
                throw new IllegalStateException("failed to close bridge backend", failure);
            }
        }
    }

    public void abortForRestart() {
        if (this.bridgeLifecycle instanceof BridgeBackendController bridgeController) {
            bridgeController.abortForRestart();
        }
    }
}
