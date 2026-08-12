package xyz.jpenilla.squaremap.common.visibilitylimit;

import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.framework.qual.DefaultQualifier;
import xyz.jpenilla.squaremap.api.MapWorld;
import xyz.jpenilla.squaremap.common.util.RenderPrimitiveEngine;

/** Limits map drawing to a circular region. */
@DefaultQualifier(NonNull.class)
final class CircleShape implements VisibilityShape {
    private final int centerX;
    private final int centerZ;
    private final int radius;
    private final RenderPrimitiveEngine.Circle primitive;

    public CircleShape(final int centerX, final int centerZ, final int radius) {
        if (radius < 1) {
            throw new IllegalArgumentException("Radius must be positive, but was " + radius);
        }
        this.centerX = centerX;
        this.centerZ = centerZ;
        this.radius = radius;
        this.primitive = RenderPrimitiveEngine.circle(centerX, centerZ, radius);
    }

    int centerX() { return this.centerX; }
    int centerZ() { return this.centerZ; }
    int radius() { return this.radius; }

    @Override
    public boolean shouldRenderChunk(final MapWorld world, final int chunkX, final int chunkZ) {
        return RenderPrimitiveEngine.circleChunk(this.primitive, chunkX, chunkZ);
    }

    @Override
    public boolean shouldRenderRegion(final MapWorld world, final int regionX, final int regionZ) {
        return RenderPrimitiveEngine.circleRegion(this.primitive, regionX, regionZ);
    }

    @Override
    public boolean shouldRenderColumn(final MapWorld world, final int blockX, final int blockZ) {
        return RenderPrimitiveEngine.circleBlock(this.primitive, blockX, blockZ);
    }

    @Override
    public int countChunksInRegion(final MapWorld world, final int regionX, final int regionZ) {
        return RenderPrimitiveEngine.circleCount(this.primitive, regionX, regionZ);
    }
}
