package xyz.jpenilla.squaremap.common.task.render;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Timer;
import java.util.concurrent.CancellationException;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.Executor;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.locks.LockSupport;
import java.util.function.BooleanSupplier;
import java.util.function.Supplier;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.StainedGlassBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import org.checkerframework.checker.nullness.qual.MonotonicNonNull;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.checker.nullness.qual.Nullable;
import org.checkerframework.framework.qual.DefaultQualifier;
import xyz.jpenilla.squaremap.api.Pair;
import xyz.jpenilla.squaremap.common.Logging;
import xyz.jpenilla.squaremap.common.config.Messages;
import xyz.jpenilla.squaremap.common.data.BiomeColors;
import xyz.jpenilla.squaremap.common.data.ChunkCoordinate;
import xyz.jpenilla.squaremap.common.data.Image;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;
import xyz.jpenilla.squaremap.common.data.RegionCoordinate;
import xyz.jpenilla.squaremap.common.util.ChunkHashMapKey;
import xyz.jpenilla.squaremap.common.util.RenderPrimitiveEngine;
import xyz.jpenilla.squaremap.common.util.Colors;
import xyz.jpenilla.squaremap.common.util.ConcurrentFIFOLoadingCache;
import xyz.jpenilla.squaremap.common.util.Numbers;
import xyz.jpenilla.squaremap.common.util.Util;
import xyz.jpenilla.squaremap.common.util.chunksnapshot.ChunkSnapshot;
import xyz.jpenilla.squaremap.common.util.chunksnapshot.ChunkSnapshotProvider;
import xyz.jpenilla.squaremap.common.util.chunksnapshot.ChunkSnapshotProviderFactory;

@DefaultQualifier(NonNull.class)
public abstract class AbstractRender implements Runnable {
    private final ExecutorService executorService;
    private final Executor executor;
    private final Supplier<ChunkSnapshotProvider> createChunkSnapshotProvider;
    private final @Nullable Map<BiomeCacheKey, BiomeColors> biomeColors;
    private ChunkSnapshotManager chunks;
    private final ChunkRenderEngine chunkRenderEngine;
    private volatile @MonotonicNonNull Thread thread;
    protected volatile State state = State.RUNNING;
    protected final MapWorldInternal mapWorld;
    protected final ServerLevel level;
    protected final AtomicInteger processedChunks = new AtomicInteger(0);
    protected final AtomicInteger processedRegions = new AtomicInteger(0);

    protected volatile @Nullable Pair<Timer, RenderProgress> progress = null;

    protected AbstractRender(
        final MapWorldInternal world,
        final ChunkSnapshotProviderFactory chunkSnapshotProviderFactory
    ) {
        this(world, chunkSnapshotProviderFactory, createRenderWorkerPool(world));
    }

