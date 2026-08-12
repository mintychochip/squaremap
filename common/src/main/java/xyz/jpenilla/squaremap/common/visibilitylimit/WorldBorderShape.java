package xyz.jpenilla.squaremap.common.visibilitylimit;

import net.minecraft.world.level.border.WorldBorder;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.framework.qual.DefaultQualifier;
import xyz.jpenilla.squaremap.api.MapWorld;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;
import xyz.jpenilla.squaremap.common.util.RenderPrimitiveEngine;

/** A visibility limit that follows the world border. */
@DefaultQualifier(NonNull.class)
public final class WorldBorderShape implements VisibilityShape {
    @Override
    public boolean shouldRenderChunk(final MapWorld world, final int chunkX, final int chunkZ) {
        return RenderPrimitiveEngine.worldBorderChunk(primitive(world), chunkX, chunkZ);
    }

    static boolean shouldRenderChunk(final int centerX, final int centerZ, final int radius, final int chunkX, final int chunkZ) {
        return RenderPrimitiveEngine.worldBorderChunk(RenderPrimitiveEngine.worldBorder(centerX, centerZ, radius), chunkX, chunkZ);
    }

    @Override
    public boolean shouldRenderRegion(final MapWorld world, final int regionX, final int regionZ) {
        return RenderPrimitiveEngine.worldBorderRegion(primitive(world), regionX, regionZ);
    }

    static boolean shouldRenderRegion(final int centerX, final int centerZ, final int radius, final int regionX, final int regionZ) {
        return RenderPrimitiveEngine.worldBorderRegion(RenderPrimitiveEngine.worldBorder(centerX, centerZ, radius), regionX, regionZ);
    }

    @Override
    public boolean shouldRenderColumn(final MapWorld world, final int blockX, final int blockZ) {
        return RenderPrimitiveEngine.worldBorderBlock(primitive(world), blockX, blockZ);
    }

    static boolean shouldRenderColumn(final int centerX, final int centerZ, final int radius, final int blockX, final int blockZ) {
        return RenderPrimitiveEngine.worldBorderBlock(RenderPrimitiveEngine.worldBorder(centerX, centerZ, radius), blockX, blockZ);
    }

    @Override
    public int countChunksInRegion(final MapWorld world, final int regionX, final int regionZ) {
        return RenderPrimitiveEngine.worldBorderCount(primitive(world), regionX, regionZ);
    }

    static int countChunksInRegion(final int centerX, final int centerZ, final int radius, final int regionX, final int regionZ) {
        return RenderPrimitiveEngine.worldBorderCount(RenderPrimitiveEngine.worldBorder(centerX, centerZ, radius), regionX, regionZ);
    }

    private static RenderPrimitiveEngine.WorldBorder primitive(final MapWorld world) {
        final WorldBorder border = ((MapWorldInternal) world).serverLevel().getWorldBorder();
        return RenderPrimitiveEngine.fromRuntime(border.getCenterX(), border.getCenterZ(), border.getSize());
    }
}
