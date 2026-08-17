package xyz.jpenilla.squaremap.common.data;

import java.util.Set;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.MapColor;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.checker.nullness.qual.Nullable;
import org.checkerframework.framework.qual.DefaultQualifier;
import xyz.jpenilla.squaremap.common.util.ColorBlender;
import xyz.jpenilla.squaremap.common.util.Colors;
import xyz.jpenilla.squaremap.common.util.chunksnapshot.ChunkSnapshot;

@DefaultQualifier(NonNull.class)
public final class BiomeColors {
    private static final int BLOCKPOS_BIOME_CACHE_SIZE = 4096;

    private static final Set<Block> GRASS_COLOR_BLOCKS = Set.of(
        Blocks.GRASS_BLOCK,
        Blocks.SHORT_GRASS,
        Blocks.TALL_GRASS,
        Blocks.FERN,
        Blocks.LARGE_FERN,
        Blocks.POTTED_FERN,
        Blocks.SUGAR_CANE
    );

    private static final Set<Block> FOLIAGE_COLOR_BLOCKS = Set.of(
        Blocks.VINE,
        Blocks.OAK_LEAVES,
        Blocks.JUNGLE_LEAVES,
        Blocks.ACACIA_LEAVES,
        Blocks.DARK_OAK_LEAVES,
        Blocks.MANGROVE_LEAVES
    );
    /** Returns the wire tint category used by the renderer. */
    public static int tintIndex(final Block block) {
        if (GRASS_COLOR_BLOCKS.contains(block)) return 1;
        if (FOLIAGE_COLOR_BLOCKS.contains(block)) return 2;
        return block.defaultMapColor() == net.minecraft.world.level.material.MapColor.WATER ? 3 : 0;
    }

    private final ColorBlender colorBlender = new ColorBlender();
    private final BlockPos.MutableBlockPos mutablePos = new BlockPos.MutableBlockPos();
    private final LevelBiomeColorData colorData;
    private final BiomeLookup biomeLookup;
    private final int blendRadius;

    @FunctionalInterface
    public interface BiomeLookup {
        Biome biome(BlockPos pos);
    }

    public static BiomeColors fixture(
        final BiomeLookup biomeLookup,
        final LevelBiomeColorData colorData,
        final int blendRadius
    ) {
        return new BiomeColors(biomeLookup, colorData, blendRadius);
    }

    private BiomeColors(final BiomeLookup biomeLookup, final LevelBiomeColorData colorData, final int blendRadius) {
        if (blendRadius < 0 || blendRadius > 15) throw new IllegalArgumentException("biome blend radius out of range: " + blendRadius);
        this.biomeLookup = biomeLookup;
        this.colorData = colorData;
        this.blendRadius = blendRadius;
    }

    public int modifyColorFromBiome(int color, final ChunkSnapshot chunk, final BlockPos pos) {
        final BlockState data = chunk.getBlockState(pos);
        final Block block = data.getBlock();

        if (GRASS_COLOR_BLOCKS.contains(block)) {
            color = this.grass(pos);
        } else if (FOLIAGE_COLOR_BLOCKS.contains(block)) {
            color = this.foliage(pos);
        } else if (block.defaultMapColor() == MapColor.WATER) {
            int modColor = this.water(pos);
            color = Colors.mix(color, modColor, 0.8F);
        }

        return color;
    }

    private int grass(final BlockPos pos) {
        if (this.blendRadius > 0) {
            return this.sampleNeighbors(pos, this.blendRadius, this::grassColorSampler);
        }
        return this.grassColorSampler(this.biome(pos), pos);
    }

    private int grassColorSampler(final Biome biome, final BlockPos pos) {
        return biome.getSpecialEffects().grassColorModifier().modifyColor(pos.getX(), pos.getZ(), this.colorData.grassColor(biome));
    }
    private int foliage(final BlockPos pos) {
        if (this.blendRadius > 0) {
            return this.sampleNeighbors(pos, this.blendRadius, (biome, b) -> this.colorData.foliageColor(biome));
        }
        return this.colorData.foliageColor(this.biome(pos));
    }
    private int water(final BlockPos pos) {
        if (this.blendRadius > 0) {
            return this.sampleNeighbors(pos, this.blendRadius, (biome, b) -> this.colorData.waterColor(biome));
        }
        return this.colorData.waterColor(this.biome(pos));
    }
    private static int requiredColor(final it.unimi.dsi.fastutil.objects.Reference2IntMap<Biome> colors, final Biome biome, final String category) {
        if (!colors.containsKey(biome)) throw new IllegalStateException("missing " + category + " biome color");
        return colors.getInt(biome);
    }

    @FunctionalInterface
    interface ColorSampler {
        int sample(Biome biome, BlockPos pos);
    }

    private int sampleNeighbors(final BlockPos pos, final int radius, final ColorSampler colorSampler) {
        this.colorBlender.reset();

        // Sampling in the y direction as well would improve output, however would complicate caching (low priority, PRs accepted)
        for (int x = pos.getX() - radius; x < pos.getX() + radius; x++) {
            for (int z = pos.getZ() - radius; z < pos.getZ() + radius; z++) {
                this.mutablePos.set(x, pos.getY(), z);

                this.colorBlender.addColor(colorSampler.sample(this.biome(this.mutablePos), this.mutablePos));
            }
        }

        return this.colorBlender.result();
    }

    private Biome biome(final BlockPos pos) {
        return this.biomeLookup.biome(pos);
    }
}
