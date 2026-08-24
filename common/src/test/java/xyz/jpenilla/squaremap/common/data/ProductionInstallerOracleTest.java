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
import java.util.HashMap;
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

/**
 * Java {@link Image} oracle for production installer mapping:
 * chunk → region-local pixels → pyramid. Does not prove live Paper.
 */
class ProductionInstallerOracleTest {
    private static final Path ROOT = Path.of(System.getProperty("squaremap.task11.root"));
    private static final Path MANIFEST = ROOT.resolve("testdata/bridge/v2/installer/manifest.json");
    private static final Path RENDER_MANIFEST = ROOT.resolve("testdata/bridge/v2/render/manifest.json");
    private static final Path COMMITTED_HASHES = ROOT.resolve("testdata/bridge/v2/installer/java-oracle-hashes.json");
    private static final Path OUT = ROOT.resolve("build/tile-oracle/installer-java");
    private static final int MAX_ZOOM = 3;
    private static final int TILE = Image.SIZE;
    private static final int CHUNK = 16;
    private static final int CHUNKS_PER_REGION = 32;
    private static final int RGBA_BYTES = TILE * TILE * 4;

    private static JsonObject manifest;
    private static JsonObject report;
    private static final Map<String, int[]> fixturePixels = new HashMap<>();

    @BeforeAll
    static void generateOracle() throws Exception {
        final byte[] manifestBytes = Files.readAllBytes(MANIFEST);
        manifest = JsonParser.parseString(Files.readString(MANIFEST)).getAsJsonObject();
        assertEquals(1, manifest.get("schema_version").getAsInt());
        assertEquals(MAX_ZOOM, manifest.get("max_zoom").getAsInt());

        final JsonObject render = JsonParser.parseString(Files.readString(RENDER_MANIFEST)).getAsJsonObject();
        for (final var element : render.getAsJsonArray("valid")) {
            final JsonObject row = element.getAsJsonObject();
            final JsonArray pixels = row.getAsJsonArray("pixels");
            assertEquals(CHUNK * CHUNK, pixels.size(), row.get("id").getAsString());
            final int[] argb = new int[CHUNK * CHUNK];
            for (int i = 0; i < argb.length; i++) {
                argb[i] = (int) pixels.get(i).getAsLong();
            }
            fixturePixels.put(row.get("id").getAsString(), argb);
        }

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

        report = new JsonObject();
        report.addProperty("backend", "java");
        report.addProperty("manifest_hash", sha256Hex(manifestBytes));
        report.addProperty("max_zoom", MAX_ZOOM);
        final JsonObject cases = new JsonObject();
        final JsonArray reportCases = new JsonArray();

        for (final var element : manifest.getAsJsonArray("cases")) {
            final JsonObject testCase = element.getAsJsonObject();
            final String id = testCase.get("id").getAsString();
            final Path caseDir = OUT.resolve(id);
            Files.createDirectories(caseDir);
            applyCase(testCase, caseDir);

            final List<String> paths = pngPaths(caseDir);
            final JsonObject hashes = new JsonObject();
            final JsonObject row = new JsonObject();
            row.addProperty("id", id);
            final JsonArray pathArray = new JsonArray();
            final JsonArray hashArray = new JsonArray();
            for (final String relative : paths) {
                final Path png = caseDir.resolve(relative);
                final BufferedImage image = ImageIO.read(png.toFile());
                assertNotNull(image, relative);
                assertEquals(TILE, image.getWidth(), relative);
                assertEquals(TILE, image.getHeight(), relative);
                final byte[] rgba = toRgba(image);
                Files.write(caseDir.resolve(relative.replace(".png", ".rgba")), rgba);
                final String hash = sha256Hex(rgba);
                pathArray.add(relative);
                hashArray.add(hash);
                hashes.addProperty(relative, hash);
            }
            row.add("paths", pathArray);
            row.add("pixel_sha256", hashArray);
            reportCases.add(row);
            cases.add(id, hashes);
        }
        report.add("cases", reportCases);
        report.add("hash_map", cases);
        final Gson gson = new GsonBuilder().setPrettyPrinting().disableHtmlEscaping().create();
        Files.writeString(OUT.resolve("report.json"), gson.toJson(report));
    }

    @Test
    void oraclePinsCommittedHashesForEveryInstallerCase() throws Exception {
        assertTrue(Files.isRegularFile(COMMITTED_HASHES), "missing committed oracle " + COMMITTED_HASHES);
        final JsonObject committed = JsonParser.parseString(Files.readString(COMMITTED_HASHES)).getAsJsonObject();
        assertEquals(1, committed.get("schema_version").getAsInt());
        assertEquals(MAX_ZOOM, committed.get("max_zoom").getAsInt());
        assertEquals(sha256Hex(Files.readAllBytes(MANIFEST)), committed.get("manifest_hash").getAsString());
        final JsonObject cases = committed.getAsJsonObject("cases");
        assertEquals(manifest.getAsJsonArray("cases").size(), cases.size());
        final JsonObject generated = report.getAsJsonObject("hash_map");
        for (final var element : manifest.getAsJsonArray("cases")) {
            final String id = element.getAsJsonObject().get("id").getAsString();
            assertTrue(cases.has(id), id);
            assertEquals(generated.getAsJsonObject(id), cases.getAsJsonObject(id), id);
        }
    }

