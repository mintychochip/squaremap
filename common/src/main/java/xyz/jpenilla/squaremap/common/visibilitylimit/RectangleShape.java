package xyz.jpenilla.squaremap.common.visibilitylimit;

import net.minecraft.core.BlockPos;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.framework.qual.DefaultQualifier;
import xyz.jpenilla.squaremap.api.MapWorld;
import xyz.jpenilla.squaremap.common.util.RenderPrimitiveEngine;

/** Limits map drawing to a rectangular region. */
@DefaultQualifier(NonNull.class)
final class RectangleShape implements VisibilityShape {
    private final RenderPrimitiveEngine.Rectangle primitive;


    RectangleShape(final BlockPos min, final BlockPos max) throws IllegalArgumentException {
        this.primitive = RenderPrimitiveEngine.rectangle(min.getX(), min.getZ(), max.getX(), max.getZ());
    }

    int minBlockX() { return this.primitive.minX(); }
    int maxBlockX() { return this.primitive.maxX(); }
    int minBlockZ() { return this.primitive.minZ(); }
    int maxBlockZ() { return this.primitive.maxZ(); }

    @Override
    public boolean shouldRenderChunk(final MapWorld world, final int chunkX, final int chunkZ) {
        return RenderPrimitiveEngine.rectangleChunk(this.primitive, chunkX, chunkZ);
    }

    @Override
    public boolean shouldRenderRegion(final MapWorld world, final int regionX, final int regionZ) {
        return RenderPrimitiveEngine.rectangleRegion(this.primitive, regionX, regionZ);
    }

    @Override
    public boolean shouldRenderColumn(final MapWorld world, final int blockX, final int blockZ) {
        return RenderPrimitiveEngine.rectangleBlock(this.primitive, blockX, blockZ);
    }

    @Override
    public int countChunksInRegion(final MapWorld world, final int regionX, final int regionZ) {
        return RenderPrimitiveEngine.rectangleCount(this.primitive, regionX, regionZ);
    }
}
