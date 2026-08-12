package xyz.jpenilla.squaremap.common.task.render;

import java.util.List;
import java.util.Set;
import java.util.Map;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import xyz.jpenilla.squaremap.common.util.Colors;
import org.junit.jupiter.api.Test;
import net.minecraft.world.level.biome.Biomes;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;
final class ChunkRenderFixtureCatalogTest {
    private static final List<String> VALID_IDS = List.of("flat-solid", "empty-chunk", "north-height-discontinuity", "south-height-discontinuity", "missing-north", "missing-south", "iterate-down", "iterate-up", "ceiling-iterate-down", "ceiling-iterate-up", "max-height-clipped", "water-depth-cap", "water-clear", "water-checkerboard", "lava-depth-checkerboard", "clear-glass", "stained-glass", "glass-disabled", "invisible-block", "biome-off", "biome-grass-radius-0", "biome-foliage-radius-0", "biome-water-radius-0", "biome-blend-radius-3", "biome-blend-cross-boundary", "negative-min-y");

    @Test
    void catalogBuildsExactOrderedCasesRelationsAndDescriptorIds() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        assertEquals(VALID_IDS, List.copyOf(catalog.cases().keySet()));
        assertEquals(10, catalog.descriptors().snapshot().getBlockStatesCount());
        assertEquals(4, catalog.descriptors().snapshot().getBiomesCount());
        assertEquals(Map.of("minecraft:desert", 11, "minecraft:forest", 12, "minecraft:plains", 13, "minecraft:swamp", 14), catalog.biomeRegistryIds());
        final Block stainedBlock = net.minecraft.core.registries.BuiltInRegistries.BLOCK.get(net.minecraft.resources.Identifier.parse("minecraft:red_stained_glass")).orElseThrow().value();
        final List<BlockState> states = List.of(Blocks.AIR.defaultBlockState(), Blocks.GLASS.defaultBlockState(), Blocks.GRASS_BLOCK.defaultBlockState(), Blocks.LAVA.defaultBlockState(), Blocks.OAK_LEAVES.defaultBlockState(), stainedBlock.defaultBlockState(), Blocks.SHORT_GRASS.defaultBlockState(), Blocks.STONE.defaultBlockState(), Blocks.VINE.defaultBlockState(), Blocks.WATER.defaultBlockState());
        final List<String> blockIds = List.of("minecraft:air", "minecraft:glass", "minecraft:grass_block", "minecraft:lava", "minecraft:oak_leaves", "minecraft:red_stained_glass", "minecraft:short_grass", "minecraft:stone", "minecraft:vine", "minecraft:water");
        for (int i = 0; i < states.size(); i++) {
            assertEquals(blockIds.get(i), catalog.descriptors().blockDescriptor(states.get(i)).registryId());
            assertEquals(List.of(states.get(i).toString()), catalog.descriptors().blockDescriptor(states.get(i)).properties());
            assertEquals(i + 1, catalog.descriptors().blockId(states.get(i)));
        }
        assertEquals(stainedBlock.defaultBlockState(), catalog.cases().get("stained-glass").center().getBlockState(new net.minecraft.core.BlockPos(0, 15, 0)));
        assertEquals(Colors.parseHex("#FFFFFF"), catalog.descriptors().blockDescriptor(Blocks.GLASS.defaultBlockState()).mapColor());
        assertEquals(11, catalog.descriptors().snapshot().getBiomes(0).getId());
        assertEquals(12, catalog.descriptors().snapshot().getBiomes(1).getId());
        assertEquals(13, catalog.descriptors().snapshot().getBiomes(2).getId());
        assertEquals(14, catalog.descriptors().snapshot().getBiomes(3).getId());
        assertEquals(0, catalog.cases().get("flat-solid").neighbors().size());
        assertEquals(1, catalog.cases().get("missing-north").neighbors().size());
        assertTrue(catalog.cases().get("missing-north").neighbors().containsKey(new ChunkPos(0, 1)));
        assertTrue(!catalog.cases().get("missing-north").neighbors().containsKey(new ChunkPos(0, -1)));
        assertEquals(1, catalog.cases().get("missing-south").neighbors().size());
        assertTrue(catalog.cases().get("missing-south").neighbors().containsKey(new ChunkPos(0, -1)));
        assertEquals(1, catalog.cases().get("south-height-discontinuity").neighbors().size());
        assertEquals(31, catalog.cases().get("south-height-discontinuity").neighbors().get(new ChunkPos(0, 1)).getHeight(net.minecraft.world.level.levelgen.Heightmap.Types.WORLD_SURFACE, 0, 0));
        assertEquals(1, catalog.cases().get("missing-north").neighbors().size());
        assertTrue(!catalog.cases().get("missing-south").neighbors().containsKey(new ChunkPos(0, 1)));
        assertEquals(List.of(new ChunkPos(-1, -1), new ChunkPos(-1, 0), new ChunkPos(-1, 1), new ChunkPos(0, -1), new ChunkPos(0, 0), new ChunkPos(0, 1), new ChunkPos(1, -1), new ChunkPos(1, 0), new ChunkPos(1, 1)), List.copyOf(catalog.cases().get("biome-blend-cross-boundary").biomeSources().keySet()));
        assertEquals(16, catalog.cases().get("max-height-clipped").settings().maxHeight);
        assertTrue(catalog.cases().get("iterate-up").settings().iterateUp);
        assertTrue(catalog.cases().get("ceiling-iterate-down").ceiling());
        assertEquals(-32, catalog.cases().get("negative-min-y").center().getMinY());
        assertEquals(32, catalog.cases().get("negative-min-y").center().getHeight());
        assertEquals(9, catalog.cases().get("biome-blend-cross-boundary").biomeSources().size());
        assertEquals(0, catalog.cases().get("flat-solid").biomeSources().size());
        assertEquals(0, catalog.cases().get("empty-chunk").biomeSources().size());
        assertEquals(0, catalog.cases().get("invisible-block").biomeSources().size());
        assertEquals(5, catalog.cases().get("biome-grass-radius-0").biomeSources().size());
        assertEquals(5, catalog.cases().get("biome-foliage-radius-0").biomeSources().size());
        assertEquals(5, catalog.cases().get("biome-water-radius-0").biomeSources().size());
        assertEquals(9, catalog.cases().get("biome-blend-radius-3").biomeSources().size());
        assertTrue(catalog.cases().get("ceiling-iterate-down").ceiling());
    }

    @Test
    void everyCaseBindsExactRenderSettingsAndResolvedPredicates() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        for (final var entry : catalog.cases().entrySet()) {
            final String id = entry.getKey();
            final var settings = entry.getValue().settings();
            assertEquals(id.equals("max-height-clipped") ? 16 : -1, settings.maxHeight);
            assertEquals(id.equals("iterate-up") || id.equals("ceiling-iterate-up"), settings.iterateUp);
            assertEquals(!id.equals("glass-disabled"), settings.glassClear);
            assertEquals(id.equals("water-checkerboard"), settings.waterCheckerboard);
            assertEquals(id.equals("water-clear"), settings.waterClear);
            assertEquals(id.equals("lava-depth-checkerboard"), settings.lavaCheckerboard);
            assertEquals(id.startsWith("biome-") && !id.equals("biome-off"), settings.biomeEnabled);
            assertEquals(id.contains("radius-3") || id.contains("cross-boundary") ? 3 : 0, settings.biomeBlend);
            assertEquals(id.equals("invisible-block") ? catalog.descriptors().blockId(Blocks.STONE.defaultBlockState()) : 0, entry.getValue().invisibleId());
            assertEquals(id.equals("ceiling-iterate-up") ? catalog.descriptors().blockId(Blocks.GRASS_BLOCK.defaultBlockState()) : 0, entry.getValue().iterateUpBaseId());
        }
    }
    @Test
    void iterateDownUsesGrassPaletteBoundaryCells() {
        final var center = ChunkRenderFixtureCatalog.create().cases().get("iterate-down").center();
        assertEquals(net.minecraft.world.level.block.Blocks.STONE, center.getBlockState(new net.minecraft.core.BlockPos(0, 13, 0)).getBlock());
        assertEquals(net.minecraft.world.level.block.Blocks.GRASS_BLOCK, center.getBlockState(new net.minecraft.core.BlockPos(0, 14, 0)).getBlock());
        assertEquals(net.minecraft.world.level.block.Blocks.SHORT_GRASS, center.getBlockState(new net.minecraft.core.BlockPos(0, 15, 0)).getBlock());
    }

    @Test
    void namedTemplatesExposeExactRepresentativeLayersAndHeights() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        final var flat = catalog.cases().get("flat-solid").center();
        assertEquals(Blocks.GRASS_BLOCK.defaultBlockState(), flat.getBlockState(new net.minecraft.core.BlockPos(0, 15, 0)));
        assertEquals(Blocks.STONE.defaultBlockState(), flat.getBlockState(new net.minecraft.core.BlockPos(0, 0, 0)));
        final var empty = catalog.cases().get("empty-chunk").center();
        assertEquals(-1, empty.getHeight(net.minecraft.world.level.levelgen.Heightmap.Types.WORLD_SURFACE, 0, 0));
        final var north = catalog.cases().get("north-height-discontinuity").neighbors().get(new ChunkPos(0, -1));
        assertEquals(23, north.getHeight(net.minecraft.world.level.levelgen.Heightmap.Types.WORLD_SURFACE, 0, 0));
        assertEquals(31, north.getHeight(net.minecraft.world.level.levelgen.Heightmap.Types.WORLD_SURFACE, 0, 15));
        final var waterDepth = catalog.cases().get("water-depth-cap").center();
        assertEquals(Blocks.STONE.defaultBlockState(), waterDepth.getBlockState(new net.minecraft.core.BlockPos(0, 0, 0)));
        assertEquals(Blocks.STONE.defaultBlockState(), waterDepth.getBlockState(new net.minecraft.core.BlockPos(0, 4, 0)));
        assertEquals(Blocks.WATER.defaultBlockState(), waterDepth.getBlockState(new net.minecraft.core.BlockPos(0, 5, 0)));
        assertEquals(Blocks.WATER.defaultBlockState(), waterDepth.getBlockState(new net.minecraft.core.BlockPos(0, 15, 0)));
        assertEquals(Blocks.AIR.defaultBlockState(), waterDepth.getBlockState(new net.minecraft.core.BlockPos(0, 16, 0)));
        assertEquals(16, waterDepth.getHeight(net.minecraft.world.level.levelgen.Heightmap.Types.WORLD_SURFACE, 0, 0) + 1);
        final var waterShort = catalog.cases().get("water-clear").center();
        assertEquals(Blocks.STONE.defaultBlockState(), waterShort.getBlockState(new net.minecraft.core.BlockPos(0, 13, 0)));
        assertEquals(Blocks.WATER.defaultBlockState(), waterShort.getBlockState(new net.minecraft.core.BlockPos(0, 14, 0)));
        assertEquals(Blocks.WATER.defaultBlockState(), waterShort.getBlockState(new net.minecraft.core.BlockPos(0, 15, 0)));
        final var lavaShort = catalog.cases().get("lava-depth-checkerboard").center();
        assertEquals(Blocks.LAVA.defaultBlockState(), lavaShort.getBlockState(new net.minecraft.core.BlockPos(0, 14, 0)));
        assertEquals(Blocks.LAVA.defaultBlockState(), lavaShort.getBlockState(new net.minecraft.core.BlockPos(0, 15, 0)));
        final var invisibleBoundary = catalog.cases().get("invisible-block").center();
        assertEquals(Blocks.GRASS_BLOCK.defaultBlockState(), invisibleBoundary.getBlockState(new net.minecraft.core.BlockPos(0, 0, 0)));
        assertEquals(Blocks.STONE.defaultBlockState(), invisibleBoundary.getBlockState(new net.minecraft.core.BlockPos(0, 15, 0)));
        final var grass = catalog.cases().get("biome-grass-radius-0").center();
        assertEquals(Blocks.STONE.defaultBlockState(), grass.getBlockState(new net.minecraft.core.BlockPos(0, 13, 0)));
        assertEquals(Blocks.GRASS_BLOCK.defaultBlockState(), grass.getBlockState(new net.minecraft.core.BlockPos(0, 14, 0)));
        assertEquals(Blocks.SHORT_GRASS.defaultBlockState(), grass.getBlockState(new net.minecraft.core.BlockPos(0, 15, 0)));
        final var foliage = catalog.cases().get("biome-foliage-radius-0").center();
        assertEquals(Blocks.VINE.defaultBlockState(), foliage.getBlockState(new net.minecraft.core.BlockPos(0, 15, 0)));
        final var clearGlass = catalog.cases().get("clear-glass").center();
        assertEquals(Blocks.STONE.defaultBlockState(), clearGlass.getBlockState(new net.minecraft.core.BlockPos(0, 14, 0)));
        assertEquals(Blocks.GLASS.defaultBlockState(), clearGlass.getBlockState(new net.minecraft.core.BlockPos(0, 15, 0)));
        final Block stainedBlock = net.minecraft.core.registries.BuiltInRegistries.BLOCK.get(net.minecraft.resources.Identifier.parse("minecraft:red_stained_glass")).orElseThrow().value();
        final var stainedGlass = catalog.cases().get("stained-glass").center();
        assertEquals(stainedBlock.defaultBlockState(), stainedGlass.getBlockState(new net.minecraft.core.BlockPos(0, 15, 0)));
        final var ceiling = catalog.cases().get("ceiling-iterate-down").center();
        assertEquals(Blocks.STONE.defaultBlockState(), ceiling.getBlockState(new net.minecraft.core.BlockPos(0, 0, 0)));
        assertEquals(Blocks.AIR.defaultBlockState(), ceiling.getBlockState(new net.minecraft.core.BlockPos(0, 4, 0)));
        assertEquals(Blocks.AIR.defaultBlockState(), ceiling.getBlockState(new net.minecraft.core.BlockPos(0, 6, 0)));
        assertEquals(Blocks.OAK_LEAVES.defaultBlockState(), ceiling.getBlockState(new net.minecraft.core.BlockPos(0, 7, 0)));
        assertEquals(Blocks.AIR.defaultBlockState(), ceiling.getBlockState(new net.minecraft.core.BlockPos(0, 8, 0)));
        assertEquals(Blocks.GRASS_BLOCK.defaultBlockState(), ceiling.getBlockState(new net.minecraft.core.BlockPos(0, 9, 0)));
        assertEquals(Blocks.AIR.defaultBlockState(), ceiling.getBlockState(new net.minecraft.core.BlockPos(0, 10, 0)));
        assertEquals(Blocks.AIR.defaultBlockState(), ceiling.getBlockState(new net.minecraft.core.BlockPos(0, 31, 0)));
        assertEquals(10, ceiling.getHeight(net.minecraft.world.level.levelgen.Heightmap.Types.WORLD_SURFACE, 0, 0) + 1);
        final var negative = catalog.cases().get("negative-min-y").center();
        assertEquals(Blocks.STONE.defaultBlockState(), negative.getBlockState(new net.minecraft.core.BlockPos(0, -32, 0)));
        assertEquals(Blocks.STONE.defaultBlockState(), negative.getBlockState(new net.minecraft.core.BlockPos(0, -17, 0)));
        assertEquals(Blocks.GRASS_BLOCK.defaultBlockState(), negative.getBlockState(new net.minecraft.core.BlockPos(0, -16, 0)));
        assertEquals(Blocks.AIR.defaultBlockState(), negative.getBlockState(new net.minecraft.core.BlockPos(0, -15, 0)));
        assertEquals(Biomes.DESERT, negative.biomeStates(0).get(0, 0, 0).unwrapKey().orElseThrow());
        assertEquals(Biomes.SWAMP, negative.biomeStates(1).get(0, 0, 0).unwrapKey().orElseThrow());
        assertEquals(Biomes.DESERT, negative.getNoiseBiome(0, -8, 0).unwrapKey().orElseThrow());
        assertEquals(Biomes.SWAMP, negative.getNoiseBiome(0, -4, 0).unwrapKey().orElseThrow());
        assertEquals(-15, negative.getHeight(net.minecraft.world.level.levelgen.Heightmap.Types.WORLD_SURFACE, 0, 0) + 1);
    }
    @Test
    void descriptorTablePinsEveryRendererFieldAndWorldIdentity() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        final Block stainedBlock = net.minecraft.core.registries.BuiltInRegistries.BLOCK.get(net.minecraft.resources.Identifier.parse("minecraft:red_stained_glass")).orElseThrow().value();
        final List<BlockState> states = List.of(Blocks.AIR.defaultBlockState(), Blocks.GLASS.defaultBlockState(), Blocks.GRASS_BLOCK.defaultBlockState(), Blocks.LAVA.defaultBlockState(), Blocks.OAK_LEAVES.defaultBlockState(), stainedBlock.defaultBlockState(), Blocks.SHORT_GRASS.defaultBlockState(), Blocks.STONE.defaultBlockState(), Blocks.VINE.defaultBlockState(), Blocks.WATER.defaultBlockState());
        final List<String> registryIds = List.of("minecraft:air", "minecraft:glass", "minecraft:grass_block", "minecraft:lava", "minecraft:oak_leaves", "minecraft:red_stained_glass", "minecraft:short_grass", "minecraft:stone", "minecraft:vine", "minecraft:water");
        final List<String> properties = List.of("Block{minecraft:air}", "Block{minecraft:glass}", "Block{minecraft:grass_block}[snowy=false]", "Block{minecraft:lava}[level=0]", "Block{minecraft:oak_leaves}[distance=7,persistent=false,waterlogged=false]", "Block{minecraft:red_stained_glass}", "Block{minecraft:short_grass}", "Block{minecraft:stone}", "Block{minecraft:vine}[east=false,north=false,south=false,up=false,west=false]", "Block{minecraft:water}[level=0]");
        final int[] colors = {0, 0xFFFFFF, 8368696, 0xFF0000, 31744, 10040115, 31744, 7368816, 31744, 4210943};
        final xyz.jpenilla.squaremap.bridge.v1.BlockTransparency[] transparency = {
            xyz.jpenilla.squaremap.bridge.v1.BlockTransparency.BLOCK_TRANSPARENCY_TRANSLUCENT,
            xyz.jpenilla.squaremap.bridge.v1.BlockTransparency.BLOCK_TRANSPARENCY_TRANSLUCENT,
            xyz.jpenilla.squaremap.bridge.v1.BlockTransparency.BLOCK_TRANSPARENCY_OPAQUE,
            xyz.jpenilla.squaremap.bridge.v1.BlockTransparency.BLOCK_TRANSPARENCY_OPAQUE,
            xyz.jpenilla.squaremap.bridge.v1.BlockTransparency.BLOCK_TRANSPARENCY_OPAQUE,
            xyz.jpenilla.squaremap.bridge.v1.BlockTransparency.BLOCK_TRANSPARENCY_TRANSLUCENT,
            xyz.jpenilla.squaremap.bridge.v1.BlockTransparency.BLOCK_TRANSPARENCY_OPAQUE,
            xyz.jpenilla.squaremap.bridge.v1.BlockTransparency.BLOCK_TRANSPARENCY_OPAQUE,
            xyz.jpenilla.squaremap.bridge.v1.BlockTransparency.BLOCK_TRANSPARENCY_OPAQUE,
            xyz.jpenilla.squaremap.bridge.v1.BlockTransparency.BLOCK_TRANSPARENCY_OPAQUE
        };
        final boolean[] glass = {false, true, false, false, false, true, false, false, false, false};
        final int[] alpha = {0, 25, 0, 0, 0, 50, 0, 0, 0, 0};
        final xyz.jpenilla.squaremap.bridge.v1.FluidClass[] fluids = {
            xyz.jpenilla.squaremap.bridge.v1.FluidClass.FLUID_CLASS_NONE, xyz.jpenilla.squaremap.bridge.v1.FluidClass.FLUID_CLASS_NONE,
            xyz.jpenilla.squaremap.bridge.v1.FluidClass.FLUID_CLASS_NONE, xyz.jpenilla.squaremap.bridge.v1.FluidClass.FLUID_CLASS_LAVA,
            xyz.jpenilla.squaremap.bridge.v1.FluidClass.FLUID_CLASS_NONE, xyz.jpenilla.squaremap.bridge.v1.FluidClass.FLUID_CLASS_NONE,
            xyz.jpenilla.squaremap.bridge.v1.FluidClass.FLUID_CLASS_NONE, xyz.jpenilla.squaremap.bridge.v1.FluidClass.FLUID_CLASS_NONE,
            xyz.jpenilla.squaremap.bridge.v1.FluidClass.FLUID_CLASS_NONE, xyz.jpenilla.squaremap.bridge.v1.FluidClass.FLUID_CLASS_WATER
        };
        final boolean[] air = {true, false, false, false, false, false, false, false, false, false};
        final int[] tint = {0, 0, 1, 0, 2, 0, 1, 0, 2, 3};
        for (int index = 0; index < states.size(); index++) {
            final var descriptor = catalog.descriptors().blockDescriptor(states.get(index));
            assertEquals(index + 1, catalog.descriptors().blockId(states.get(index)));
            assertEquals(registryIds.get(index), descriptor.registryId());
            assertEquals(List.of(properties.get(index)), descriptor.properties());
            assertEquals(colors[index], descriptor.mapColor());
            assertEquals(transparency[index], descriptor.transparency());
            assertEquals(glass[index], descriptor.glass());
            assertEquals(alpha[index], descriptor.glassAlphaPercent());
            assertEquals(fluids[index], descriptor.fluid());
            assertEquals(air[index], descriptor.air());
            assertEquals(tint[index], descriptor.tintIndex());
        }
        final int[] biomeIds = {11, 12, 13, 14};
        final String[] biomeNames = {"minecraft:desert", "minecraft:forest", "minecraft:plains", "minecraft:swamp"};
        final int[] grass = {-4212907, -8798118, -7226023, -9780146};
        final int[] foliage = {-7102941, -11819480, -10116313, 6975545};
        final int[] water = {4159204, 4159204, 4159204, 6388580};
        final var biomeRegistryIds = catalog.biomeRegistryIds();
        for (int index = 0; index < biomeIds.length; index++) {
            final var descriptor = catalog.descriptors().snapshot().getBiomes(index);
            final int expectedId = biomeIds[index];
            assertEquals(expectedId, descriptor.getId());
            assertEquals(biomeNames[index], biomeRegistryIds.keySet().stream().filter(name -> biomeRegistryIds.get(name) == expectedId).findFirst().orElseThrow());
            assertEquals(grass[index], descriptor.getGrassColor());
            assertEquals(foliage[index], descriptor.getFoliageColor());
            assertEquals(water[index], descriptor.getWaterColor());
            assertEquals(0, descriptor.getTintIndex());
        }
        final var registry = catalog.descriptors().snapshot();
        assertEquals(11, registry.getRevision());
        assertEquals("minecraft", registry.getWorld().getNamespace());
        assertEquals("overworld", registry.getWorld().getValue());
        assertEquals(1, registry.getWorld().getEpoch());
        assertEquals(catalog.registryWorld(), registry.getWorld());
    }
    @Test
    void invisiblePredicateChangesRenderedPixels() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        final var model = catalog.cases().get("invisible-block");
        final var visible = new ChunkRenderFixtureCatalog.CaseModel(model.id(), model.center(), model.neighbors(), model.settings(), model.biomeSources(), model.wireSnapshot(), model.registry(), model.height(), model.ceiling(), 0, model.iterateUpBaseId());
        final var hiddenPixels = ChunkRenderFixtureCatalog.engine(catalog, model, 0).renderChunkResult(null, model.center(), null).pixels();
        final var visiblePixels = ChunkRenderFixtureCatalog.engine(catalog, visible, 0).renderChunkResult(null, visible.center(), null).pixels();
        assertFalse(java.util.Arrays.equals(hiddenPixels, visiblePixels));
    }
    @Test
    void allCasesBindExactTopologyAndWireIdentity() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        final Set<String> north = Set.of("north-height-discontinuity", "missing-south", "water-depth-cap", "water-clear", "water-checkerboard", "lava-depth-checkerboard", "clear-glass", "stained-glass", "glass-disabled");
        final Set<String> south = Set.of("south-height-discontinuity", "missing-north");
        final Set<ChunkPos> radiusZeroSources = Set.of(new ChunkPos(-1, 0), new ChunkPos(0, -1), new ChunkPos(0, 0), new ChunkPos(0, 1), new ChunkPos(1, 0));
        final Set<ChunkPos> radiusThreeSources = Set.of(new ChunkPos(-1, -1), new ChunkPos(-1, 0), new ChunkPos(-1, 1), new ChunkPos(0, -1), new ChunkPos(0, 0), new ChunkPos(0, 1), new ChunkPos(1, -1), new ChunkPos(1, 0), new ChunkPos(1, 1));
        for (final var entry : catalog.cases().entrySet()) {
            final String id = entry.getKey();
            final var model = entry.getValue();
            assertEquals(new ChunkPos(0, 0), model.center().pos());
            assertEquals(model.ceiling(), model.center().dimensionType().hasCeiling());
            final Set<ChunkPos> expectedNeighbors = new java.util.HashSet<>();
            if (north.contains(id)) expectedNeighbors.add(new ChunkPos(0, -1));
            if (south.contains(id)) expectedNeighbors.add(new ChunkPos(0, 1));
            assertEquals(expectedNeighbors, model.neighbors().keySet());
            final Set<ChunkPos> expectedSources = id.contains("radius-0") ? radiusZeroSources : id.contains("radius-3") || id.contains("cross-boundary") ? radiusThreeSources : Set.of();
            assertEquals(expectedSources, model.biomeSources().keySet());
            for (final var source : model.biomeSources().entrySet()) assertEquals(source.getKey(), source.getValue().pos());
            final List<xyz.jpenilla.squaremap.common.util.chunksnapshot.ChunkSnapshot> snapshots = new java.util.ArrayList<>();
            snapshots.add(model.center()); snapshots.addAll(model.neighbors().values()); snapshots.addAll(model.biomeSources().values());
            for (final var snapshot : snapshots) {
                assertEquals(model.center().dimensionType(), snapshot.dimensionType());
                assertEquals(model.center().getMinY(), snapshot.getMinY());
                assertEquals(model.center().getHeight(), snapshot.getHeight());
                final var wire = catalog.wireSnapshot(snapshot);
                assertEquals(catalog.registryWorld(), wire.getWorld());
                assertEquals(11, wire.getRevision());
                assertEquals(snapshot.pos().x(), wire.getCoordinate().getX());
                assertEquals(snapshot.pos().z(), wire.getCoordinate().getZ());
                assertEquals(snapshot.getMinY(), wire.getMinY());
                assertEquals(snapshot.getMinY() + snapshot.getHeight() - 1, wire.getMaxY());
                assertEquals(model.ceiling(), wire.getCeiling());
            }
        }
    }
}
