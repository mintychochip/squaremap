package xyz.jpenilla.squaremap.common.task.render;

import java.util.ArrayList;
import java.util.List;
import java.util.HashMap;
import java.util.Map;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.assertThrows;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

/** Behavioral parity gate for direct and production-sink chunk rendering. */
class ChunkRenderEngineParityTest {
    @Test
    void biomeBlendRadiusZeroVsPositiveChangesKnownEastBoundaryPixel() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        final var fixture = catalog.cases().get("biome-blend-radius-3");
        final int[] radiusZero = ChunkRenderFixtureCatalog.engine(catalog, fixture, 0)
            .renderChunkResult(null, fixture.center(), null).pixels();
        final int[] radiusPositive = ChunkRenderFixtureCatalog.engine(catalog, fixture, 3)
            .renderChunkResult(null, fixture.center(), null).pixels();
        final int eastBoundary = 15 * 16;
        assertNotEquals(radiusZero[eastBoundary], radiusPositive[eastBoundary],
            "x=15 must sample the explicit east-neighbor biome at positive radius");
        boolean changed = false;
        for (int i = 0; i < 256; i++) {
            if (radiusZero[i] != radiusPositive[i]) {
                changed = true;
                break;
            }
        }
        assertTrue(changed, "blend radius must alter at least one pixel");
    }

    @Test
    void missingBiomeSourceFailsClosedWithCaseAndCoordinate() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        final var original = catalog.cases().get("biome-blend-radius-3");
        final Map<net.minecraft.world.level.ChunkPos, xyz.jpenilla.squaremap.common.util.chunksnapshot.ChunkSnapshot> sources = new HashMap<>(original.biomeSources());
        sources.remove(new net.minecraft.world.level.ChunkPos(-1, 0));
        final var model = new ChunkRenderFixtureCatalog.CaseModel(original.id(), original.center(), original.neighbors(),
            original.settings(), Map.copyOf(sources), original.wireSnapshot(), original.registry(), original.height(),
            original.ceiling(), original.invisibleId(), original.iterateUpBaseId());
        final IllegalStateException error = assertThrows(IllegalStateException.class, () ->
            ChunkRenderFixtureCatalog.engine(catalog, model, 3).renderChunkResult(null, model.center(), null));
        assertEquals("missing required biome source chunk [-1, 0] for biome-blend-radius-3", error.getMessage());
    }
    @Test
    void externalSinkReceivesCenterAndSouthWritesInOrderWithoutChangingResult() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        final var fixture = catalog.cases().get("south-height-discontinuity");
        final ChunkRenderEngine engine = ChunkRenderFixtureCatalog.engine(catalog, fixture, 0);
        final List<long[]> writes = new ArrayList<>();
        final ChunkRenderEngine.PixelResult withSouth = engine.renderChunk(
            (x, z, color) -> writes.add(new long[] {x, z, color}),
            null, fixture.center(), fixture.neighbors().get(new net.minecraft.world.level.ChunkPos(0, 1)));
        final ChunkRenderEngine.PixelResult centerOnly = engine.renderChunkResult(null, fixture.center(), null);
        assertEquals(272, writes.size());
        assertArrayEquals(centerOnly.pixels(), withSouth.pixels());
        assertArrayEquals(centerOnly.southEdge(), withSouth.southEdge());
        final int centerX = fixture.center().pos().getMinBlockX();
        final int centerZ = fixture.center().pos().getMinBlockZ();
        for (int x = 0; x < 16; x++) {
            for (int z = 0; z < 16; z++) {
                final long[] write = writes.get(x * 16 + z);
                assertEquals(centerX + x, write[0]);
                assertEquals(centerZ + z, write[1]);
                assertEquals(withSouth.pixels()[x * 16 + z], (int) write[2]);
            }
        }
        final List<Integer> expectedSouth = new ArrayList<>();
        engine.scanTopRow((x, z, color) -> expectedSouth.add(color), centerOnly.southEdge().clone(),
            fixture.neighbors().get(new net.minecraft.world.level.ChunkPos(0, 1)));
        for (int x = 0; x < 16; x++) {
            final long[] write = writes.get(256 + x);
            assertEquals(centerX + x, write[0]);
            assertEquals(centerZ + 16, write[1]);
            assertEquals(expectedSouth.get(x).intValue(), (int) write[2]);
        }
    }

    @Test
    void freshDirectAndDispatchEnginesMatchEveryCatalogCase() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        for (final var entry : catalog.cases().entrySet()) {
            final var fixture = entry.getValue();
            final ChunkRenderEngine directEngine = ChunkRenderFixtureCatalog.engine(catalog, fixture, fixture.settings().biomeBlend);
            final ChunkRenderEngine dispatchEngine = ChunkRenderFixtureCatalog.engine(catalog, fixture, fixture.settings().biomeBlend);
            final Map<Long, Integer> writes = new HashMap<>();
            final ChunkRenderEngine.PixelResult dispatched = AbstractRender.renderChunkDispatch(
                dispatchEngine,
                (x, z, color) -> writes.put(key(x, z), color),
                fixture.neighbors().get(new net.minecraft.world.level.ChunkPos(0, -1)),
                fixture.center(),
                fixture.neighbors().get(new net.minecraft.world.level.ChunkPos(0, 1))
            );
            final ChunkRenderEngine.PixelResult direct = directEngine.renderChunkResult(
                fixture.neighbors().get(new net.minecraft.world.level.ChunkPos(0, -1)),
                fixture.center(),
                fixture.neighbors().get(new net.minecraft.world.level.ChunkPos(0, 1))
            );
            assertArrayEquals(direct.pixels(), dispatched.pixels(), entry.getKey());
            assertArrayEquals(direct.southEdge(), dispatched.southEdge(), entry.getKey());
            assertEquals(256 + (fixture.neighbors().containsKey(new net.minecraft.world.level.ChunkPos(0, 1)) ? 16 : 0), writes.size(), entry.getKey());
            for (int x = 0; x < 16; x++) for (int z = 0; z < 16; z++) {
                assertEquals(direct.pixels()[x * 16 + z], writes.get(key(x, z)), entry.getKey() + " pixel " + (x * 16 + z));
            }
        }
    }
    private static long key(final int x, final int z) {
        return ((long) x << 32) ^ (z & 0xffffffffL);
    }
}