    protected AbstractRender(
        final MapWorldInternal mapWorld,
        final ChunkSnapshotProviderFactory chunkSnapshotProviderFactory,
        final ExecutorService workerPool
    ) {
        this.mapWorld = mapWorld;
        this.executorService = workerPool;
        this.executor = new RenderWorkerExecutor(workerPool, this::running);
        this.level = mapWorld.serverLevel();
        this.createChunkSnapshotProvider = () -> chunkSnapshotProviderFactory.createChunkSnapshotProvider(this.level);
        this.chunks = this.createChunkSnapshotManager();
        this.biomeColors = this.mapWorld.config().MAP_BIOMES
            ? new ConcurrentHashMap<>()
            : null; // this should be null if we are not mapping biomes
        this.chunkRenderEngine = new ChunkRenderEngine(new ChunkRenderEngine.Adapter() {
            @Override public ChunkRenderEngine.Settings settings() {
                final var config = AbstractRender.this.mapWorld.config();
                return new ChunkRenderEngine.Settings(
                    config.MAP_MAX_HEIGHT, config.MAP_ITERATE_UP, config.MAP_GLASS_CLEAR,
                    config.MAP_WATER_CHECKERBOARD, config.MAP_WATER_CLEAR,
                    config.MAP_LAVA_CHECKERBOARD, config.MAP_BIOMES, config.MAP_BIOMES_BLEND
                );
            }
            @Override public int mapColor(final net.minecraft.world.level.block.state.BlockState state) {
                return AbstractRender.this.mapWorld.getMapColor(state);
            }
            @Override public boolean invisibleBlock(final net.minecraft.world.level.block.Block block) {
                return AbstractRender.this.mapWorld.advanced().invisibleBlocks.contains(block);
            }
            @Override public boolean iterateUpBaseBlock(final net.minecraft.world.level.block.Block block) {
                return AbstractRender.this.mapWorld.advanced().iterateUpBaseBlocks.contains(block);
            }
            @Override public ChunkRenderEngine.@Nullable BiomeModifier biomeModifier() {
                if (AbstractRender.this.biomeColors == null) return null;
                return (color, chunk, pos, blendRadius) -> AbstractRender.this.biomeColors
                    .computeIfAbsent(new BiomeCacheKey(Thread.currentThread(), blendRadius), $ -> new BiomeColors(AbstractRender.this.mapWorld, AbstractRender.this.chunks, blendRadius))
                    .modifyColorFromBiome(color, chunk, pos);
            }
            @Override public boolean running() { return AbstractRender.this.running(); }
            @Override public boolean rendersPaused() { return AbstractRender.this.mapWorld.renderManager().rendersPaused(); }
            @Override public boolean shouldRenderColumn(final int blockX, final int blockZ) { return AbstractRender.this.mapWorld.visibilityLimit().shouldRenderColumn(blockX, blockZ); }
            @Override public void sleep(final long millis) { AbstractRender.sleep((int) millis); }
        });
    }

    private int maximumActiveChunkRequests() {
        final int factor = Integer.getInteger("squaremap.maximumActiveChunkRequestsFactor", 48);
        final int value = ((ThreadPoolExecutor) this.executorService).getCorePoolSize() * factor;
        return Integer.getInteger("squaremap.maximumActiveChunkRequests", value);
    }

    protected abstract void render();

    protected final boolean running() {
        return this.state == State.RUNNING;
    }

    @Override
    public final void run() {
        if (!this.running()) {
            return;
        }

        this.thread = Thread.currentThread();

        try {
            this.render();
        } catch (final Exception ex) {
            Logging.logger().warn("Encountered exception executing render", ex);
        }

        this.renderStopped();
    }

    private void renderStopped() {
        if (this instanceof BackgroundRender) {
            return;
        }
        final State state = this.state;
        this.shutdown();
        this.mapWorld.renderManager().renderStopped(state == State.CANCELLED || state == State.RUNNING);
        final String msg = state == State.RUNNING ? Messages.LOG_FINISHED_RENDERING : Messages.LOG_CANCELLED_RENDERING;
        Logging.info(msg, "world", this.mapWorld.identifier().asString());
    }

    private synchronized void shutdown() {
        if (this.progress != null) {
            this.progress.left().cancel();
            this.progress = null;
        }

        if (!this.executorService.isShutdown()) {
            this.executorService.shutdownNow();
        }
    }

    public final void stop() {
        this.stop(State.STOPPED);
    }

    public final void cancel() {
        this.stop(State.CANCELLED);
    }

    private void stop(final State state) {
        if (this.state != State.RUNNING) {
            throw new IllegalStateException("Stop already requested");
        }
        this.state = state;
        this.shutdown();
        final Thread thread = this.thread;
        if (thread != null) {
            thread.interrupt();
        } else {
            this.renderStopped();
        }
    }

    public abstract int totalChunks();

    public abstract int totalRegions();

    public final int processedChunks() {
        return this.processedChunks.get();
    }

