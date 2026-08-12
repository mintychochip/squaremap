package xyz.jpenilla.squaremap.common.task.render;

import com.google.gson.JsonObject;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import java.util.Map;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.QuartPos;
import net.minecraft.core.registries.Registries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.BiomeManager;
import net.minecraft.world.level.biome.Biomes;
import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;
import org.junit.jupiter.api.Test;

final class ChunkRenderBiomeSelectorTest {
    private static final SelectorVector[] SELECTOR_VECTORS = {
        new SelectorVector(-65, -33, 31, -17, -9, 7),
        new SelectorVector(-17, -1, 15, -5, -1, 3),
        new SelectorVector(-16, 0, 16, -4, -1, 3),
        new SelectorVector(-15, 1, 17, -4, 0, 4),
        new SelectorVector(-3, -3, -3, -1, -2, -1),
        new SelectorVector(-2, -2, -2, -1, -1, -1),
        new SelectorVector(-1, -1, -1, -1, -1, -1),
        new SelectorVector(0, 0, 0, -1, -1, -1),
        new SelectorVector(1, 1, 1, 0, 0, 0),
        new SelectorVector(2, 2, 2, 0, 0, 0),
        new SelectorVector(3, 3, 3, 0, 0, 1),
        new SelectorVector(4, 4, 4, 1, 1, 1),
        new SelectorVector(5, 5, 5, 1, 1, 1),
        new SelectorVector(15, 63, -17, 3, 15, -5),
        new SelectorVector(16, 64, -16, 3, 16, -4),
        new SelectorVector(17, 65, -15, 4, 16, -4),
        new SelectorVector(31, -64, 32, 7, -17, 8),
        new SelectorVector(32, -63, 33, 8, -16, 8),
        new SelectorVector(33, -62, 34, 8, -16, 8),
        new SelectorVector(Integer.MIN_VALUE, 0, Integer.MAX_VALUE, 536870911, -1, 536870912),
        new SelectorVector(Integer.MAX_VALUE, 0, Integer.MIN_VALUE, 536870911, -1, 536870912)
    };

