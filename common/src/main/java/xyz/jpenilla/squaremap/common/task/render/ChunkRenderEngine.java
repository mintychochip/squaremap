package xyz.jpenilla.squaremap.common.task.render;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.StainedGlassBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import org.checkerframework.checker.nullness.qual.Nullable;
import xyz.jpenilla.squaremap.common.data.BiomeColors;
import xyz.jpenilla.squaremap.common.data.Image;
import xyz.jpenilla.squaremap.common.util.Colors;
import xyz.jpenilla.squaremap.common.util.RenderPrimitiveEngine;
import xyz.jpenilla.squaremap.common.util.chunksnapshot.ChunkSnapshot;

/** The one canonical Java chunk-to-pixel state machine shared by production and fixture oracles. */
final class ChunkRenderEngine {
    @FunctionalInterface
    interface PixelSink {
        void setPixel(int worldX, int worldZ, int argb);
    }

    @FunctionalInterface
    interface BiomeModifier {
        int modify(int color, ChunkSnapshot chunk, BlockPos pos, int blendRadius);
    }

    static final class Settings {
        final int maxHeight;
        final boolean iterateUp;
        final boolean glassClear;
        final boolean waterCheckerboard;
        final boolean waterClear;
        final boolean lavaCheckerboard;
        final boolean biomeEnabled;
        final int biomeBlend;

        Settings(final int maxHeight, final boolean iterateUp, final boolean glassClear,
                 final boolean waterCheckerboard, final boolean waterClear,
                 final boolean lavaCheckerboard, final boolean biomeEnabled,
                 final int biomeBlend) {
            this.maxHeight = maxHeight;
            this.iterateUp = iterateUp;
            this.glassClear = glassClear;
            this.waterCheckerboard = waterCheckerboard;
            this.waterClear = waterClear;
            this.lavaCheckerboard = lavaCheckerboard;
            this.biomeEnabled = biomeEnabled;
            if (biomeBlend < 0 || biomeBlend > 15) {
                throw new IllegalArgumentException("biome blend radius out of range: " + biomeBlend);
            }
            this.biomeBlend = biomeBlend;
        }
    }

    record PixelResult(int[] pixels, int[] southEdge) {
        PixelResult {
            if (pixels.length != 256 || southEdge.length != 16) {
                throw new IllegalArgumentException("Expected 256 pixels and 16 edge values");
            }
            pixels = pixels.clone();
            southEdge = southEdge.clone();
        }
        @Override public int[] pixels() { return pixels.clone(); }
        @Override public int[] southEdge() { return southEdge.clone(); }
    }

    interface Adapter {
        Settings settings();
        int mapColor(BlockState state);
        boolean invisibleBlock(Block block);
        boolean iterateUpBaseBlock(Block block);
        @Nullable BiomeModifier biomeModifier();
        boolean running();
        boolean rendersPaused();
        boolean shouldRenderColumn(int blockX, int blockZ);
        void sleep(long millis);
    }
    private final Adapter adapter;

    ChunkRenderEngine(final Adapter adapter) {
        this.adapter = adapter;
    }

    PixelResult renderChunkResult(final @Nullable ChunkSnapshot north, final ChunkSnapshot center,
                                  final @Nullable ChunkSnapshot south) {
        return this.renderChunk((x, z, color) -> {}, north, center, south);
    }

    PixelResult renderChunk(final PixelSink sink, final @Nullable ChunkSnapshot north,
                            final ChunkSnapshot center, final @Nullable ChunkSnapshot south) {
        final int[] lastY = new int[16];
        if (north != null) this.getLastYFromBottomRow(north, lastY);
        final int[] pixels = new int[256];
        final int centerX = center.pos().getMinBlockX();
        final int centerZ = center.pos().getMinBlockZ();
        final PixelSink recordingSink = (x, z, color) -> {
            sink.setPixel(x, z, color);
            final int localX = x - centerX;
            final int localZ = z - centerZ;
            if (localX >= 0 && localX < 16 && localZ >= 0 && localZ < 16) {
                pixels[localX * 16 + localZ] = color;
            }
        };
        this.scanChunk(recordingSink, lastY, center);
        final int[] edge = lastY.clone();
        if (south != null) this.scanTopRow(sink, lastY, south);
        return new PixelResult(pixels, edge);
    }

    void scanChunk(final Image image, final int[] lastY, final ChunkSnapshot chunk) {
        this.scanChunk(image::setPixel, lastY, chunk);
    }