    public final int processedRegions() {
        return this.processedRegions.get();
    }

    protected final void clearCaches() {
        this.chunks = this.createChunkSnapshotManager();
        if (this.biomeColors != null) {
            this.biomeColors.clear();
        }
    }

    private ChunkSnapshotManager createChunkSnapshotManager() {
        return new ChunkSnapshotManager(
            this.createChunkSnapshotProvider.get(),
            this.maximumActiveChunkRequests(),
            this.mapWorld.config().MAP_BIOMES_BLEND > 0,
            this::running
        );
    }

    public final void restartProgressLogger() {
        final @Nullable RenderProgress old;
        final @Nullable Pair<Timer, RenderProgress> progress = this.progress;
        if (progress != null) {
            progress.left().cancel();
            old = progress.right();
            this.progress = RenderProgress.printProgress(this, old);
        }
    }

    protected final void mapRegion(final RegionCoordinate region) {
        final Image image = new Image(region, this.mapWorld.tilesPath(), this.mapWorld.config().ZOOM_MAX);
        final int startX = region.getChunkX();
        final int startZ = region.getChunkZ();
        final List<CompletableFuture<Void>> futures = new ArrayList<>();
        for (int chunkX = startX; chunkX < startX + 32; chunkX++) {
            futures.add(this.mapChunkColumn(image, chunkX, startZ));
        }
        for (final CompletableFuture<Void> future : futures) {
            try {
                future.get();
            } catch (final InterruptedException ignore) {
                return;
            } catch (final CancellationException | ExecutionException ex) {
                Logging.logger().warn("Exception mapping region {}", region, ex);
            }
        }
        if (this.running()) {
            this.mapWorld.saveImage(image);
        }
    }

    static ChunkRenderEngine.PixelResult renderChunkDispatch(
        final ChunkRenderEngine engine,
        final ChunkRenderEngine.PixelSink sink,
        final @Nullable ChunkSnapshot north,
        final ChunkSnapshot center,
        final @Nullable ChunkSnapshot south
    ) {
        return engine.renderChunk(sink, north, center, south);
    }

    protected final CompletableFuture<Void> mapSingleChunk(final Image image, final int chunkX, final int chunkZ) {
        final CompletableFuture<@Nullable ChunkSnapshot> chunkFuture = this.chunks.snapshot(new ChunkPos(chunkX, chunkZ));
        final CompletableFuture<@Nullable ChunkSnapshot> northChunk = this.chunks.snapshotDirect(new ChunkPos(chunkX, chunkZ - 1));

        // queue up the southern chunk in case it was stored with improper yDiff
        // https://github.com/pl3xgaming/Pl3xMap/issues/15
        final CompletableFuture<@Nullable ChunkSnapshot> southChunk;
        final int down = chunkZ + 1;
        if (Numbers.chunkToRegion(chunkZ) == Numbers.chunkToRegion(down)) {
            // Prime left and right (don't need bottom 3 neighbors primed by #snapshot)
            this.chunks.snapshotDirect(new ChunkPos(chunkX + 1, down));
            this.chunks.snapshotDirect(new ChunkPos(chunkX - 1, down));
            southChunk = this.chunks.snapshotDirect(new ChunkPos(chunkX, down));
        } else {
            // chunk belongs to a different region, add to queue
            this.mapWorld.chunkModified(new ChunkCoordinate(chunkX, down));
            southChunk = CompletableFuture.completedFuture(null);
        }

        return CompletableFuture.allOf(northChunk, chunkFuture, southChunk).thenRunAsync(() -> {
            if (!this.running()) {
                return;
            }
            final @Nullable ChunkSnapshot north = northChunk.join();
            final @Nullable ChunkSnapshot chunk = chunkFuture.join();
            if (chunk != null) {
                renderChunkDispatch(this.chunkRenderEngine, image::setPixel, north, chunk, southChunk.join());
            }

            this.processedChunks.incrementAndGet();
        }, this.executor).exceptionally(thr -> {
            Logging.logger().warn("Exception mapping chunk at [{}, {}] in {}", chunkX, chunkZ, this.mapWorld.identifier().asString(), thr);
            return null;
        });
    }

