package xyz.jpenilla.squaremap.common.util.chunksnapshot;

import net.minecraft.util.BitStorage;
import net.minecraft.util.Mth;
import net.minecraft.util.SimpleBitStorage;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.levelgen.Heightmap;

final class HeightmapSnapshot {
    private final BitStorage data;
    private final LevelHeightAccessor heightAccessor;

    HeightmapSnapshot(
        final ChunkAccess chunk,
        final LevelHeightAccessor heightAccessor,
        final Heightmap.Types heightmapType
    ) {
        this.data = new SimpleBitStorage(
            Mth.ceillog2(heightAccessor.getHeight() + 1),
            256,
            chunk.getOrCreateHeightmapUnprimed(heightmapType).getRawData().clone()
        );
        this.heightAccessor = heightAccessor;
    }

    HeightmapSnapshot(final LevelHeightAccessor heightAccessor, final int[] firstAvailableHeights) {
        if (firstAvailableHeights.length != 256) throw new IllegalArgumentException("heightmap must contain 256 values");
        this.data = new SimpleBitStorage(Mth.ceillog2(heightAccessor.getHeight() + 1), 256);
        for (int i = 0; i < firstAvailableHeights.length; i++) {
            final int value = firstAvailableHeights[i];
            if (value < heightAccessor.getMinY() || value > heightAccessor.getMinY() + heightAccessor.getHeight()) throw new IllegalArgumentException("heightmap value out of bounds");
            this.data.set(i, value - heightAccessor.getMinY());
        }
        this.heightAccessor = heightAccessor;
    }

    public int getFirstAvailable(final int x, final int z) {
        return this.getFirstAvailable(getIndex(x, z));
    }

    private int getFirstAvailable(final int index) {
        return this.data.get(index) + this.heightAccessor.getMinY();
    }

    private static int getIndex(final int x, final int z) {
        return x + z * 16;
    }
}
