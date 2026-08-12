package xyz.jpenilla.squaremap.common.bridge.snapshot;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.TreeMap;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.StainedGlassBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.bridge.v1.BiomeDescriptor;
import xyz.jpenilla.squaremap.bridge.v1.BlockStateDescriptor;
import xyz.jpenilla.squaremap.bridge.v1.BlockTransparency;
import xyz.jpenilla.squaremap.bridge.v1.FluidClass;
import xyz.jpenilla.squaremap.bridge.v1.RegistryReplace;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;
import xyz.jpenilla.squaremap.common.data.BiomeColors;
import xyz.jpenilla.squaremap.common.data.LevelBiomeColorData;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;
import xyz.jpenilla.squaremap.common.util.Colors;
import xyz.jpenilla.squaremap.common.util.Util;

/** Deterministic session-local render descriptor exporter. */
public final class RegistryDescriptorExporter {
    private final RegistryReplace exported;
    private final Map<BlockState, Integer> blockIds;
    private final Map<Biome, Integer> biomeIds;

    public RegistryDescriptorExporter(final long revision, final List<BlockDescriptor> blocks, final List<BiomeDescriptorInput> biomes) {
        this.exported = export(revision, blocks, biomes);
        this.blockIds = Map.of(); this.biomeIds = Map.of();
    }

    private RegistryDescriptorExporter(final RegistryReplace exported, final Map<BlockState, Integer> blockIds, final Map<Biome, Integer> biomeIds) {
        this.exported = exported; this.blockIds = Map.copyOf(blockIds); this.biomeIds = Map.copyOf(biomeIds);
    }

    /** Builds descriptors from the actual per-world Minecraft registries and color providers. */
    public static RegistryDescriptorExporter create(final MapWorldInternal world, final long revision, final long epoch) {
        Objects.requireNonNull(world, "world");
        final List<StateInput> states = new ArrayList<>();
        for (final Block block : BuiltInRegistries.BLOCK) {
            final String registryId = BuiltInRegistries.BLOCK.getKey(block).toString();
            for (final BlockState state : block.getStateDefinition().getPossibleStates()) {
                final List<String> properties = List.of(state.toString());
                final int color = world.getMapColor(state);
                final boolean glass = block == Blocks.GLASS || block instanceof StainedGlassBlock;
                final int alpha = glass ? (block == Blocks.GLASS ? 25 : 50) : 0;
                final BlockTransparency transparency = world.advanced().invisibleBlocks.contains(block) ? BlockTransparency.BLOCK_TRANSPARENCY_INVISIBLE
                    : (glass || color == Colors.clearMapColor() ? BlockTransparency.BLOCK_TRANSPARENCY_TRANSLUCENT : BlockTransparency.BLOCK_TRANSPARENCY_OPAQUE);
                states.add(new StateInput(state, new BlockDescriptor(registryId, properties, color, transparency, glass, alpha, fluid(state.getFluidState()), BiomeColors.tintIndex(block))));
            }
        }
        final List<BiomeInput> biomes = new ArrayList<>();
        final LevelBiomeColorData colors = world.levelBiomeColorData();
        for (final Biome biome : Util.biomeRegistry(world.serverLevel())) {
            biomes.add(new BiomeInput(biome, new BiomeDescriptorInput(Util.biomeRegistry(world.serverLevel()).getKey(biome).toString(),
                colors.grassColors().getInt(biome), colors.foliageColors().getInt(biome), colors.waterColors().getInt(biome), 0)));
        }
        states.sort(Comparator.comparing((StateInput input) -> new BlockKey(input.descriptor.registryId(), input.descriptor.properties())));
        biomes.sort(Comparator.comparing(input -> input.descriptor.registryId()));
        final RegistryReplace.Builder wire = RegistryReplace.newBuilder().setRevision(revision).setWorld(WorldIdentity.newBuilder()
            .setNamespace(world.identifier().namespace()).setValue(world.identifier().value()).setEpoch(epoch));
        final Map<BlockState, Integer> blockIds = new HashMap<>();
        int id = 1;
        for (final StateInput input : states) {
            wire.addBlockStates(input.descriptor.toProto(id)); blockIds.put(input.state, id++);
        }
        final Map<Biome, Integer> biomeIds = new HashMap<>();
        for (final BiomeInput input : biomes) {
            wire.addBiomes(input.descriptor.toProto(id)); biomeIds.put(input.biome, id++);
        }
        return new RegistryDescriptorExporter(wire.build(), blockIds, biomeIds);
    }

    public RegistryReplace snapshot() { return this.exported; }
    public int blockId(final BlockState state) { return this.blockIds.getOrDefault(state, 0); }
    public int biomeId(final Biome biome) { return this.biomeIds.getOrDefault(biome, 0); }

    public static String blockKey(final String registryId, final String propertyString) {
        Objects.requireNonNull(registryId, "registryId");
        Objects.requireNonNull(propertyString, "propertyString");
        return registryId.length() + ":" + registryId + propertyString.length() + ":" + propertyString;
    }

