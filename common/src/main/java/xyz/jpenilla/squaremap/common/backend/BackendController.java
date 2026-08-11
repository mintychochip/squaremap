package xyz.jpenilla.squaremap.common.backend;

import com.google.inject.Inject;
import com.google.inject.Singleton;
import java.util.Objects;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import org.checkerframework.checker.nullness.qual.Nullable;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.common.Logging;
import xyz.jpenilla.squaremap.common.bridge.process.BackendMode;
import xyz.jpenilla.squaremap.common.bridge.process.BridgeBootstrapConfig;

@Singleton
public final class BackendController {
    private final BackendMode mode;
    private final BackendExecutor legacy;
    private final BackendExecutor bridge;
    private final xyz.jpenilla.squaremap.common.config.ConfigBridgeExporter configExporter;
    @Inject
    public BackendController(final BridgeBootstrapConfig config, final LegacyBackendController legacy, final BridgeBackendController bridge,
                             final xyz.jpenilla.squaremap.common.config.ConfigBridgeExporter configExporter) {
        this(config.backendMode(), legacy::execute, bridge::execute, configExporter);
    }
    private BackendController(final BackendMode mode, final BackendExecutor legacy, final BackendExecutor bridge,
                              final xyz.jpenilla.squaremap.common.config.ConfigBridgeExporter configExporter) {
        this.mode = Objects.requireNonNull(mode); this.legacy = Objects.requireNonNull(legacy);
        this.bridge = Objects.requireNonNull(bridge); this.configExporter = Objects.requireNonNull(configExporter);
    }
    BackendController(final BackendMode mode, final BackendExecutor legacy, final BackendExecutor bridge) {
        this.mode = Objects.requireNonNull(mode); this.legacy = Objects.requireNonNull(legacy); this.bridge = Objects.requireNonNull(bridge); this.configExporter = null;
    }
    public CompletionStage<BackendResult> fullRender(final WorldIdentifier world) { return route(new FullRender(world)); }
    public CompletionStage<BackendResult> radiusRender(final WorldIdentifier world, int x, int z, int radius) { return route(new RadiusRender(world,x,z,radius)); }
    public CompletionStage<BackendResult> cancelRender(final WorldIdentifier world) { return route(new CancelRender(world)); }
    public CompletionStage<BackendResult> pauseRenders(final WorldIdentifier world) { return route(new PauseRenders(world)); }
    public CompletionStage<BackendResult> resetMap(final WorldIdentifier world) { return route(new ResetMap(world)); }
    public CompletionStage<BackendResult> reload() { return route(new Reload()); }
    public CompletionStage<BackendResult> health() { return route(new Health()); }
    CompletionStage<BackendResult> publishConfig() { return this.configExporter == null ? CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.INVALID_CONFIG)) : route(new ConfigSync(this.configExporter.export())); }
    CompletionStage<BackendResult> restartProgressLogging() { return route(new RestartProgressLogging()); }
    private CompletionStage<BackendResult> route(final BackendRequest request) {
        try { return switch (mode) {
            case JAVA -> legacy.execute(request);
            case RUST -> bridge.execute(request);
            case SHADOW -> { final CompletionStage<BackendResult> java = legacy.execute(request); try { bridge.execute(request).whenComplete((r,e)-> { if(e!=null) Logging.logger().warn("Shadow backend control request failed",e); }); } catch(Throwable e) { Logging.logger().warn("Shadow backend control request could not be mirrored",e); } yield java; }
        }; } catch(Throwable failure) { return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.FAILED)); }
    }
    @FunctionalInterface interface BackendExecutor { CompletionStage<BackendResult> execute(BackendRequest request); }
    sealed interface BackendRequest permits FullRender,RadiusRender,CancelRender,PauseRenders,ResetMap,Reload,Health,RestartProgressLogging,ConfigSync { @Nullable WorldIdentifier world(); }
    record FullRender(WorldIdentifier world) implements BackendRequest { public FullRender { Objects.requireNonNull(world); } }
    record RadiusRender(WorldIdentifier world,int centerX,int centerZ,int radius) implements BackendRequest { public RadiusRender { Objects.requireNonNull(world); } }
    record CancelRender(WorldIdentifier world) implements BackendRequest { public CancelRender { Objects.requireNonNull(world); } }
    record PauseRenders(WorldIdentifier world) implements BackendRequest { public PauseRenders { Objects.requireNonNull(world); } }
    record ResetMap(WorldIdentifier world) implements BackendRequest { public ResetMap { Objects.requireNonNull(world); } }
    record Reload() implements BackendRequest { public WorldIdentifier world(){return null;} }
    record Health() implements BackendRequest { public WorldIdentifier world(){return null;} }
    record RestartProgressLogging() implements BackendRequest { public WorldIdentifier world(){return null;} }
    record ConfigSync(xyz.jpenilla.squaremap.bridge.v1.ConfigReplace config) implements BackendRequest { public ConfigSync { Objects.requireNonNull(config); } public WorldIdentifier world(){return null;} }
}