    void scanChunk(final PixelSink sink, final int[] lastY, final ChunkSnapshot chunk) {
        while (this.adapter.rendersPaused() && this.adapter.running()) {
            this.adapter.sleep(500);
        }
        final int blockX = chunk.pos().getMinBlockX();
        final int blockZ = chunk.pos().getMinBlockZ();
        for (int x = 0; x < 16; x++) {
            for (int z = 0; z < 16; z++) {
                if (!this.adapter.running()) return;
                if (this.adapter.shouldRenderColumn(blockX + x, blockZ + z)) {
                    sink.setPixel(blockX + x, blockZ + z, this.scanBlock(chunk, x, z, lastY));
                }
            }
        }
    }

    void scanTopRow(final Image image, final int[] lastY, final ChunkSnapshot chunk) {
        this.scanTopRow(image::setPixel, lastY, chunk);
    }

    void scanTopRow(final PixelSink sink, final int[] lastY, final ChunkSnapshot chunk) {
        final int blockX = chunk.pos().getMinBlockX();
        final int blockZ = chunk.pos().getMinBlockZ();
        for (int x = 0; x < 16; x++) {
            if (!this.adapter.running()) return;
            if (this.adapter.shouldRenderColumn(blockX + x, blockZ)) {
                sink.setPixel(blockX + x, blockZ, this.scanBlock(chunk, x, 0, lastY));
            }
        }
    }

    private int effectiveMaxHeight(final ChunkSnapshot chunk) {
        return this.adapter.settings().maxHeight == -1 ? chunk.getMaxY() + 1 : this.adapter.settings().maxHeight;
    }

    int[] getLastYFromBottomRow(final ChunkSnapshot chunk) {
        final int[] lastY = new int[16];
        this.getLastYFromBottomRow(chunk, lastY);
        return lastY;
    }

    private void getLastYFromBottomRow(final ChunkSnapshot chunk, final int[] lastY) {
        final BlockPos.MutableBlockPos mutablePos = new BlockPos.MutableBlockPos();
        for (int x = 0; x < 16; x++) {
            if (!this.adapter.running()) return;
            final int topY = chunk.getHeight(Heightmap.Types.WORLD_SURFACE, x, 15) + 1;
            mutablePos.set(chunk.pos().getMinBlockX() + x, Math.min(topY, this.effectiveMaxHeight(chunk)), chunk.pos().getMinBlockZ() + 15);
            final BlockState state = this.adapter.settings().iterateUp ? this.iterateUp(chunk, mutablePos) : this.iterateDown(chunk, mutablePos);
            if (this.adapter.settings().glassClear && isGlass(state)) this.handleGlass(chunk, mutablePos);
            lastY[x] = mutablePos.getY();
        }
    }

    private int scanBlock(final ChunkSnapshot chunk, final int imgX, final int imgZ, final int[] lastY) {
        final int blockX = chunk.pos().getMinBlockX() + imgX;
        final int blockZ = chunk.pos().getMinBlockZ() + imgZ;
        final BlockPos.MutableBlockPos mutablePos = new BlockPos.MutableBlockPos();
        final int topY = chunk.getHeight(Heightmap.Types.WORLD_SURFACE, imgX, imgZ) + 1;
        mutablePos.set(blockX, Math.min(topY, this.effectiveMaxHeight(chunk)), blockZ);
        if (topY <= chunk.getMinY()) return Colors.clearMapColor();

        final BlockState state = this.adapter.settings().iterateUp ? this.iterateUp(chunk, mutablePos) : this.iterateDown(chunk, mutablePos);
        if (this.adapter.settings().glassClear && isGlass(state)) {
            final int glassColor = this.adapter.mapColor(state);
            final float glassAlpha = state.getBlock() == Blocks.GLASS ? 0.25F : 0.5F;
            final BlockState underlying = this.handleGlass(chunk, mutablePos);
            final int color = this.getColor(chunk, imgX, imgZ, lastY, underlying, mutablePos);
            return RenderPrimitiveEngine.glass(color, glassColor, glassAlpha);
        }
        return this.getColor(chunk, imgX, imgZ, lastY, state, mutablePos);
    }

