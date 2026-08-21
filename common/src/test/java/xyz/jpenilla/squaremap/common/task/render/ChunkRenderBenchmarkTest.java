package xyz.jpenilla.squaremap.common.task.render;

import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;

/** Opt-in renderer-only workload driver for paired Java/Rust measurements. */
final class ChunkRenderBenchmarkTest {
    private static final int WARMUP_PASSES = 10;
    private static final int MEASURED_PASSES = 30;

    @Test
    void rendererWorkload() throws Exception {
        if (!Boolean.getBoolean("squaremap.renderBenchmark")) return;
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        final List<ChunkRenderEngine> engines = new ArrayList<>();
        final List<ChunkRenderEngine.PixelResult> expected = new ArrayList<>();
        final List<ChunkRenderFixtureCatalog.CaseModel> models = new ArrayList<>(catalog.cases().values());
        for (final var model : models) {
            final var engine = ChunkRenderFixtureCatalog.engine(catalog, model, model.settings().biomeBlend);
            engines.add(engine);
            expected.add(render(engine, model));
        }
        for (int pass = 0; pass < WARMUP_PASSES; pass++) {
            for (int index = 0; index < engines.size(); index++) render(engines.get(index), models.get(index));
        }
        final long start = System.nanoTime();
        long checksum = 0;
        final long[] passNanos = new long[MEASURED_PASSES];
        for (int pass = 0; pass < MEASURED_PASSES; pass++) {
            final long passStart = System.nanoTime();
            long passChecksum = 0;
            for (int index = 0; index < engines.size(); index++) {
                final var result = render(engines.get(index), models.get(index));
                passChecksum = checksum(passChecksum, result);
            }
            checksum = checksum * 31 + passChecksum;
            passNanos[pass] = Math.max(1L, System.nanoTime() - passStart);
        }
        final long elapsed = Math.max(1L, System.nanoTime() - start);
        for (int index = 0; index < engines.size(); index++) {
            final var result = render(engines.get(index), models.get(index));
            assertArrayEquals(expected.get(index).pixels(), result.pixels(), models.get(index).id());
            assertArrayEquals(expected.get(index).southEdge(), result.southEdge(), models.get(index).id());
        }
        final long items = (long) MEASURED_PASSES * models.size();
        long expectedPassChecksum = 0;
        for (final var result : expected) expectedPassChecksum = checksum(expectedPassChecksum, result);
        long expectedSequenceChecksum = 0;
        for (int pass = 0; pass < MEASURED_PASSES; pass++) expectedSequenceChecksum = expectedSequenceChecksum * 31 + expectedPassChecksum;
        assertEquals(expectedSequenceChecksum, checksum, "deterministic renderer checksum");
        final double itemsPerSecond = items * 1_000_000_000.0 / elapsed;
        final Path manifest = Path.of(System.getProperty("squaremap.task11.root"), "testdata/bridge/v2/render/manifest.json");
        final String manifestHash = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(Files.readAllBytes(manifest)));
        final StringBuilder passNanosJson = new StringBuilder("[");
        for (int i = 0; i < passNanos.length; i++) {
            if (i > 0) passNanosJson.append(',');
            passNanosJson.append(passNanos[i]);
        }
        passNanosJson.append(']');
        final String json = String.format("{\"backend\":\"java\",\"workload\":\"chunk-render-v2\",\"case_count\":%d,\"warmup_passes\":%d,\"measured_passes\":%d,\"elapsed_nanos\":%d,\"items_per_second\":%.6f,\"checksum\":%d,\"manifest_hash\":\"%s\",\"pass_nanos\":%s}", models.size(), WARMUP_PASSES, MEASURED_PASSES, elapsed, itemsPerSecond, checksum, manifestHash, passNanosJson);
        System.out.println(json);
        writeAbOut(json);
    }

    private static void writeAbOut(final String json) throws Exception {
        final String abOut = System.getProperty("squaremap.abOut");
        if (abOut == null || abOut.isBlank()) {
            return;
        }
        final Path path = Path.of(abOut);
        final Path parent = path.getParent();
        if (parent != null) {
            Files.createDirectories(parent);
        }
        Files.writeString(path, json + System.lineSeparator());
    }

    private static ChunkRenderEngine.PixelResult render(final ChunkRenderEngine engine, final ChunkRenderFixtureCatalog.CaseModel model) {
        return engine.renderChunkResult(model.neighbors().get(new net.minecraft.world.level.ChunkPos(0, -1)), model.center(), model.neighbors().get(new net.minecraft.world.level.ChunkPos(0, 1)));
    }

    private static long checksum(long current, final ChunkRenderEngine.PixelResult result) {
        long checksum = current;
        for (final int pixel : result.pixels()) checksum = checksum * 31 + Integer.toUnsignedLong(pixel);
        for (final int edge : result.southEdge()) checksum = checksum * 31 + edge;
        return checksum;
    }
}