    protected final CompletableFuture<Void> mapChunkColumn(final Image image, final int chunkX, final int startChunkZ) {
        final List<CompletableFuture<ChunkSnapshot>> futures = new ArrayList<>(33);

        final CompletableFuture<@Nullable ChunkSnapshot> aboveChunkFuture = this.chunks.snapshotDirect(new ChunkPos(chunkX, startChunkZ - 1));
        futures.add(aboveChunkFuture);

        for (int chunkZ = startChunkZ; chunkZ < startChunkZ + 32; chunkZ++) {
            if (!this.mapWorld.visibilityLimit().shouldRenderChunk(chunkX, chunkZ)) {
                // skip rendering this chunk in the chunk column - it's outside the visibility limit
                // (this chunk was already excluded from the chunk count, so not incrementing that is on purpose)
                continue;
            }
            futures.add(this.chunks.snapshot(new ChunkPos(chunkX, chunkZ)));
        }

        return CompletableFuture.allOf(futures.toArray(CompletableFuture[]::new)).thenRunAsync(() -> {
            if (!this.running()) {
                return;
            }
            final int[] lastY = new int[16];
            for (final CompletableFuture<@Nullable ChunkSnapshot> future : futures) {
                final @Nullable ChunkSnapshot snapshot = future.join();
                if (future == aboveChunkFuture && snapshot != null) {
                    System.arraycopy(this.getLastYFromBottomRow(snapshot), 0, lastY, 0, lastY.length);
                } else if (snapshot != null) {
                    this.scanChunk(image, lastY, snapshot);
                    this.processedChunks.incrementAndGet();
                } else {
                    this.processedChunks.incrementAndGet();
                }
            }
        }, this.executor).exceptionally(thr -> {
            Logging.logger().warn("Exception mapping chunk column starting at [{}, {}] in {}", chunkX, startChunkZ, this.mapWorld.identifier().asString(), thr);
            return null;
        });
    }

    private void scanChunk(final Image image, final int[] lastY, final ChunkSnapshot chunk) {
        this.chunkRenderEngine.scanChunk(image, lastY, chunk);
    }

    private void scanTopRow(final Image image, final int[] lastY, final ChunkSnapshot chunk) {
        this.chunkRenderEngine.scanTopRow(image, lastY, chunk);
    }

    private int[] getLastYFromBottomRow(final ChunkSnapshot chunk) {
        return this.chunkRenderEngine.getLastYFromBottomRow(chunk);
    }


    private static ExecutorService createRenderWorkerPool(final MapWorldInternal world) {
        return Util.newFixedThreadPool(
            getThreads(world.config().MAX_RENDER_THREADS),
            Util.squaremapThreadFactory("render-worker", world.serverLevel()),
            new ThreadPoolExecutor.DiscardPolicy()
        );
    }

    protected static int getThreads(final int threads) {
        return getThreads(threads, 2);
    }

    protected static int getThreads(int threads, final int factor) {
        if (threads == -1) {
            threads = Runtime.getRuntime().availableProcessors() / factor;
        }
        return Math.max(1, threads);
    }

    protected static void sleep(int ms) {
        try {
            Thread.sleep(ms);
        } catch (final InterruptedException ignore) {
        }
    }

    public static final class ChunkSnapshotManager {
        private static final int MAXIMUM_CAPACITY = 2048;

        private final ChunkSnapshotProvider chunkSnapshotProvider;
        private final int maximumActiveRequests;
        private final ConcurrentFIFOLoadingCache<ChunkHashMapKey, CompletableFuture<@Nullable ChunkSnapshot>> cache;
        public final AtomicLong active = new AtomicLong();
        public final AtomicLong done = new AtomicLong();
        private final boolean biomeBlend;
        private final BooleanSupplier running;