    private int getColor(final ChunkSnapshot chunk, final int imgX, final int imgZ, final int[] lastY,
                         final BlockState state, final BlockPos.MutableBlockPos mutablePos) {
        int color = this.adapter.mapColor(state);
        final @Nullable BiomeModifier modifier = this.adapter.biomeModifier();
        if (modifier != null && this.adapter.settings().biomeEnabled) {
            color = modifier.modify(color, chunk, mutablePos, this.adapter.settings().biomeBlend);
        }
        final int odd = RenderPrimitiveEngine.parity(imgX, imgZ);
        final @Nullable DepthResult fluid = findDepthIfFluid(mutablePos, state, chunk);
        if (fluid != null) {
            final RenderPrimitiveEngine.FluidKind kind = fluidKindForRender(color, state.getFluidState());
            return RenderPrimitiveEngine.fluid(fluid.depth, color, kind == RenderPrimitiveEngine.FluidKind.WATER,
                this.adapter.mapColor(fluid.state), this.adapter.settings().waterCheckerboard,
                this.adapter.settings().waterClear,
                kind == RenderPrimitiveEngine.FluidKind.LAVA && this.adapter.settings().lavaCheckerboard, odd);
        }
        final int curY = mutablePos.getY();
        final int previousY = lastY[imgX];
        lastY[imgX] = curY;
        return RenderPrimitiveEngine.terrain(curY, previousY, color, odd);
    }

    private BlockState iterateDown(final ChunkSnapshot chunk, final BlockPos.MutableBlockPos pos) {
        BlockState state;
        if (chunk.dimensionType().hasCeiling()) {
            do { pos.move(Direction.DOWN); state = chunk.getBlockState(pos); }
            while (!state.isAir() && pos.getY() > chunk.getMinY());
        }
        do { pos.move(Direction.DOWN); state = chunk.getBlockState(pos); }
        while (this.clearOrInvisible(state) && pos.getY() > chunk.getMinY());
        return state;
    }

    private BlockState iterateUp(final ChunkSnapshot chunk, final BlockPos.MutableBlockPos pos) {
        BlockState state;
        final int height = pos.getY();
        pos.setY(chunk.getMinY());
        if (chunk.dimensionType().hasCeiling()) {
            do { pos.move(Direction.UP); state = chunk.getBlockState(pos); }
            while (!state.isAir() && pos.getY() < height);
            do { pos.move(Direction.UP); state = chunk.getBlockState(pos); }
            while (!this.adapter.iterateUpBaseBlock(state.getBlock()) && pos.getY() < height);
        }
        do { pos.move(Direction.DOWN); state = chunk.getBlockState(pos); }
        while (this.clearOrInvisible(state) && pos.getY() > chunk.getMinY());
        return state;
    }

    private boolean clearOrInvisible(final BlockState state) {
        return this.adapter.mapColor(state) == Colors.clearMapColor() || this.adapter.invisibleBlock(state.getBlock());
    }

    private static boolean isGlass(final BlockState state) {
        final Block block = state.getBlock();
        return block == Blocks.GLASS || block instanceof StainedGlassBlock;
    }

    private BlockState handleGlass(final ChunkSnapshot chunk, final BlockPos.MutableBlockPos pos) {
        BlockState state = chunk.getBlockState(pos);
        while (isGlass(state)) {
            state = this.iterateDown(chunk, pos);
        }
        return state;
    }

    private record DepthResult(int depth, BlockState state) {}

    private static @Nullable DepthResult findDepthIfFluid(final BlockPos pos, final BlockState state, final ChunkSnapshot chunk) {
        if (pos.getY() > chunk.getMinY() && !state.getFluidState().isEmpty()) {
            BlockState fluidState;
            int fluidDepth = 0;
            int yBelowSurface = pos.getY() - 1;
            final BlockPos.MutableBlockPos mutablePos = new BlockPos.MutableBlockPos();
            mutablePos.set(pos);
            do {
                mutablePos.setY(yBelowSurface--);
                fluidState = chunk.getBlockState(mutablePos);
                ++fluidDepth;
            } while (yBelowSurface > chunk.getMinY() && fluidDepth <= 10 && !fluidState.getFluidState().isEmpty());
            return new DepthResult(fluidDepth, fluidState);
        }
        return null;
    }

    private static RenderPrimitiveEngine.FluidKind fluidKindForRender(final int color, final FluidState fluidState) {
        final Fluid fluid = fluidState.getType();
        return RenderPrimitiveEngine.classifyUnknownFluid(color,
            fluid == Fluids.WATER || fluid == Fluids.FLOWING_WATER,
            fluid == Fluids.LAVA || fluid == Fluids.FLOWING_LAVA);
    }
}
