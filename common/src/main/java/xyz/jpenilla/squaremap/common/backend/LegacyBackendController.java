package xyz.jpenilla.squaremap.common.backend;

import com.google.inject.Inject;
import com.google.inject.Provider;
import com.google.inject.Singleton;
import java.io.IOException;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import net.minecraft.core.BlockPos;
import xyz.jpenilla.squaremap.common.SquaremapCommon;
import xyz.jpenilla.squaremap.common.WorldManager;
import xyz.jpenilla.squaremap.common.data.DirectoryProvider;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;
import xyz.jpenilla.squaremap.common.task.render.RenderFactory;
import xyz.jpenilla.squaremap.common.util.FileUtil;

/** Java-authoritative control implementation. */
@Singleton
public final class LegacyBackendController {
    private final RenderFactory renderFactory;
    private final WorldManager worlds;
    private final DirectoryProvider directories;
    private final Provider<SquaremapCommon> common;

    @Inject
    public LegacyBackendController(
        final RenderFactory renderFactory,
        final WorldManager worlds,
        final DirectoryProvider directories,
        final Provider<SquaremapCommon> common
    ) {
        this.renderFactory = renderFactory;
        this.worlds = worlds;
        this.directories = directories;
        this.common = common;
    }

    CompletionStage<BackendResult> execute(final BackendController.BackendRequest request) {
        try {
            if (request instanceof BackendController.Reload) {
                this.common.get().reload();
                return completed(new BackendResult(BackendResult.Code.RELOADED,
                    java.util.List.of(new BackendResult.Substitution("version", new BackendResult.Text(this.common.get().version())))));
            }
            if (request instanceof BackendController.RestartProgressLogging) {
                this.worlds.worlds().forEach(world -> world.renderManager().restartRenderProgressLogging());
                return completed(BackendResult.of(BackendResult.Code.PROGRESS_LOGGING_RESTARTED));
            }
            if (request instanceof BackendController.ConfigSync) return completed(BackendResult.of(BackendResult.Code.HEALTHY));
            if (request instanceof BackendController.Health) return completed(BackendResult.of(BackendResult.Code.HEALTHY));
            final MapWorldInternal world = request.world() == null ? null : this.worlds.getWorldIfEnabled(request.world()).orElse(null);
            if (world == null) return completed(request.world() == null
                ? BackendResult.of(BackendResult.Code.INVALID_REQUEST)
                : BackendResult.world(BackendResult.Code.UNKNOWN_WORLD, request.world()));
            if (request instanceof BackendController.FullRender) {
                if (world.renderManager().isRendering()) return completed(world(BackendResult.Code.RENDER_IN_PROGRESS, world));
                world.renderManager().startRender(this.renderFactory.createFullRender(world));
                return completed(world(BackendResult.Code.FULL_RENDER_STARTED, world));
            }
            if (request instanceof BackendController.RadiusRender radius) {
                if (radius.radius() < 1) return completed(world(BackendResult.Code.INVALID_REQUEST, world));
                if (world.renderManager().isRendering()) return completed(world(BackendResult.Code.RENDER_IN_PROGRESS, world));
                world.renderManager().startRender(this.renderFactory.createRadiusRender(
                    world, new BlockPos(radius.centerX(), 0, radius.centerZ()), radius.radius()));
                return completed(world(BackendResult.Code.RADIUS_RENDER_STARTED, world));
            }
            if (request instanceof BackendController.CancelRender) {
                if (!world.renderManager().isRendering()) return completed(world(BackendResult.Code.RENDER_NOT_IN_PROGRESS, world));
                world.renderManager().cancelRender();
                return completed(world(BackendResult.Code.RENDER_CANCELLED, world));
            }
            if (request instanceof BackendController.PauseRenders) {
                final boolean paused = !world.renderManager().rendersPaused();
                world.renderManager().pauseRenders(paused);
                return completed(world(paused ? BackendResult.Code.RENDERS_PAUSED : BackendResult.Code.RENDERS_RESUMED, world));
            }
            if (request instanceof BackendController.ResetMap) {
                try {
                    FileUtil.deleteContentsRecursively(this.directories.getAndCreateTilesDirectory(world.serverLevel()));
                    world.didReset();
                    return completed(world(BackendResult.Code.MAP_RESET, world));
                } catch (final IOException failure) {
                    return completed(world(BackendResult.Code.FAILED, world));
                }
            }
            return completed(BackendResult.of(BackendResult.Code.INVALID_REQUEST));
        } catch (final RuntimeException failure) {
            return completed(BackendResult.of(BackendResult.Code.FAILED));
        }
    }

    private static BackendResult world(final BackendResult.Code code, final MapWorldInternal world) {
        return BackendResult.world(code, world.identifier());
    }

    private static CompletionStage<BackendResult> completed(final BackendResult result) {
        return CompletableFuture.completedFuture(result);
    }
}