    @Test void selectorMatchesIndependentSeededQuartTraceAndHeterogeneousBiomeOracle() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        final var model = catalog.cases().get("biome-blend-cross-boundary");
        final var biomes = VanillaRegistries.createLookup().lookupOrThrow(Registries.BIOME);
        final Holder<Biome> plains = biomes.getOrThrow(Biomes.PLAINS);
        final Holder<Biome> forest = biomes.getOrThrow(Biomes.FOREST);
        final Holder<Biome> swamp = biomes.getOrThrow(Biomes.SWAMP);
        final Holder<Biome> desert = biomes.getOrThrow(Biomes.DESERT);
        final Map<ChunkPos, xyz.jpenilla.squaremap.common.util.chunksnapshot.ChunkSnapshot> sourcesForVectors = new java.util.LinkedHashMap<>();
        for (final SelectorVector vector : SELECTOR_VECTORS) {
            final ChunkPos source = new ChunkPos(QuartPos.toSection(vector.quartX()), QuartPos.toSection(vector.quartZ()));
            if (!sourcesForVectors.containsKey(source)) {
                final Holder<Biome> sourceBiome = oracleBiome(source, plains, forest, swamp, desert);
                sourcesForVectors.put(source, source.equals(new ChunkPos(0, 0)) ? model.center() : catalog.testBiomeSource(model, sourceBiome, source));
            }
        }
        final var vectorModel = new ChunkRenderFixtureCatalog.CaseModel(model.id(), model.center(), model.neighbors(), model.settings(), sourcesForVectors, model.wireSnapshot(), model.registry(), model.height(), model.ceiling(), model.invisibleId(), model.iterateUpBaseId());
        for (final SelectorVector vector : SELECTOR_VECTORS) {
            final ChunkPos expectedSource = new ChunkPos(QuartPos.toSection(vector.quartX()), QuartPos.toSection(vector.quartZ()));
            final ChunkRenderFixtureCatalog.BiomeSelection selected = catalog.selectBiome(vectorModel, vector.blockX(), vector.blockY(), vector.blockZ());
            assertEquals(oracleBiomeId(catalog, expectedSource), catalog.biomeRegistryIds().get(selected.biome().unwrapKey().orElseThrow().identifier().toString()));
            assertEquals(List.of(new ChunkRenderFixtureCatalog.QuartRequest(vector.quartX(), vector.quartY(), vector.quartZ())), selected.quartRequests());
            assertEquals(Set.of(expectedSource), selected.sourceKeys());
        }
        final int[][] representatives = {
            {-3, 15, -3, -1, 3, -1, -1, -1}, {-3, 15, 0, -1, 4, 0, -1, 0}, {-3, 15, 16, -1, 3, 3, -1, 0},
            {0, 15, -2, 0, 3, -1, 0, -1}, {-1, 15, 11, -1, 3, 2, -1, 0}, {-1, 15, 17, 0, 3, 4, 0, 1},
            {16, 15, -1, 4, 3, -1, 1, -1}, {15, 15, 7, 4, 3, 1, 1, 0}, {17, 15, 17, 4, 3, 4, 1, 1}
        };
        for (final int[] representative : representatives) {
            final ChunkRenderFixtureCatalog.BiomeSelection actual = catalog.selectBiome(model, representative[0], representative[1], representative[2]);
            final ChunkPos source = new ChunkPos(representative[6], representative[7]);
            assertEquals(oracleBiomeId(catalog, source), catalog.biomeRegistryIds().get(actual.biome().unwrapKey().orElseThrow().identifier().toString()));
            assertEquals(List.of(new ChunkRenderFixtureCatalog.QuartRequest(representative[3], representative[4], representative[5])), actual.quartRequests());
            assertEquals(Set.of(source), actual.sourceKeys());
        }
    }

    private record SelectorVector(int blockX, int blockY, int blockZ, int quartX, int quartY, int quartZ) {}

    @Test void selectorFailsBeforeRenderingWhenSourceIsMissing() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        final var original = catalog.cases().get("biome-blend-radius-3");
        final var missing = new ChunkRenderFixtureCatalog.CaseModel(original.id(), original.center(), original.neighbors(), original.settings(), java.util.Map.of(), original.wireSnapshot(), original.registry(), original.height(), original.ceiling(), original.invisibleId(), original.iterateUpBaseId());
        assertThrows(IllegalStateException.class, () -> catalog.selectedBiome(missing, 0, 15, 0));
    }

    @Test void sourceMapCoversExactlySeededSelectorNeighborhood() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        for (final String id : new String[]{"biome-grass-radius-0", "biome-foliage-radius-0", "biome-water-radius-0", "biome-blend-radius-3", "biome-blend-cross-boundary"}) {
            final var model = catalog.cases().get(id);
            final Set<ChunkPos> expected = id.contains("radius-0")
                ? Set.of(new ChunkPos(-1, 0), new ChunkPos(0, -1), new ChunkPos(0, 0), new ChunkPos(0, 1), new ChunkPos(1, 0))
                : Set.of(new ChunkPos(-1, -1), new ChunkPos(-1, 0), new ChunkPos(-1, 1), new ChunkPos(0, -1), new ChunkPos(0, 0), new ChunkPos(0, 1), new ChunkPos(1, -1), new ChunkPos(1, 0), new ChunkPos(1, 1));
            assertEquals(expected, model.biomeSources().keySet());
            for (final var entry : model.biomeSources().entrySet()) assertEquals(entry.getKey(), entry.getValue().pos());
            assertSame(model.center(), model.biomeSources().get(new ChunkPos(0, 0)));
            assertArrayEquals(catalog.wireSnapshot(model.center()).toByteArray(), catalog.wireSnapshot(model.biomeSources().get(new ChunkPos(0, 0))).toByteArray());
        }
    }
    @Test void blendCasesExerciseNonzeroRadiusAndAllNineSources() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        final var radiusZero = catalog.cases().get("biome-grass-radius-0");
        final var radiusThree = catalog.cases().get("biome-blend-radius-3");
        final var cross = catalog.cases().get("biome-blend-cross-boundary");
        final var radiusZeroSamples = ChunkRenderFixtureDocument.build(catalog).document().getAsJsonArray("valid").asList().stream().filter(value -> value.getAsJsonObject().get("id").getAsString().equals("biome-grass-radius-0")).findFirst().orElseThrow().getAsJsonObject().getAsJsonArray("grass_samples");
        final var radiusThreeSamples = ChunkRenderFixtureDocument.build(catalog).document().getAsJsonArray("valid").asList().stream().filter(value -> value.getAsJsonObject().get("id").getAsString().equals("biome-blend-radius-3")).findFirst().orElseThrow().getAsJsonObject().getAsJsonArray("grass_samples");
        final var crossSamples = ChunkRenderFixtureDocument.build(catalog).document().getAsJsonArray("valid").asList().stream().filter(value -> value.getAsJsonObject().get("id").getAsString().equals("biome-blend-cross-boundary")).findFirst().orElseThrow().getAsJsonObject().getAsJsonArray("grass_samples");
        assertEquals(3, radiusThree.settings().biomeBlend);
        assertEquals(0, radiusZero.settings().biomeBlend);
        assertEquals(256, radiusZeroSamples.size());
        assertEquals(441, radiusThreeSamples.size());
        assertEquals(441, crossSamples.size());
        final Set<Integer> crossBiomeIds = new HashSet<>();
        for (final var sample : crossSamples) crossBiomeIds.add(sample.getAsJsonObject().get("biome_id").getAsInt());
        assertTrue(crossBiomeIds.contains(catalog.biomeRegistryIds().get("minecraft:forest")));
        assertTrue(crossBiomeIds.contains(catalog.biomeRegistryIds().get("minecraft:desert")));
        assertTrue(crossBiomeIds.contains(catalog.biomeRegistryIds().get("minecraft:swamp")));
        assertTrue(crossBiomeIds.contains(catalog.biomeRegistryIds().get("minecraft:plains")));
    }

    @Test void grassSampleVectorsHaveExactOrderAndProvenance() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        final ChunkRenderFixtureDocument.Projection projection = ChunkRenderFixtureDocument.build(catalog);
        for (final var value : projection.document().getAsJsonArray("valid")) {
            final JsonObject row = value.getAsJsonObject();
            final String id = row.get("id").getAsString();
            final int expected = id.equals("biome-grass-radius-0") ? 256 : id.equals("biome-blend-radius-3") || id.equals("biome-blend-cross-boundary") ? 441 : 0;
            final var samples = row.getAsJsonArray("grass_samples");
            assertEquals(expected, samples.size());
            final var model = catalog.cases().get(id);
            final Set<String> coordinates = new HashSet<>();
            for (int index = 0; index < samples.size(); index++) {
                final JsonObject sample = samples.get(index).getAsJsonObject();
                final int x = sample.get("block_x").getAsInt();
                final int y = sample.get("block_y").getAsInt();
                final int z = sample.get("block_z").getAsInt();
                final String coordinate = x + ":" + y + ":" + z;
                assertTrue(coordinates.add(coordinate));
                assertEquals(15, y);
                final var selected = catalog.selectBiome(model, x, y, z).biome();
                assertEquals(catalog.descriptors().biomeId(selected), sample.get("biome_id").getAsInt());
                final int expectedArgb = selected.value().getSpecialEffects().grassColorModifier().modifyColor(x, z, catalog.colorData().grassColor(selected.value()));
                assertEquals(Integer.toUnsignedLong(expectedArgb), sample.get("resolved_grass_argb").getAsLong());
                assertTrue(sample.get("resolved_grass_argb").getAsLong() >= 0 && sample.get("resolved_grass_argb").getAsLong() <= 0xffff_ffffL);
                if (id.equals("biome-grass-radius-0")) { assertEquals(index / 16, x); assertEquals(index % 16, z); }
                if (id.equals("biome-blend-radius-3") || id.equals("biome-blend-cross-boundary")) { assertEquals(-3 + index / 21, x); assertEquals(-3 + index % 21, z); }
            }
        }
    }
    @Test void grassTuplesCoverEveryEngineBiomeModifierRequest() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        for (final String id : new String[]{"biome-grass-radius-0", "biome-blend-radius-3", "biome-blend-cross-boundary"}) {
            final var model = catalog.cases().get(id);
            final Set<String> requests = new HashSet<>();
            ChunkRenderFixtureCatalog.engine(catalog, model, model.settings().biomeBlend, requests)
                .renderChunkResult(model.neighbors().get(new ChunkPos(0, -1)), model.center(), model.neighbors().get(new ChunkPos(0, 1)));
            final Set<String> samples = new HashSet<>();
            final var row = ChunkRenderFixtureDocument.build(catalog).document().getAsJsonArray("valid").asList().stream()
                .map(value -> value.getAsJsonObject()).filter(value -> value.get("id").getAsString().equals(id)).findFirst().orElseThrow();
            for (final var sample : row.getAsJsonArray("grass_samples")) {
                final var object = sample.getAsJsonObject();
                samples.add(object.get("block_x").getAsInt() + ":" + object.get("block_y").getAsInt() + ":" + object.get("block_z").getAsInt());
            }
            assertEquals(id.equals("biome-grass-radius-0") ? 256 : 441, requests.size(), id + " renderer lookup count");
            assertEquals(samples, requests, id + " renderer lookup coordinates");
        }
    }

    private static Holder<Biome> oracleBiome(final ChunkPos source, final Holder<Biome> plains, final Holder<Biome> forest, final Holder<Biome> swamp, final Holder<Biome> desert) {
        return source.x() == 1 && source.z() == 0 ? desert : source.x() == -1 ? forest : source.z() == 1 ? swamp : plains;
    }
    private static int oracleBiomeId(final ChunkRenderFixtureCatalog catalog, final ChunkPos source) {
        final String id = source.x() == 1 && source.z() == 0 ? "minecraft:desert" : source.x() == -1 ? "minecraft:forest" : source.z() == 1 ? "minecraft:swamp" : "minecraft:plains";
        return catalog.biomeRegistryIds().get(id);
    }
}
