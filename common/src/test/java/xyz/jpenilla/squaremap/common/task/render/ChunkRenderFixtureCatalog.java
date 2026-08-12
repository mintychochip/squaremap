package xyz.jpenilla.squaremap.common.task.render;

import java.nio.file.Path;
import it.unimi.dsi.fastutil.objects.Reference2IntOpenHashMap;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.IdMapper;
import net.minecraft.core.QuartPos;
import net.minecraft.core.registries.Registries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.Biomes;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.biome.BiomeManager;
import net.minecraft.world.level.chunk.PalettedContainer;
import net.minecraft.world.level.chunk.Strategy;
import net.minecraft.world.level.dimension.BuiltinDimensionTypes;
import net.minecraft.world.level.dimension.DimensionType;
import xyz.jpenilla.squaremap.bridge.v1.BlockTransparency;
import xyz.jpenilla.squaremap.bridge.v1.FluidClass;
import xyz.jpenilla.squaremap.bridge.v1.RegistryReplace;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;
import xyz.jpenilla.squaremap.common.bridge.snapshot.ChunkSnapshotEncoder;
import xyz.jpenilla.squaremap.common.bridge.snapshot.RegistryDescriptorExporter;
import xyz.jpenilla.squaremap.common.data.BiomeColors;
import xyz.jpenilla.squaremap.common.data.LevelBiomeColorData;
import xyz.jpenilla.squaremap.common.util.Colors;
import xyz.jpenilla.squaremap.common.util.chunksnapshot.ChunkSnapshot;
import xyz.jpenilla.squaremap.common.util.chunksnapshot.ChunkSnapshotFactory;

