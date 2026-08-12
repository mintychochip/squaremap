package xyz.jpenilla.squaremap.common.util.chunksnapshot;

import java.util.EnumMap;
import java.util.Map;
import net.minecraft.core.Holder;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.PalettedContainer;
import net.minecraft.world.level.dimension.DimensionType;
import net.minecraft.world.level.levelgen.Heightmap;

/** No-server construction seam for copied, actual chunk state used by parity fixtures. */
public final class ChunkSnapshotFactory {
    private ChunkSnapshotFactory() {
    }

    public static ChunkSnapshot create(
        final PalettedContainer<BlockState>[] blockStates,
        final PalettedContainer<Holder<Biome>>[] biomeStates,
        final int[] firstAvailableHeights,
        final DimensionType dimensionType,
        final ChunkPos pos,
        final int minY,
        final int height
    ) {
        if (blockStates.length == 0 || blockStates.length != biomeStates.length || height != blockStates.length * 16) {
            throw new IllegalArgumentException("snapshot sections do not match height");
        }
        if (firstAvailableHeights.length != 256) {
            throw new IllegalArgumentException("heightmap must contain 256 values");
        }
        final LevelHeightAccessor heightAccessor = LevelHeightAccessor.create(minY, height);
        final PalettedContainer<BlockState>[] blocks = blockStates.clone();
        final PalettedContainer<Holder<Biome>>[] biomes = biomeStates.clone();
        final boolean[] empty = new boolean[blocks.length];
        for (int i = 0; i < blocks.length; i++) {
            blocks[i] = blocks[i].copy();
            biomes[i] = biomes[i].copy();
            empty[i] = !blocks[i].maybeHas(state -> !state.isAir());
        }
        final Map<Heightmap.Types, HeightmapSnapshot> heightmaps = new EnumMap<>(Heightmap.Types.class);
        heightmaps.put(Heightmap.Types.WORLD_SURFACE, new HeightmapSnapshot(heightAccessor, firstAvailableHeights));
        return new ChunkSnapshotImpl(heightAccessor, blocks, biomes, heightmaps, empty, dimensionType, pos);
    }
}
