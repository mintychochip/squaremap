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
import xyz.jpenilla.squaremap.bridge.v1.GrassColorModifier;
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
    private final Map<Holder<Biome>, Integer> biomeIds;
    private RegistryDescriptorExporter(final RegistryReplace exported, final Map<BlockState, Integer> blockIds, final Map<Holder<Biome>, Integer> biomeIds) {
        this.exported = exported;
        this.blockIds = Map.copyOf(blockIds);
        this.biomeIds = Map.copyOf(biomeIds);
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
                states.add(new StateInput(state, new BlockDescriptor(registryId, properties, color, transparency, glass, alpha, fluid(state.getFluidState()), state.isAir(), world.advanced().iterateUpBaseBlocks.contains(block), BiomeColors.tintIndex(block))));
            }
        }
        final List<BiomeInput> biomes = new ArrayList<>();
        final LevelBiomeColorData colors = world.levelBiomeColorData();
        for (final Biome biome : Util.biomeRegistry(world.serverLevel())) {
            biomes.add(new BiomeInput(biome, new BiomeDescriptorInput(Util.biomeRegistry(world.serverLevel()).getKey(biome).toString(),
                colors.grassColor(biome), colors.foliageColor(biome), colors.waterColor(biome), 0, wireModifier(biome))));
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
        final Map<Holder<Biome>, Integer> biomeIds = new HashMap<>();
        for (final BiomeInput input : biomes) {
            wire.addBiomes(input.descriptor.toProto(id));
            biomeIds.put(Util.biomeRegistry(world.serverLevel()).wrapAsHolder(input.biome), id++);
        }
        return new RegistryDescriptorExporter(wire.build(), blockIds, biomeIds);
    }
    private static int requiredBiomeColor(final it.unimi.dsi.fastutil.objects.Reference2IntMap<Biome> colors, final Biome biome, final String category) {
        if (!colors.containsKey(biome)) throw new IllegalStateException("missing " + category + " biome color");
        return colors.getInt(biome);
    }

    public record FixtureState(BlockState state, BlockDescriptor descriptor) {
        public FixtureState { Objects.requireNonNull(state); Objects.requireNonNull(descriptor); }
    }

    public record FixtureBiome(Holder<Biome> holder, BiomeDescriptorInput descriptor) {
        public FixtureBiome { Objects.requireNonNull(holder); Objects.requireNonNull(descriptor); }
    }

    public static RegistryDescriptorExporter createFixture(final WorldIdentity world, final long revision,
                                                            final List<FixtureState> states,
                                                            final List<FixtureBiome> biomes) {
        Objects.requireNonNull(world, "world");
        Objects.requireNonNull(states, "states");
        Objects.requireNonNull(biomes, "biomes");
        final List<FixtureState> sortedStates = new ArrayList<>(states);
        sortedStates.sort(Comparator.comparing((FixtureState input) -> BuiltInRegistries.BLOCK.getKey(input.state().getBlock()).toString())
            .thenComparing(input -> input.state().toString()));
        final List<FixtureBiome> sortedBiomes = new ArrayList<>(biomes);
        sortedBiomes.sort(Comparator.comparing(input -> input.holder().unwrapKey()
            .orElseThrow(() -> new IllegalStateException("fixture biome holder has no key")).identifier().toString()));
        final Map<BlockKey, FixtureState> descriptorKeys = new TreeMap<>();
        for (final FixtureState input : sortedStates) {
            final String expectedRegistryId = BuiltInRegistries.BLOCK.getKey(input.state().getBlock()).toString();
            final List<String> expectedProperties = List.of(input.state().toString());
            if (!expectedRegistryId.equals(input.descriptor().registryId()) || !expectedProperties.equals(input.descriptor().properties())
                || input.descriptor().air() != input.state().isAir()) {
                throw new IllegalStateException("fixture descriptor does not match actual block state: " + input.state());
            }
            final BlockKey descriptorKey = new BlockKey(input.descriptor().registryId(), input.descriptor().properties());
            if (descriptorKeys.put(descriptorKey, input) != null) {
                throw new IllegalStateException("duplicate fixture block descriptor key: " + descriptorKey);
            }
        }
        final Map<String, FixtureBiome> biomeKeys = new HashMap<>();
        for (final FixtureBiome input : sortedBiomes) {
            final String expectedRegistryId = input.holder().unwrapKey()
                .orElseThrow(() -> new IllegalStateException("fixture biome holder has no key")).identifier().toString();
            if (!expectedRegistryId.equals(input.descriptor().registryId())) {
                throw new IllegalStateException("fixture descriptor does not match actual biome holder: " + expectedRegistryId);
            }
            if (biomeKeys.put(input.descriptor().registryId(), input) != null) {
                throw new IllegalStateException("duplicate fixture biome descriptor key: " + input.descriptor().registryId());
            }
        }
        final RegistryReplace.Builder wire = RegistryReplace.newBuilder().setRevision(revision).setWorld(world);
        final Map<BlockState, Integer> blockIds = new HashMap<>();
        final Map<Holder<Biome>, Integer> biomeIds = new HashMap<>();
        int id = 1;
        for (final FixtureState input : sortedStates) {
            if (blockIds.put(input.state(), id) != null) throw new IllegalStateException("duplicate fixture block state");
            wire.addBlockStates(input.descriptor().toProto(id++));
        }
        for (final FixtureBiome input : sortedBiomes) {
            if (biomeIds.put(input.holder(), id) != null) throw new IllegalStateException("duplicate fixture biome holder");
            wire.addBiomes(input.descriptor().toProto(id++));
        }
        return new RegistryDescriptorExporter(wire.build(), blockIds, biomeIds);
    }

    public RegistryReplace snapshot() { return this.exported; }
    public int blockId(final BlockState state) {
        final Integer id = this.blockIds.get(state);
        if (id == null) throw new IllegalStateException("undeclared block state: " + state);
        return id;
    }
    public int biomeId(final Holder<Biome> biome) {
        final Integer id = this.biomeIds.get(biome);
        if (id == null) throw new IllegalStateException("undeclared biome holder: " + biome);
        return id;
    }
    public int biomeId(final Biome biome) {
        for (final Map.Entry<Holder<Biome>, Integer> entry : this.biomeIds.entrySet()) {
            if (entry.getKey().value() == biome) return entry.getValue();
        }
        throw new IllegalStateException("undeclared biome value: " + biome);
    }

    public static String blockKey(final String registryId, final String propertyString) {
        Objects.requireNonNull(registryId, "registryId");
        Objects.requireNonNull(propertyString, "propertyString");
        return registryId.length() + ":" + registryId + propertyString.length() + ":" + propertyString;
    }

    public BlockDescriptor blockDescriptor(final BlockState state) {
        final int id = this.blockId(state);
        final BlockStateDescriptor descriptor = this.exported.getBlockStates(id - 1);
        return new BlockDescriptor(BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString(), List.of(state.toString()),
            descriptor.getMapColor(), descriptor.getTransparency(), descriptor.getGlass(), descriptor.getGlassAlphaPercent(),
            descriptor.getFluid(), descriptor.getAir(), descriptor.getIterateUpBase(), descriptor.getTintIndex());
    }
    public BiomeDescriptorInput biomeDescriptor(final Holder<Biome> biome) {
        final int id = this.biomeId(biome);
        final BiomeDescriptor descriptor = this.exported.getBiomes(id - this.exported.getBlockStatesCount() - 1);
        return new BiomeDescriptorInput(biome.unwrapKey().orElseThrow().identifier().toString(), descriptor.getGrassColor(),
            descriptor.getFoliageColor(), descriptor.getWaterColor(), descriptor.getTintIndex(), descriptor.getGrassColorModifier());
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

    private static GrassColorModifier wireModifier(final Biome biome) {
        return switch (biome.getSpecialEffects().grassColorModifier()) {
            case NONE -> GrassColorModifier.GRASS_COLOR_MODIFIER_NONE;
            case DARK_FOREST -> GrassColorModifier.GRASS_COLOR_MODIFIER_DARK_FOREST;
            case SWAMP -> GrassColorModifier.GRASS_COLOR_MODIFIER_SWAMP;
        };
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
                                  boolean glass, int glassAlphaPercent, FluidClass fluid, boolean air, boolean iterateUpBase, int tintIndex) {
        public BlockDescriptor { Objects.requireNonNull(registryId); properties = List.copyOf(properties); Objects.requireNonNull(transparency); Objects.requireNonNull(fluid); }
        public BlockDescriptor(final String registryId, final List<String> properties, final int mapColor, final BlockTransparency transparency,
                               final boolean glass, final int glassAlphaPercent, final FluidClass fluid, final boolean air, final int tintIndex) {
            this(registryId, properties, mapColor, transparency, glass, glassAlphaPercent, fluid, air, false, tintIndex);
        }
        public BlockDescriptor(final String registryId, final String propertyString, final int mapColor, final BlockTransparency transparency, final boolean glass, final FluidClass fluid, final int tintIndex) {
            this(registryId, List.of(propertyString), mapColor, transparency, glass, glass ? (propertyString.contains("stained") ? 50 : 25) : 0, fluid, false, false, tintIndex);
        }
        BlockStateDescriptor toProto(final int id) { return BlockStateDescriptor.newBuilder().setId(id).setMapColor(mapColor).setTransparency(transparency).setGlass(glass).setGlassAlphaPercent(glassAlphaPercent).setFluid(fluid).setAir(air).setIterateUpBase(iterateUpBase).setTintIndex(tintIndex).build(); }
    }
    public record BiomeDescriptorInput(String registryId, int grassColor, int foliageColor, int waterColor, int tintIndex,
                                       GrassColorModifier grassColorModifier) {
        public BiomeDescriptorInput { Objects.requireNonNull(registryId); Objects.requireNonNull(grassColorModifier); }
        BiomeDescriptor toProto(final int id) { return BiomeDescriptor.newBuilder().setId(id).setGrassColor(grassColor).setFoliageColor(foliageColor).setWaterColor(waterColor).setTintIndex(tintIndex).setGrassColorModifier(grassColorModifier).build(); }
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
