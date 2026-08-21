package xyz.jpenilla.squaremap.common.data;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.awt.image.BufferedImage;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.stream.Stream;
import javax.imageio.ImageIO;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.common.util.FileUtil;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

/** Opt-in pyramid PNG workload driver for paired Java/Rust measurements. */
final class ImagePyramidBenchmarkTest {
    private static final int WARMUP_PASSES = 10;
    private static final int MEASURED_PASSES = 30;
    private static final int MAX_ZOOM = 3;
    private static final int TILE = Image.SIZE;
    private static final int RGBA_BYTES = TILE * TILE * 4;

    private static final Map<String, JsonObject> casesById = new LinkedHashMap<>();

    @Test
    void pyramidWorkload() throws Exception {
        if (!Boolean.getBoolean("squaremap.tileBenchmark")) return;
        final Path manifestPath = Path.of(System.getProperty("squaremap.task11.root"), "testdata/bridge/v2/tiles/manifest.json");
        final byte[] manifestBytes = Files.readAllBytes(manifestPath);
        final JsonObject manifest = JsonParser.parseString(Files.readString(manifestPath)).getAsJsonObject();
        assertEquals(MAX_ZOOM, manifest.get("max_zoom").getAsInt());
        final List<JsonObject> cases = new ArrayList<>();
        for (final var element : manifest.getAsJsonArray("cases")) {
            final JsonObject testCase = element.getAsJsonObject();
            cases.add(testCase);
            casesById.put(testCase.get("id").getAsString(), testCase);
        }
        assertEquals(10, cases.size());
        final Path root = Files.createTempDirectory("squaremap-tile-bench-");
        try {
            final Map<String, Path> dirs = new LinkedHashMap<>();
            for (final JsonObject testCase : cases) {
                final Path dir = root.resolve(testCase.get("id").getAsString());
                Files.createDirectories(dir);
                dirs.put(testCase.get("id").getAsString(), dir);
            }
            for (int pass = 0; pass < WARMUP_PASSES; pass++) {
                for (final JsonObject testCase : cases) {
                    applyCase(testCase, dirs.get(testCase.get("id").getAsString()));
                }
            }
            final long start = System.nanoTime();
            final long[] passNanos = new long[MEASURED_PASSES];
            for (int pass = 0; pass < MEASURED_PASSES; pass++) {
                final long passStart = System.nanoTime();
                for (final JsonObject testCase : cases) {
                    applyCase(testCase, dirs.get(testCase.get("id").getAsString()));
                }
                passNanos[pass] = Math.max(1L, System.nanoTime() - passStart);
            }
            final long elapsed = Math.max(1L, System.nanoTime() - start);
            long checksum = 0;
            for (final JsonObject testCase : cases) {
                final Path caseDir = dirs.get(testCase.get("id").getAsString());
                for (final String relative : pngPaths(caseDir)) {
                    final BufferedImage image = ImageIO.read(caseDir.resolve(relative).toFile());
                    assertNotNull(image, relative);
                    assertEquals(TILE, image.getWidth(), relative);
                    assertEquals(TILE, image.getHeight(), relative);
                    final byte[] rgba = toRgba(image);
                    for (final byte value : rgba) {
                        checksum = checksum * 31 + Byte.toUnsignedLong(value);
                    }
                }
            }
            final long items = (long) MEASURED_PASSES * cases.size();
            final double itemsPerSecond = items * 1_000_000_000.0 / elapsed;
            final String manifestHash = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(manifestBytes));
            final StringBuilder passNanosJson = new StringBuilder("[");
            for (int i = 0; i < passNanos.length; i++) {
                if (i > 0) passNanosJson.append(',');
                passNanosJson.append(passNanos[i]);
            }
            passNanosJson.append(']');
            System.out.printf("{\"backend\":\"java\",\"workload\":\"pyramid-png-v2\",\"case_count\":%d,\"warmup_passes\":%d,\"measured_passes\":%d,\"elapsed_nanos\":%d,\"items_per_second\":%.6f,\"checksum\":%d,\"manifest_hash\":\"%s\",\"pass_nanos\":%s}%n", cases.size(), WARMUP_PASSES, MEASURED_PASSES, elapsed, itemsPerSecond, checksum, manifestHash, passNanosJson);
        } finally {
            FileUtil.deleteRecursively(root);
        }
    }

    private static void applyCase(final JsonObject testCase, final Path directory) {
        if (testCase.has("existing_from")) {
            final String from = testCase.get("existing_from").getAsString();
            final JsonObject prior = casesById.get(from);
            assertNotNull(prior, from);
            applyCase(prior, directory);
        }
        final JsonArray region = testCase.getAsJsonArray("region");
        final int rx = region.get(0).getAsInt();
        final int rz = region.get(1).getAsInt();
        if (testCase.has("existing")) {
            savePattern(directory, rx, rz, testCase.getAsJsonObject("existing"));
        }
        savePattern(directory, rx, rz, testCase.getAsJsonObject("pixels"));
    }

    private static void savePattern(final Path directory, final int rx, final int rz, final JsonObject pixels) {
        final Image image = new Image(new RegionCoordinate(rx, rz), directory, MAX_ZOOM);
        paint(image, pixels);
        image.save();
    }

    private static void paint(final Image image, final JsonObject pixels) {
        final String kind = pixels.get("kind").getAsString();
        switch (kind) {
            case "solid" -> {
                final int argb = parseArgb(pixels.get("argb").getAsString());
                for (int x = 0; x < TILE; x++) {
                    for (int z = 0; z < TILE; z++) {
                        image.setPixel(x, z, argb);
                    }
                }
            }
            case "rect" -> {
                final int argb = parseArgb(pixels.get("argb").getAsString());
                final int x0 = pixels.get("x").getAsInt();
                final int z0 = pixels.get("z").getAsInt();
                final int w = pixels.get("w").getAsInt();
                final int h = pixels.get("h").getAsInt();
                for (int x = x0; x < x0 + w; x++) {
                    for (int z = z0; z < z0 + h; z++) {
                        image.setPixel(x, z, argb);
                    }
                }
            }
            case "checker" -> {
                final int step = pixels.get("step").getAsInt();
                final int even = parseArgb(pixels.get("even").getAsString());
                final int odd = parseArgb(pixels.get("odd").getAsString());
                for (int x = 0; x < TILE; x++) {
                    for (int z = 0; z < TILE; z++) {
                        final boolean isEven = ((x / step) + (z / step)) % 2 == 0;
                        image.setPixel(x, z, isEven ? even : odd);
                    }
                }
            }
            case "gradient" -> {
                for (int x = 0; x < TILE; x++) {
                    for (int z = 0; z < TILE; z++) {
                        image.setPixel(x, z, 0xFF000000 | ((x & 255) << 16) | ((z & 255) << 8));
                    }
                }
            }
            default -> throw new IllegalArgumentException("unknown pixel kind: " + kind);
        }
    }

    private static List<String> pngPaths(final Path caseDir) throws IOException {
        final List<String> paths = new ArrayList<>();
        try (Stream<Path> zooms = Files.list(caseDir)) {
            for (final Path zoom : zooms.filter(Files::isDirectory).toList()) {
                try (Stream<Path> children = Files.list(zoom)) {
                    children.filter(path -> path.getFileName().toString().endsWith(".png"))
                        .map(path -> caseDir.relativize(path).toString().replace('\\', '/'))
                        .forEach(paths::add);
                }
            }
        }
        paths.sort(Comparator.naturalOrder());
        return paths;
    }

    private static byte[] toRgba(final BufferedImage image) {
        final byte[] rgba = new byte[RGBA_BYTES];
        for (int z = 0; z < TILE; z++) {
            for (int x = 0; x < TILE; x++) {
                final int argb = image.getRGB(x, z);
                final int offset = (z * TILE + x) * 4;
                rgba[offset] = (byte) ((argb >> 16) & 0xFF);
                rgba[offset + 1] = (byte) ((argb >> 8) & 0xFF);
                rgba[offset + 2] = (byte) (argb & 0xFF);
                rgba[offset + 3] = (byte) ((argb >>> 24) & 0xFF);
            }
        }
        return rgba;
    }

    private static int parseArgb(final String hex) {
        assertTrue(hex.startsWith("0x") || hex.startsWith("0X"), hex);
        return Integer.parseUnsignedInt(hex.substring(2), 16);
    }
}