    @Test
    void chunk00NativeOriginMatchesFlatSolidPixel00() throws Exception {
        final int color = fixturePixels.get("flat-solid")[0];
        assertEquals(color, readPng("fixture-flat-solid-at-0-0", "3/0_0.png").getRGB(0, 0));
        assertTrue(pngPaths(OUT.resolve("fixture-flat-solid-at-0-0")).contains("3/0_0.png"));
    }

    @Test
    void chunk10SitsAtRegionLocal16_0() throws Exception {
        final int color = fixturePixels.get("flat-solid")[0];
        assertEquals(color, readPng("fixture-flat-solid-at-1-0", "3/0_0.png").getRGB(16, 0));
        assertEquals(0, readPng("fixture-flat-solid-at-1-0", "3/0_0.png").getRGB(0, 0) & 0x00FFFFFF, "unset origin stays unset/transparent");
    }

    @Test
    void chunk3131SitsAtRegionLocal496_496() throws Exception {
        final int color = fixturePixels.get("flat-solid")[0];
        assertEquals(color, readPng("fixture-flat-solid-at-31-31", "3/0_0.png").getRGB(496, 496));
    }

    @Test
    void negativeChunkUsesNegativeZoomPaths() throws Exception {
        final List<String> paths = pngPaths(OUT.resolve("fixture-flat-solid-at-negative"));
        assertTrue(paths.contains("3/-1_-1.png"), paths.toString());
        assertTrue(paths.contains("0/-1_-1.png"), paths.toString());
        final int color = fixturePixels.get("flat-solid")[0];
        assertEquals(color, readPng("fixture-flat-solid-at-negative", "3/-1_-1.png").getRGB(496, 496));
    }

    @Test
    void chunk320UsesEastNativeTile() throws Exception {
        final List<String> paths = pngPaths(OUT.resolve("fixture-flat-solid-at-32-0"));
        assertTrue(paths.contains("3/1_0.png"), paths.toString());
        final int color = fixturePixels.get("flat-solid")[0];
        assertEquals(color, readPng("fixture-flat-solid-at-32-0", "3/1_0.png").getRGB(0, 0));
    }

    private static void applyCase(final JsonObject testCase, final Path directory) {
        final Map<String, Image> images = new LinkedHashMap<>();
        for (final var install : testCase.getAsJsonArray("installs")) {
            final JsonObject row = install.getAsJsonObject();
            final String fixture = row.get("fixture").getAsString();
            final JsonArray chunk = row.getAsJsonArray("chunk");
            final int chunkX = chunk.get(0).getAsInt();
            final int chunkZ = chunk.get(1).getAsInt();
            final int regionX = Math.floorDiv(chunkX, CHUNKS_PER_REGION);
            final int regionZ = Math.floorDiv(chunkZ, CHUNKS_PER_REGION);
            final int localX = Math.floorMod(chunkX, CHUNKS_PER_REGION) * CHUNK;
            final int localZ = Math.floorMod(chunkZ, CHUNKS_PER_REGION) * CHUNK;
            final int[] pixels = fixturePixels.get(fixture);
            assertNotNull(pixels, fixture);
            final String key = regionX + "," + regionZ;
            final Image image = images.computeIfAbsent(key, ignored -> new Image(new RegionCoordinate(regionX, regionZ), directory, MAX_ZOOM));
            for (int x = 0; x < CHUNK; x++) {
                for (int z = 0; z < CHUNK; z++) {
                    image.setPixel(localX + x, localZ + z, pixels[x * CHUNK + z]);
                }
            }
            image.save();
        }
    }

    private static List<String> pngPaths(final Path caseDir) throws IOException {
        final List<String> paths = new ArrayList<>();
        if (!Files.isDirectory(caseDir)) {
            return paths;
        }
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

    private static BufferedImage readPng(final String caseId, final String relative) throws IOException {
        final Path file = OUT.resolve(caseId).resolve(relative);
        final BufferedImage image = ImageIO.read(file.toFile());
        assertNotNull(image, relative);
        assertEquals(TILE, image.getWidth(), relative);
        assertEquals(TILE, image.getHeight(), relative);
        return image;
    }

    private static String sha256Hex(final byte[] bytes) throws NoSuchAlgorithmException {
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
    }
}
