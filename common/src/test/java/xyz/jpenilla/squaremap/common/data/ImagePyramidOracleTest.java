package xyz.jpenilla.squaremap.common.data;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.awt.image.BufferedImage;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.stream.Stream;
import javax.imageio.ImageIO;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

/** Java {@link Image} pyramid oracle for the shared tile catalog. */
class ImagePyramidOracleTest {
    private static final Path ROOT = Path.of(System.getProperty("squaremap.task11.root"));
    private static final Path MANIFEST = ROOT.resolve("testdata/bridge/v2/tiles/manifest.json");
    private static final Path COMMITTED_HASHES = ROOT.resolve("testdata/bridge/v2/tiles/java-oracle-hashes.json");
    private static final Path OUT = ROOT.resolve("build/tile-oracle/java");
    private static final int MAX_ZOOM = 3;
    private static final int TILE = Image.SIZE;
    private static final int RGBA_BYTES = TILE * TILE * 4;

    private static JsonObject manifest;
    private static JsonObject report;
    private static final Map<String, JsonObject> casesById = new LinkedHashMap<>();

    @BeforeAll
    static void generateOracle() throws Exception {
        final byte[] manifestBytes = Files.readAllBytes(MANIFEST);
        manifest = JsonParser.parseString(Files.readString(MANIFEST)).getAsJsonObject();
        assertEquals(MAX_ZOOM, manifest.get("max_zoom").getAsInt());

        if (Files.exists(OUT)) {
            try (Stream<Path> walk = Files.walk(OUT)) {
                walk.sorted(Comparator.reverseOrder()).forEach(path -> {
                    try {
                        Files.delete(path);
                    } catch (final IOException ex) {
                        throw new RuntimeException(ex);
                    }
                });
            }
        }
        Files.createDirectories(OUT);

        for (final var element : manifest.getAsJsonArray("cases")) {
            final JsonObject testCase = element.getAsJsonObject();
            casesById.put(testCase.get("id").getAsString(), testCase);
        }

        report = new JsonObject();
        report.addProperty("backend", "java");
        report.addProperty("manifest_hash", sha256Hex(manifestBytes));
        report.addProperty("max_zoom", MAX_ZOOM);
        final JsonArray reportCases = new JsonArray();

        for (final JsonObject testCase : casesById.values()) {
            final String id = testCase.get("id").getAsString();
            final Path caseDir = OUT.resolve(id);
            Files.createDirectories(caseDir);
            applyCase(testCase, caseDir);

            final List<String> paths = pngPaths(caseDir);
            final JsonObject row = new JsonObject();
            row.addProperty("id", id);
            final JsonArray pathArray = new JsonArray();
            final JsonArray hashes = new JsonArray();
            for (final String relative : paths) {
                final Path png = caseDir.resolve(relative);
                final BufferedImage image = ImageIO.read(png.toFile());
                assertNotNull(image, relative);
                assertEquals(TILE, image.getWidth(), relative);
                assertEquals(TILE, image.getHeight(), relative);
                final byte[] rgba = toRgba(image);
                Files.write(caseDir.resolve(relative.replace(".png", ".rgba")), rgba);
                pathArray.add(relative);
                hashes.add(sha256Hex(rgba));
            }
            row.add("paths", pathArray);
            row.add("pixel_sha256", hashes);
            reportCases.add(row);
        }
        report.add("cases", reportCases);
        final Gson gson = new GsonBuilder().setPrettyPrinting().disableHtmlEscaping().create();
        Files.writeString(OUT.resolve("report.json"), gson.toJson(report));
    }

