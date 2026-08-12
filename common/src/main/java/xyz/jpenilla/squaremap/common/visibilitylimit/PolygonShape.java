package xyz.jpenilla.squaremap.common.visibilitylimit;

import java.util.List;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.framework.qual.DefaultQualifier;
import xyz.jpenilla.squaremap.api.MapWorld;
import xyz.jpenilla.squaremap.api.Point;
import xyz.jpenilla.squaremap.common.util.RenderPrimitiveEngine;

@DefaultQualifier(NonNull.class)
final class PolygonShape implements VisibilityShape {
    private final List<Point> points;
    private final RenderPrimitiveEngine.Polygon primitive;


    public PolygonShape(final List<Point> points) {
        this.points = points;
        final int[][] coordinates = new int[points.size()][2];
        for (int i = 0; i < points.size(); i++) {
            final Point point = points.get(i);
            coordinates[i][0] = (int) point.x();
            coordinates[i][1] = (int) point.z();
        }
        this.primitive = RenderPrimitiveEngine.polygon(coordinates);
    }

    List<Point> points() { return this.points; }

    @Override
    public boolean shouldRenderChunk(final MapWorld world, final int chunkX, final int chunkZ) {
        return RenderPrimitiveEngine.polygonChunk(this.primitive, chunkX, chunkZ);
    }

    @Override
    public boolean shouldRenderRegion(final MapWorld world, final int regionX, final int regionZ) {
        return RenderPrimitiveEngine.polygonRegion(this.primitive, regionX, regionZ);
    }

    @Override
    public boolean shouldRenderColumn(final MapWorld world, final int blockX, final int blockZ) {
        return RenderPrimitiveEngine.polygonBlock(this.primitive, blockX, blockZ);
    }

    @Override
    public int countChunksInRegion(final MapWorld world, final int regionX, final int regionZ) {
        return RenderPrimitiveEngine.polygonCount(this.primitive, regionX, regionZ);
    }
}