        public ChunkSnapshotManager(
            final ChunkSnapshotProvider chunkSnapshotProvider,
            final int maximumActiveRequests,
            final boolean biomeBlend,
            final BooleanSupplier running
        ) {
            this.chunkSnapshotProvider = chunkSnapshotProvider;
            this.maximumActiveRequests = maximumActiveRequests;
            this.cache = new ConcurrentFIFOLoadingCache<>(
                MAXIMUM_CAPACITY,
                (int) (MAXIMUM_CAPACITY * 0.8),
                this::load
            );
            this.biomeBlend = biomeBlend;
            this.running = running;
        }

        private CompletableFuture<ChunkSnapshot> load(final ChunkHashMapKey key) {
            if (!this.maybeWait()) {
                return CompletableFuture.completedFuture(null);
            }
            this.active.incrementAndGet();
            final CompletableFuture<@Nullable ChunkSnapshot> future = this.chunkSnapshotProvider.asyncSnapshot(ChunkPos.getX(key.key), ChunkPos.getZ(key.key));
            future.whenComplete(($, $$) -> this.done.incrementAndGet());
            return future;
        }

        private boolean maybeWait() {
            for (int failures = 1; (this.active.get() - this.done.get()) >= this.maximumActiveRequests; ++failures) {
                if (!this.running.getAsBoolean()) {
                    return false;
                }
                final boolean interrupted = Thread.interrupted();
                Thread.yield();
                LockSupport.parkNanos(TimeUnit.MILLISECONDS.toNanos(Math.min(10, failures)));
                if (interrupted) {
                    Thread.currentThread().interrupt();
                }
            }
            return true;
        }

        // requests neighbors when biomes are mapped
        public CompletableFuture<@Nullable ChunkSnapshot> snapshot(final ChunkPos chunkPos) {
            final CompletableFuture<@Nullable ChunkSnapshot> future = this.snapshotDirect(chunkPos);
            if (!this.biomeBlend) {
                return future;
            }

            final int x = chunkPos.x();
            final int z = chunkPos.z();

            final List<CompletableFuture<@Nullable ChunkSnapshot>> neighborFutures = List.of(
                this.snapshotDirect(new ChunkPos(x - 1, z - 1)),
                this.snapshotDirect(new ChunkPos(x, z - 1)),
                this.snapshotDirect(new ChunkPos(x + 1, z + 1)),
                this.snapshotDirect(new ChunkPos(x - 1, z)),
                this.snapshotDirect(new ChunkPos(x + 1, z)),
                this.snapshotDirect(new ChunkPos(x - 1, z + 1)),
                this.snapshotDirect(new ChunkPos(x, z + 1)),
                this.snapshotDirect(new ChunkPos(x + 1, z - 1))
            );

            return CompletableFuture.allOf(neighborFutures.toArray(CompletableFuture[]::new)).thenCompose($ -> future);
            //return future;
        }

        // only requests the specific chunk
        public CompletableFuture<@Nullable ChunkSnapshot> snapshotDirect(final ChunkPos chunkPos) {
            return this.cache.get(new ChunkHashMapKey(chunkPos));
        }
    }

    private record RenderWorkerExecutor(Executor wrapped, BooleanSupplier running) implements Executor {
        @Override
        public void execute(final Runnable task) {
            this.wrapped.execute(new WorkerTask(task, this.running));
        }

        private record WorkerTask(Runnable wrapped, BooleanSupplier running) implements Runnable {
            @Override
            public void run() {
                if (this.running.getAsBoolean()) {
                    this.wrapped.run();
                }
            }
        }
    }

    private record BiomeCacheKey(Thread thread, int blendRadius) {
    }

    protected enum State {
        RUNNING,
        STOPPED,
        CANCELLED
    }
}