    @Test
    void oracleGeneratesReportAndRgbaDumpsForEveryCase() throws Exception {
        assertTrue(Files.isRegularFile(OUT.resolve("report.json")));
        assertEquals("java", report.get("backend").getAsString());
        assertEquals(sha256Hex(Files.readAllBytes(MANIFEST)), report.get("manifest_hash").getAsString());
        assertEquals(MAX_ZOOM, report.get("max_zoom").getAsInt());
        final JsonArray reportCases = report.getAsJsonArray("cases");
        assertEquals(manifest.getAsJsonArray("cases").size(), reportCases.size());
        int index = 0;
        for (final var element : manifest.getAsJsonArray("cases")) {
            final JsonObject expected = element.getAsJsonObject();
            final JsonObject actual = reportCases.get(index).getAsJsonObject();
            final String id = expected.get("id").getAsString();
            assertEquals(id, actual.get("id").getAsString());
            final JsonArray paths = actual.getAsJsonArray("paths");
            final JsonArray hashes = actual.getAsJsonArray("pixel_sha256");
            assertTrue(paths.size() > 0, id);
            assertEquals(paths.size(), hashes.size(), id);
            for (int i = 0; i < paths.size(); i++) {
                final String relative = paths.get(i).getAsString();
                final Path png = OUT.resolve(id).resolve(relative);
                final Path rgba = OUT.resolve(id).resolve(relative.replace(".png", ".rgba"));
                assertTrue(Files.isRegularFile(png), png.toString());
                assertTrue(Files.isRegularFile(rgba), rgba.toString());
                assertEquals(RGBA_BYTES, Files.size(rgba), rgba.toString());
                assertEquals(hashes.get(i).getAsString(), sha256Hex(Files.readAllBytes(rgba)), relative);
                final BufferedImage image = ImageIO.read(png.toFile());
                assertNotNull(image, relative);
                assertEquals(TILE, image.getWidth());
                assertEquals(TILE, image.getHeight());
            }
            index++;
        }
    }

    @Test
    void pathTableForSolidOriginEastAndNegative() throws Exception {
        assertPaths("solid-origin", List.of("3/0_0.png", "2/0_0.png", "1/0_0.png", "0/0_0.png"));
        assertEquals(0xFF112233, readPng("solid-origin", "3/0_0.png").getRGB(0, 0));
        assertEquals(0xFF112233, readPng("solid-origin", "2/0_0.png").getRGB(0, 0));
        assertEquals(0xFF112233, readPng("solid-origin", "1/0_0.png").getRGB(0, 0));
        assertEquals(0xFF112233, readPng("solid-origin", "0/0_0.png").getRGB(0, 0));

        assertPaths("solid-east", List.of("3/1_0.png", "2/0_0.png", "1/0_0.png", "0/0_0.png"));
        assertEquals(0xFF445566, readPng("solid-east", "3/1_0.png").getRGB(0, 0));
        assertEquals(0xFF445566, readPng("solid-east", "2/0_0.png").getRGB(256, 0));
        assertEquals(0xFF445566, readPng("solid-east", "1/0_0.png").getRGB(128, 0));
        assertEquals(0xFF445566, readPng("solid-east", "0/0_0.png").getRGB(64, 0));

        assertPaths("solid-negative", List.of("3/-1_-1.png", "2/-1_-1.png", "1/-1_-1.png", "0/-1_-1.png"));
        assertEquals(0xFF778899, readPng("solid-negative", "3/-1_-1.png").getRGB(0, 0));
        assertEquals(0xFF778899, readPng("solid-negative", "2/-1_-1.png").getRGB(256, 256));
        assertEquals(0xFF778899, readPng("solid-negative", "1/-1_-1.png").getRGB(384, 384));
        assertEquals(0xFF778899, readPng("solid-negative", "0/-1_-1.png").getRGB(448, 448));
    }

    @Test
    void checkerStep2Zoom1OriginPixelIsEvenColor() throws Exception {
        assertEquals(0xFFAA0000, readPng("checker-step2", "2/0_0.png").getRGB(0, 0));
    }