/** Immutable actual-state catalog shared by Java rendering and wire projections. */
final class ChunkRenderFixtureCatalog {
    static final long BIOME_ZOOM_SEED = 0x5EEDL;
    private static final int REVISION = 11;
    private static final List<String> IDS = List.of("flat-solid", "empty-chunk", "north-height-discontinuity", "south-height-discontinuity", "missing-north", "missing-south", "iterate-down", "iterate-up", "ceiling-iterate-down", "ceiling-iterate-up", "max-height-clipped", "water-depth-cap", "water-clear", "water-checkerboard", "lava-depth-checkerboard", "clear-glass", "stained-glass", "glass-disabled", "invisible-block", "biome-off", "biome-grass-radius-0", "biome-foliage-radius-0", "biome-water-radius-0", "biome-blend-radius-3", "biome-blend-cross-boundary", "negative-min-y");
    private final RegistryDescriptorExporter descriptors;
    private final WorldIdentity registryWorld;
    private final LevelBiomeColorData colorData;
    private final Map<String, Integer> biomeRegistryIds;
    private final Map<String, CaseModel> cases;
    private ChunkRenderFixtureCatalog(final RegistryDescriptorExporter descriptors, final WorldIdentity world,
                                      final LevelBiomeColorData colorData, final Map<String, Integer> biomeRegistryIds,
                                      final Map<String, CaseModel> cases) {
        this.descriptors = descriptors;
        this.registryWorld = world;
        this.colorData = colorData;
        this.biomeRegistryIds = Collections.unmodifiableMap(new LinkedHashMap<>(biomeRegistryIds));
        this.cases = Collections.unmodifiableMap(new LinkedHashMap<>(cases));
    }
    static { net.minecraft.SharedConstants.tryDetectVersion(); net.minecraft.server.Bootstrap.bootStrap(); }
    static ChunkRenderFixtureCatalog create() {
        final HolderLookup.Provider lookup = VanillaRegistries.createLookup();
        final HolderLookup.RegistryLookup<Biome> biomes = lookup.lookupOrThrow(Registries.BIOME);
        final Holder<Biome> plains = biomes.getOrThrow(Biomes.PLAINS);
        final Holder<Biome> forest = biomes.getOrThrow(Biomes.FOREST);
        final Holder<Biome> swamp = biomes.getOrThrow(Biomes.SWAMP);
        final Holder<Biome> desert = biomes.getOrThrow(Biomes.DESERT);
        final LevelBiomeColorData colors = LevelBiomeColorData.create(
            () -> biomes.listElements().map(Holder::value).iterator(),
            new Reference2IntOpenHashMap<>(), new Reference2IntOpenHashMap<>(), new Reference2IntOpenHashMap<>(),
            LevelBiomeColorData.readImages(Path.of(System.getProperty("squaremap.task11.root"), "web", "public", "images"))
        );
        final WorldIdentity world = WorldIdentity.newBuilder().setNamespace("minecraft").setValue("overworld").setEpoch(1).build();
        final Block stainedBlock = net.minecraft.core.registries.BuiltInRegistries.BLOCK.get(net.minecraft.resources.Identifier.parse("minecraft:red_stained_glass")).orElseThrow().value();
        final List<RegistryDescriptorExporter.FixtureState> states = List.of(
            state(Blocks.AIR), state(Blocks.STONE), state(Blocks.GRASS_BLOCK), state(Blocks.WATER), state(Blocks.LAVA),
            state(Blocks.GLASS, Colors.parseHex("#FFFFFF")), state(stainedBlock), state(Blocks.SHORT_GRASS),
            state(Blocks.OAK_LEAVES), state(Blocks.VINE)
        );
        final RegistryDescriptorExporter descriptors = RegistryDescriptorExporter.createFixture(
            world, REVISION, states, List.of(biome(plains, colors), biome(forest, colors), biome(swamp, colors), biome(desert, colors))
        );
        final Map<String, Integer> biomeRegistryIds = new LinkedHashMap<>();
        biomeRegistryIds.put("minecraft:desert", descriptors.biomeId(desert));
        biomeRegistryIds.put("minecraft:forest", descriptors.biomeId(forest));
        biomeRegistryIds.put("minecraft:plains", descriptors.biomeId(plains));
        biomeRegistryIds.put("minecraft:swamp", descriptors.biomeId(swamp));
        final DimensionType overworld = lookup.getOrThrow(BuiltinDimensionTypes.OVERWORLD).value(), nether = lookup.getOrThrow(BuiltinDimensionTypes.NETHER).value();
        final Map<String, CaseModel> models = new LinkedHashMap<>();
        for (final String id : IDS) {
            final boolean ceiling = id.startsWith("ceiling-");
            final DimensionType dimension = ceiling ? nether : overworld;
            final Holder<Biome> centerBiome = id.contains("foliage") ? forest : id.contains("water") ? swamp : plains;
            final String centerTemplate = id.equals("north-height-discontinuity") || id.equals("south-height-discontinuity") ? "F32" : id.equals("iterate-up") ? "F" : template(id);
            final ChunkSnapshot center = snapshot(centerTemplate, centerBiome, new ChunkPos(0, 0), dimension, desert, swamp);
            final Map<ChunkPos, ChunkSnapshot> neighbors = new LinkedHashMap<>();
            final boolean northRequired = switch (id) {
                case "north-height-discontinuity", "missing-south", "water-depth-cap", "water-clear", "water-checkerboard", "lava-depth-checkerboard", "clear-glass", "stained-glass", "glass-disabled" -> true;
                default -> false;
            };
            final boolean southRequired = id.equals("south-height-discontinuity") || id.equals("missing-north");
            if (northRequired) neighbors.put(new ChunkPos(0, -1), snapshot(id.equals("north-height-discontinuity") ? "D24N" : "F", plains, new ChunkPos(0, -1), dimension, desert, swamp));
            if (southRequired) neighbors.put(new ChunkPos(0, 1), snapshot(id.equals("south-height-discontinuity") ? "D24S" : "F", desert, new ChunkPos(0, 1), dimension, desert, swamp));
            final boolean biomeEnabled = id.startsWith("biome-") && !id.equals("biome-off");
            final int blendRadius = id.contains("radius-3") || id.contains("cross-boundary") ? 3 : 0;
            final Map<ChunkPos, ChunkSnapshot> sources = biomeEnabled ? sourceSnapshots(id, centerBiome, forest, swamp, desert, center, dimension, blendRadius, desert, swamp) : Map.of();
            final ChunkRenderEngine.Settings settings = new ChunkRenderEngine.Settings(id.equals("max-height-clipped") ? 16 : -1, id.equals("iterate-up") || id.equals("ceiling-iterate-up"), !id.equals("glass-disabled"), id.equals("water-checkerboard"), id.equals("water-clear"), id.equals("lava-depth-checkerboard"), biomeEnabled, blendRadius);
            final int invisibleId = id.equals("invisible-block") ? descriptors.blockId(Blocks.STONE.defaultBlockState()) : 0;
            final int iterateUpBaseId = id.equals("ceiling-iterate-up") ? descriptors.blockId(Blocks.GRASS_BLOCK.defaultBlockState()) : 0;
            models.put(id, new CaseModel(id, center, Collections.unmodifiableMap(neighbors), settings, sources, ChunkSnapshotEncoder.encodePortable(center, descriptors, world, REVISION), descriptors.snapshot(), center.getHeight(), ceiling, invisibleId, iterateUpBaseId));
        }
        final ChunkRenderFixtureCatalog catalog = new ChunkRenderFixtureCatalog(descriptors, world, colors, biomeRegistryIds, models); catalog.assertEveryCellHasDescriptor(); return catalog;
    }
    private static String template(final String id) { return switch (id) { case "empty-chunk" -> "A"; case "north-height-discontinuity" -> "D24N"; case "south-height-discontinuity" -> "D24S"; case "iterate-down" -> "Bgrass"; case "ceiling-iterate-down", "ceiling-iterate-up" -> "C32"; case "water-depth-cap" -> "W11"; case "water-clear", "water-checkerboard" -> "W2"; case "lava-depth-checkerboard" -> "L2"; case "clear-glass", "glass-disabled" -> "G"; case "stained-glass" -> "SG"; case "invisible-block" -> "I"; case "biome-grass-radius-0" -> "Bgrass"; case "biome-foliage-radius-0" -> "Bfoliage"; case "biome-water-radius-0" -> "Bwater"; case "negative-min-y" -> "NEG"; case "max-height-clipped" -> "D24"; default -> "F"; }; }
    private static RegistryDescriptorExporter.FixtureState state(final Block block) { return state(block, Colors.rgb(block.defaultBlockState().getMapColor(EmptyBlockGetter.INSTANCE, net.minecraft.core.BlockPos.ZERO))); }
    private static FluidClass fluid(final BlockState state) { if (state.getFluidState().isEmpty()) return FluidClass.FLUID_CLASS_NONE; return state.getFluidState().getType() == net.minecraft.world.level.material.Fluids.LAVA ? FluidClass.FLUID_CLASS_LAVA : FluidClass.FLUID_CLASS_WATER; }
    private static RegistryDescriptorExporter.FixtureState state(final Block block, final int color) { final BlockState state = block.defaultBlockState(); final boolean glass = block == Blocks.GLASS || block instanceof net.minecraft.world.level.block.StainedGlassBlock; final int alpha = glass ? (block == Blocks.GLASS ? 25 : 50) : 0; final BlockTransparency transparency = glass || color == Colors.clearMapColor() ? BlockTransparency.BLOCK_TRANSPARENCY_TRANSLUCENT : state.isAir() ? BlockTransparency.BLOCK_TRANSPARENCY_INVISIBLE : BlockTransparency.BLOCK_TRANSPARENCY_OPAQUE; return new RegistryDescriptorExporter.FixtureState(state, new RegistryDescriptorExporter.BlockDescriptor(net.minecraft.core.registries.BuiltInRegistries.BLOCK.getKey(block).toString(), List.of(state.toString()), color, transparency, glass, alpha, fluid(state), state.isAir(), xyz.jpenilla.squaremap.common.data.BiomeColors.tintIndex(block))); }
    private static RegistryDescriptorExporter.FixtureBiome biome(final Holder<Biome> holder, final LevelBiomeColorData colors) {
        return new RegistryDescriptorExporter.FixtureBiome(holder, new RegistryDescriptorExporter.BiomeDescriptorInput(holder.unwrapKey().orElseThrow().identifier().toString(), colors.grassColor(holder.value()), colors.foliageColor(holder.value()), colors.waterColor(holder.value()), 0));
    }
    private static ChunkSnapshot snapshot(final String name, final Holder<Biome> biome, final ChunkPos pos, final DimensionType dimension, final Holder<Biome> negativeDesert, final Holder<Biome> negativeSwamp) {
        final int minY = name.equals("NEG") ? -32 : 0;
        final int height = name.equals("NEG") || name.equals("F32") || name.equals("C32") || name.startsWith("D24") ? 32 : 16;
        final Strategy<BlockState> bs = Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY);
        final IdMapper<Holder<Biome>> ids = new IdMapper<>();
        VanillaRegistries.createLookup().lookupOrThrow(Registries.BIOME).listElements().forEach(ids::add);
        final Strategy<Holder<Biome>> bio = Strategy.createForBiomes(ids);
        @SuppressWarnings("unchecked") final PalettedContainer<BlockState>[] blocks = (PalettedContainer<BlockState>[]) new PalettedContainer<?>[height / 16];
        @SuppressWarnings("unchecked") final PalettedContainer<Holder<Biome>>[] biomes = (PalettedContainer<Holder<Biome>>[]) new PalettedContainer<?>[height / 16];
        final BlockState air = Blocks.AIR.defaultBlockState();
        final BlockState stone = Blocks.STONE.defaultBlockState();
        final BlockState grass = Blocks.GRASS_BLOCK.defaultBlockState();
        final BlockState water = Blocks.WATER.defaultBlockState();
        final BlockState lava = Blocks.LAVA.defaultBlockState();
        final BlockState glass = Blocks.GLASS.defaultBlockState();
        final Block stainedBlock = net.minecraft.core.registries.BuiltInRegistries.BLOCK.get(net.minecraft.resources.Identifier.parse("minecraft:red_stained_glass")).orElseThrow().value();
        final BlockState stainedGlass = stainedBlock.defaultBlockState();
        final BlockState leaves = Blocks.OAK_LEAVES.defaultBlockState();
        final BlockState shortGrass = Blocks.SHORT_GRASS.defaultBlockState();
        for (int s = 0; s < blocks.length; s++) {
            blocks[s] = new PalettedContainer<>(air, bs);
            final Holder<Biome> sectionBiome = name.equals("NEG") ? (s == 0 ? negativeDesert : negativeSwamp) : biome;
            biomes[s] = new PalettedContainer<>(sectionBiome, bio);
        }
        final int[] heights = new int[256];
        for (int x = 0; x < 16; x++) for (int z = 0; z < 16; z++) {
            final int firstAvailable = switch (name) {
                case "A" -> minY;
                case "W11", "W2", "L2" -> minY + 16;
                case "D24N" -> z == 15 ? minY + 32 : minY + 24;
                case "D24S" -> z == 0 ? minY + 32 : minY + 24;
                case "D24" -> minY + 24;
                case "NEG" -> minY + 17;
                default -> minY + (name.equals("C32") ? 10 : 16);
            };
            heights[x + z * 16] = firstAvailable;
            for (int y = minY; y < minY + height; y++) {
                final int localY = y - minY;
                final BlockState cell = switch (name) {
                    case "A" -> air;
                    case "W11" -> localY < 5 ? stone : localY < 16 ? water : air;
                    case "W2" -> localY < 14 ? stone : localY < 16 ? water : air;
                    case "L2" -> localY < 14 ? stone : localY < 16 ? lava : air;
                    case "G" -> localY < 15 ? stone : localY == 15 ? glass : air;
                    case "SG" -> localY < 15 ? stone : localY == 15 ? stainedGlass : air;
                    case "I" -> localY < 15 ? grass : localY == 15 ? stone : air;
                    case "Bfoliage" -> localY < 15 ? stone : localY == 15 ? leaves : air;
                    case "Bwater" -> localY < 15 ? stone : localY == 15 ? water : air;
                    case "Bgrass" -> localY < 14 ? stone : localY == 14 ? grass : localY == 15 ? shortGrass : air;
                    case "C32" -> localY < 4 ? stone : localY < 7 ? air : localY == 7 ? leaves : localY == 9 ? grass : air;
                    case "NEG" -> y < -16 ? stone : y == -16 ? grass : air;
                    case "D24N", "D24S", "D24" -> localY < firstAvailable - minY - 1 ? stone
                        : localY == firstAvailable - minY - 1 ? grass : air;
                    default -> localY < 15 ? stone : localY == 15 ? grass : air;
                };
                blocks[Math.floorDiv(y, 16) - Math.floorDiv(minY, 16)].set(x, y & 15, z, cell);
                if (name.equals("Bfoliage") && x == 0 && z == 0 && localY == 15) {
                    blocks[Math.floorDiv(y, 16) - Math.floorDiv(minY, 16)].set(x, y & 15, z, Blocks.VINE.defaultBlockState());
                }
            }
        }
        return ChunkSnapshotFactory.create(blocks, biomes, heights, dimension, pos, minY, height);
    }

    private static Map<ChunkPos, ChunkSnapshot> sourceSnapshots(final String id, final Holder<Biome> center, final Holder<Biome> forest, final Holder<Biome> swamp, final Holder<Biome> desert, final ChunkSnapshot target, final DimensionType dimension, final int radius, final Holder<Biome> negativeDesert, final Holder<Biome> negativeSwamp) {
        final List<int[]> probes = new java.util.ArrayList<>();
        final int min = radius == 0 ? 0 : -radius;
        final int max = radius == 0 ? 15 : 17;
        final int y = 15;
        for (int x = min; x <= max; x++) for (int z = min; z <= max; z++) probes.add(new int[]{x, y, z});
        final Set<ChunkPos> keys = recordedSourceKeys(probes);
        final Map<ChunkPos, ChunkSnapshot> result = new LinkedHashMap<>();
        final List<ChunkPos> ordered = new java.util.ArrayList<>(keys); ordered.sort(java.util.Comparator.comparingInt(ChunkPos::x).thenComparingInt(ChunkPos::z));
        for (final ChunkPos sourcePos : ordered) {
            final Holder<Biome> sourceBiome = sourcePos.x() == 1 && sourcePos.z() == 0 ? desert : sourcePos.x() == -1 ? forest : sourcePos.z() == 1 ? swamp : center;
            result.put(sourcePos, sourcePos.equals(new ChunkPos(0, 0)) ? target : snapshot("F", sourceBiome, sourcePos, dimension, negativeDesert, negativeSwamp));
        }
        return Collections.unmodifiableMap(result);
    }
    private static Set<ChunkPos> recordedSourceKeys(final List<int[]> probes) {
        final Set<ChunkPos> keys = new java.util.LinkedHashSet<>();
        final Holder<Biome> fallback = VanillaRegistries.createLookup().lookupOrThrow(Registries.BIOME).getOrThrow(Biomes.PLAINS);
        final BiomeManager manager = new BiomeManager((quartX, quartY, quartZ) -> {
            keys.add(new ChunkPos(QuartPos.toSection(quartX), QuartPos.toSection(quartZ)));
            return fallback;
        }, BIOME_ZOOM_SEED);
        for (final int[] probe : probes) manager.getBiome(new BlockPos(probe[0], probe[1], probe[2]));
        return keys;
    }
    static ChunkRenderEngine engine(final ChunkRenderFixtureCatalog catalog, final CaseModel model, final int blendRadius) { return engine(catalog, model, blendRadius, null); }
    static ChunkRenderEngine engine(final ChunkRenderFixtureCatalog catalog, final CaseModel model, final int blendRadius, final Set<String> biomeRequests) {
        final ChunkRenderEngine.Settings settings = new ChunkRenderEngine.Settings(
            model.settings.maxHeight, model.settings.iterateUp, model.settings.glassClear,
            model.settings.waterCheckerboard, model.settings.waterClear, model.settings.lavaCheckerboard,
            model.settings.biomeEnabled, blendRadius
        );
        final BiomeManager biomeManager = new BiomeManager((quartX, quartY, quartZ) -> {
            final ChunkPos sourcePos = new ChunkPos(QuartPos.toSection(quartX), QuartPos.toSection(quartZ));
            final ChunkSnapshot source = model.biomeSources.get(sourcePos);
            if (source == null) throw new IllegalStateException("missing required biome source chunk " + sourcePos + " for " + model.id);
            return source.getNoiseBiome(quartX, quartY, quartZ);
        }, BIOME_ZOOM_SEED);
        final BiomeColors biomeColors = BiomeColors.fixture(pos -> { if (biomeRequests != null) biomeRequests.add(pos.getX() + ":" + pos.getY() + ":" + pos.getZ()); return biomeManager.getBiome(pos).value(); }, catalog.colorData, blendRadius);
        return new ChunkRenderEngine(new ChunkRenderEngine.Adapter() {
            @Override public ChunkRenderEngine.Settings settings() { return settings; }
            @Override public int mapColor(final BlockState state) { return catalog.descriptors.blockDescriptor(state).mapColor(); }
            @Override public boolean invisibleBlock(final Block block) { return catalog.descriptors.blockId(block.defaultBlockState()) == model.invisibleId; }
            @Override public boolean iterateUpBaseBlock(final Block block) { return catalog.descriptors.blockId(block.defaultBlockState()) == model.iterateUpBaseId; }
            @Override public ChunkRenderEngine.@org.checkerframework.checker.nullness.qual.Nullable BiomeModifier biomeModifier() { return (color, chunk, pos, radius) -> biomeColors.modifyColorFromBiome(color, chunk, pos); }
            @Override public boolean running() { return true; }
            @Override public boolean rendersPaused() { return false; }
            @Override public boolean shouldRenderColumn(final int blockX, final int blockZ) { return true; }
            @Override public void sleep(final long millis) { }
        });
    }
    private void assertEveryCellHasDescriptor() {
        for (final CaseModel model : cases.values()) {
            final List<ChunkSnapshot> all = new java.util.ArrayList<>();
            all.add(model.center);
            all.addAll(model.neighbors.values());
            all.addAll(model.biomeSources.values());
            for (final ChunkSnapshot snapshot : all) {
                for (int s = 0; s < snapshot.getSectionsCount(); s++) {
                    final PalettedContainer<BlockState> blocks = snapshot.blockStates(s);
                    final PalettedContainer<Holder<Biome>> biomes = snapshot.biomeStates(s);
                    for (int x = 0; x < 16; x++) for (int y = 0; y < 16; y++) for (int z = 0; z < 16; z++) descriptors.blockId(blocks.get(x, y, z));
                    for (int x = 0; x < 4; x++) for (int y = 0; y < 4; y++) for (int z = 0; z < 4; z++) descriptors.biomeId(biomes.get(x, y, z));
                }
            }
        }
    }
    xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshot wireSnapshot(final ChunkSnapshot snapshot) {
        return ChunkSnapshotEncoder.encodePortable(snapshot, descriptors, registryWorld, 11);
    }
    RegistryDescriptorExporter descriptors() { return descriptors; }
    LevelBiomeColorData colorData() { return colorData; }
    Map<String, Integer> biomeRegistryIds() { return biomeRegistryIds; }
    WorldIdentity registryWorld() { return registryWorld; }
    Map<String, CaseModel> cases() { return cases; }
    record QuartRequest(int x, int y, int z) {}
    record BiomeSelection(Holder<Biome> biome, Set<ChunkPos> sourceKeys, List<QuartRequest> quartRequests) {}
    BiomeSelection selectBiome(final CaseModel model, final int blockX, final int blockY, final int blockZ) {
        final Set<ChunkPos> used = new java.util.LinkedHashSet<>();
        final List<QuartRequest> requests = new java.util.ArrayList<>();
        final BiomeManager manager = new BiomeManager((quartX, quartY, quartZ) -> {
            final ChunkPos sourcePos = new ChunkPos(QuartPos.toSection(quartX), QuartPos.toSection(quartZ));
            used.add(sourcePos);
            requests.add(new QuartRequest(quartX, quartY, quartZ));
            final ChunkSnapshot source = model.biomeSources.get(sourcePos);
            if (source == null) throw new IllegalStateException("missing required biome source chunk " + sourcePos + " for " + model.id + " at block " + blockX + "," + blockY + "," + blockZ);
            return source.getNoiseBiome(quartX, quartY, quartZ);
        }, BIOME_ZOOM_SEED);
        return new BiomeSelection(manager.getBiome(new net.minecraft.core.BlockPos(blockX, blockY, blockZ)), used, requests);
    }
    ChunkSnapshot testBiomeSource(final CaseModel model, final Holder<Biome> biome, final ChunkPos pos) {
        return snapshot("F", biome, pos, model.center.dimensionType(), biome, biome);
    }
    Holder<Biome> selectedBiome(final CaseModel model, final int blockX, final int blockY, final int blockZ) {
        return selectBiome(model, blockX, blockY, blockZ).biome();
    }
    long biomeZoomSeed() { return BIOME_ZOOM_SEED; }
    record CaseModel(String id, ChunkSnapshot center, Map<ChunkPos, ChunkSnapshot> neighbors, ChunkRenderEngine.Settings settings, Map<ChunkPos, ChunkSnapshot> biomeSources, xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshot wireSnapshot, RegistryReplace registry, int height, boolean ceiling, int invisibleId, int iterateUpBaseId) { }
}