    public static RegistryReplace export(final long revision, final List<BlockDescriptor> blocks, final List<BiomeDescriptorInput> biomes) {
        final TreeMap<BlockKey, BlockDescriptor> sortedBlocks = new TreeMap<>();
        for (final BlockDescriptor descriptor : blocks) if (sortedBlocks.put(new BlockKey(descriptor.registryId(), descriptor.properties()), descriptor) != null) throw new IllegalStateException("duplicate block descriptor key");
        final TreeMap<String, BiomeDescriptorInput> sortedBiomes = new TreeMap<>();
        for (final BiomeDescriptorInput descriptor : biomes) if (sortedBiomes.put(descriptor.registryId(), descriptor) != null) throw new IllegalStateException("duplicate biome descriptor");
        final RegistryReplace.Builder result = RegistryReplace.newBuilder().setRevision(revision);
        int id = 1;
        for (final BlockDescriptor descriptor : sortedBlocks.values()) result.addBlockStates(descriptor.toProto(id++));
        for (final BiomeDescriptorInput descriptor : sortedBiomes.values()) result.addBiomes(descriptor.toProto(id++));
        return result.build();
    }

    private static FluidClass fluid(final FluidState fluid) {
        if (fluid.isEmpty()) return FluidClass.FLUID_CLASS_NONE;
        if (fluid.getType() == Fluids.WATER || fluid.getType() == Fluids.FLOWING_WATER) return FluidClass.FLUID_CLASS_WATER;
        if (fluid.getType() == Fluids.LAVA || fluid.getType() == Fluids.FLOWING_LAVA) return FluidClass.FLUID_CLASS_LAVA;
        return FluidClass.FLUID_CLASS_OTHER;
    }

    private record BlockKey(String registryId, List<String> properties) implements Comparable<BlockKey> {
        @Override public int compareTo(final BlockKey other) {
            int value = this.registryId.compareTo(other.registryId); if (value != 0) return value;
            int size = Math.min(this.properties.size(), other.properties.size());
            for (int i = 0; i < size; i++) { value = this.properties.get(i).compareTo(other.properties.get(i)); if (value != 0) return value; }
            return Integer.compare(this.properties.size(), other.properties.size());
        }
    }
    private record StateInput(BlockState state, BlockDescriptor descriptor) {}
    private record BiomeInput(Biome biome, BiomeDescriptorInput descriptor) {}

    public record BlockDescriptor(String registryId, List<String> properties, int mapColor, BlockTransparency transparency,
                                  boolean glass, int glassAlphaPercent, FluidClass fluid, int tintIndex) {
        public BlockDescriptor { Objects.requireNonNull(registryId); properties = List.copyOf(properties); Objects.requireNonNull(transparency); Objects.requireNonNull(fluid); }
        public BlockDescriptor(final String registryId, final String propertyString, final int mapColor, final BlockTransparency transparency, final boolean glass, final FluidClass fluid, final int tintIndex) {
            this(registryId, List.of(propertyString), mapColor, transparency, glass, glass ? (propertyString.contains("stained") ? 50 : 25) : 0, fluid, tintIndex);
        }
        BlockStateDescriptor toProto(final int id) { return BlockStateDescriptor.newBuilder().setId(id).setMapColor(mapColor).setTransparency(transparency).setGlass(glass).setGlassAlphaPercent(glassAlphaPercent).setFluid(fluid).setTintIndex(tintIndex).build(); }
    }
    public record BiomeDescriptorInput(String registryId, int grassColor, int foliageColor, int waterColor, int tintIndex) {
        public BiomeDescriptorInput { Objects.requireNonNull(registryId); }
        BiomeDescriptor toProto(final int id) { return BiomeDescriptor.newBuilder().setId(id).setGrassColor(grassColor).setFoliageColor(foliageColor).setWaterColor(waterColor).setTintIndex(tintIndex).build(); }
    }
    public record RegistrySnapshot(long revision, List<BlockStateDescriptor> blocks, List<BiomeDescriptor> biomes, WorldIdentity world) {
        public RegistrySnapshot { blocks = List.copyOf(blocks); biomes = List.copyOf(biomes); }
        public static RegistrySnapshot of(final RegistryReplace replace) { return new RegistrySnapshot(replace.getRevision(), replace.getBlockStatesList(), replace.getBiomesList(), replace.hasWorld() ? replace.getWorld() : null); }
        public RegistryReplace toRegistryReplace() {
            final RegistryReplace.Builder builder = RegistryReplace.newBuilder().setRevision(revision).addAllBlockStates(blocks).addAllBiomes(biomes);
            if (world != null) builder.setWorld(world);
            return builder.build();
        }
        public Map<String,Integer> sortedBlockIds() { final TreeMap<String,Integer> result = new TreeMap<>(); for (BlockStateDescriptor d : blocks) result.put(Integer.toString(d.getId()), d.getId()); return result; }
    }
}