    @Test
    void sparseChunk00DoesNotPaintPixel16_0OnNativeZoom() throws Exception {
        assertEquals(0, readPng("sparse-chunk-0-0", "3/0_0.png").getRGB(16, 0));
        assertEquals(0xFF010203, readPng("sparse-chunk-0-0", "3/0_0.png").getRGB(0, 0));
    }

    @Test
    void unsetVsTransparentWritesClearPixelAndLeavesNeighbor() throws Exception {
        final BufferedImage nativeTile = readPng("unset-vs-transparent", "3/0_0.png");
        assertEquals(0, nativeTile.getRGB(0, 0));
        assertEquals(0xFF070707, nativeTile.getRGB(1, 0));
    }

    @Test
    void mergeExistingQuadrantPreservesOriginAndWritesEast() throws Exception {
        final BufferedImage parent = readPng("merge-existing-quadrant", "2/0_0.png");
        assertEquals(0xFF112233, parent.getRGB(0, 0));
        assertEquals(0xFF220000, parent.getRGB(256, 0));
    }

    @Test
    void generatedHashesMatchCommittedJavaOracle() throws Exception {
        assertTrue(Files.isRegularFile(COMMITTED_HASHES), COMMITTED_HASHES.toString());
        final JsonObject committed = JsonParser.parseString(Files.readString(COMMITTED_HASHES)).getAsJsonObject();
        assertEquals(1, committed.get("schema_version").getAsInt());
        assertEquals(MAX_ZOOM, committed.get("max_zoom").getAsInt());
        assertEquals(sha256Hex(Files.readAllBytes(MANIFEST)), committed.get("manifest_hash").getAsString());
        final JsonObject cases = committed.getAsJsonObject("cases");
        assertEquals(manifest.getAsJsonArray("cases").size(), cases.size());
        for (final var element : report.getAsJsonArray("cases")) {
            final JsonObject row = element.getAsJsonObject();
            final String id = row.get("id").getAsString();
            assertTrue(cases.has(id), id);
            final JsonObject expected = cases.getAsJsonObject(id);
            final JsonArray paths = row.getAsJsonArray("paths");
            final JsonArray hashes = row.getAsJsonArray("pixel_sha256");
            assertEquals(paths.size(), expected.size(), id);
            for (int i = 0; i < paths.size(); i++) {
                final String relative = paths.get(i).getAsString();
                assertTrue(expected.has(relative), id + " " + relative);
                assertEquals(expected.get(relative).getAsString(), hashes.get(i).getAsString(), id + " " + relative);
            }
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

    private static void assertPaths(final String id, final List<String> expected) {
        final List<String> actual = new ArrayList<>();
        for (final var element : caseRow(id).getAsJsonArray("paths")) {
            actual.add(element.getAsString());
        }
        assertEquals(expected.size(), actual.size(), id + " " + actual);
        for (final String relative : expected) {
            assertTrue(actual.contains(relative), id + " missing " + relative + " in " + actual);
            assertTrue(Files.isRegularFile(OUT.resolve(id).resolve(relative)), relative);
        }
    }

    private static JsonObject caseRow(final String id) {
        for (final var element : report.getAsJsonArray("cases")) {
            final JsonObject row = element.getAsJsonObject();
            if (id.equals(row.get("id").getAsString())) {
                return row;
            }
        }
        throw new AssertionError(id);
    }

    private static BufferedImage readPng(final String caseId, final String relative) throws IOException {
        final Path file = OUT.resolve(caseId).resolve(relative);
        final BufferedImage image = ImageIO.read(file.toFile());
        assertNotNull(image, relative);
        assertEquals(TILE, image.getWidth(), relative);
        assertEquals(TILE, image.getHeight(), relative);
        return image;
    }

    private static int parseArgb(final String hex) {
        assertTrue(hex.startsWith("0x") || hex.startsWith("0X"), hex);
        return Integer.parseUnsignedInt(hex.substring(2), 16);
    }

    private static String sha256Hex(final byte[] bytes) throws NoSuchAlgorithmException {
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
    }
}
