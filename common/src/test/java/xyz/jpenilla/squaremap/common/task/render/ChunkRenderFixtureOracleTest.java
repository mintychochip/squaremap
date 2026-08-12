package xyz.jpenilla.squaremap.common.task.render;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashSet;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;
import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

class ChunkRenderFixtureOracleTest {
    static final Path FIXTURE = Path.of(System.getProperty("squaremap.task11.root"), "testdata/bridge/v2/render/manifest.json");
    private static final List<String> VALID_IDS = List.of("flat-solid", "empty-chunk", "north-height-discontinuity", "south-height-discontinuity", "missing-north", "missing-south", "iterate-down", "iterate-up", "ceiling-iterate-down", "ceiling-iterate-up", "max-height-clipped", "water-depth-cap", "water-clear", "water-checkerboard", "lava-depth-checkerboard", "clear-glass", "stained-glass", "glass-disabled", "invisible-block", "biome-off", "biome-grass-radius-0", "biome-foliage-radius-0", "biome-water-radius-0", "biome-blend-radius-3", "biome-blend-cross-boundary", "negative-min-y");
    private static final List<String> MALFORMED_IDS = List.of("malformed-unknown-descriptor", "malformed-heightmap", "malformed-palette", "malformed-bounds", "malformed-crc", "malformed-length", "malformed-section-count", "malformed-section-y", "malformed-packed-width", "malformed-trailing-bits", "malformed-duplicate-palette", "malformed-protobuf-body", "malformed-generation", "malformed-neighbor-coordinate");
    @Test void manifestIsExactAndCompareOnly() throws Exception {
        final Path root = Path.of(System.getProperty("squaremap.task11.root")).toAbsolutePath().normalize(); final JsonObject expected = JsonParser.parseString(Files.readString(FIXTURE)).getAsJsonObject(); final ChunkRenderFixtureDocument.Projection projection = ChunkRenderFixtureDocument.build(ChunkRenderFixtureCatalog.create());
        assertEquals(2, expected.get("schema_version").getAsInt()); assertEquals("chunk-render-java-v2", expected.get("generator").getAsString()); assertEquals(24301L, expected.get("biome_zoom_seed").getAsLong()); assertEquals(26, expected.getAsJsonArray("valid").size()); assertEquals(14, expected.getAsJsonArray("malformed").size()); assertEquals(expected, projection.document()); final Set<Path> references = new HashSet<>(); for (final var rowValue : expected.getAsJsonArray("valid")) collect(rowValue.getAsJsonObject(), references); for (final var rowValue : expected.getAsJsonArray("malformed")) references.add(Path.of(rowValue.getAsJsonObject().get("path").getAsString())); assertEquals(projection.files().keySet(), references); assertProjectionFilesMatch(root, FIXTURE, projection);
        final List<String> valid = new java.util.ArrayList<>(); for (final var row : expected.getAsJsonArray("valid")) { final JsonObject object = row.getAsJsonObject(); assertTrue(valid.add(object.get("id").getAsString())); assertEquals(256, object.getAsJsonArray("pixels").size()); assertEquals(16, object.getAsJsonArray("south_edge").size()); final String id = object.get("id").getAsString(); final int expectedSamples = id.equals("biome-grass-radius-0") ? 256 : id.equals("biome-blend-radius-3") || id.equals("biome-blend-cross-boundary") ? 441 : 0; assertEquals(expectedSamples, object.getAsJsonArray("grass_samples").size()); for (final var pixel : object.getAsJsonArray("pixels")) assertTrue(pixel.getAsLong() >= 0 && pixel.getAsLong() <= 0xffff_ffffL); for (final var edge : object.getAsJsonArray("south_edge")) assertTrue(edge.getAsInt() >= Integer.MIN_VALUE && edge.getAsInt() <= Integer.MAX_VALUE); } assertEquals(VALID_IDS, valid);
        final List<String> malformed = new java.util.ArrayList<>();
        final Map<String, String> classifiers = new HashMap<>();
        classifiers.put("malformed-unknown-descriptor", "UnknownDescriptor(block)");
        classifiers.put("malformed-heightmap", "HeightOutOfRange");
        classifiers.put("malformed-palette", "PaletteLength(block)");
        classifiers.put("malformed-bounds", "VerticalBounds");
        classifiers.put("malformed-crc", "Crc");
        classifiers.put("malformed-length", "DecompressedLength");
        classifiers.put("malformed-section-count", "SectionCountMismatch");
        classifiers.put("malformed-section-y", "SectionYOutOfRange");
        classifiers.put("malformed-packed-width", "InvalidPacking(block length)");
        classifiers.put("malformed-trailing-bits", "InvalidPacking(block index)");
        classifiers.put("malformed-duplicate-palette", "DuplicatePalette(block)");
        classifiers.put("malformed-protobuf-body", "Protobuf");
        classifiers.put("malformed-generation", "RegistryMismatch");
        classifiers.put("malformed-neighbor-coordinate", "CoordinateMismatch");
        for (final var row : expected.getAsJsonArray("malformed")) {
            final JsonObject object = row.getAsJsonObject();
            final String id = object.get("id").getAsString();
            assertTrue(malformed.add(id)); assertEquals(classifiers.get(id), object.get("classifier").getAsString());
            assertTrue(object.has("path")); assertTrue(object.has("mutation"));
        }
        assertEquals(MALFORMED_IDS, malformed);
    }
    @Test void sidecarCorruptionFailsCanonicalByteComparison(@TempDir final Path temp) throws Exception {
        final ChunkRenderFixtureDocument.Projection projection = ChunkRenderFixtureDocument.build(ChunkRenderFixtureCatalog.create());
        final Path renderRoot = temp.resolve("render");
        final Path manifest = renderRoot.resolve("manifest.json");
        Files.createDirectories(renderRoot);
        Files.writeString(manifest, projection.document().toString());
        for (final var entry : projection.files().entrySet()) {
            final Path file = renderRoot.resolve(entry.getKey());
            Files.createDirectories(file.getParent());
            Files.write(file, entry.getValue());
        }
        assertProjectionFilesMatch(renderRoot, manifest, projection);
        final Path corruptedPath = renderRoot.resolve(projection.files().keySet().stream().filter(path -> projection.files().get(path).length > 0).findFirst().orElseThrow());
        final byte[] corrupted = Files.readAllBytes(corruptedPath);
        corrupted[0] ^= 1;
        Files.write(corruptedPath, corrupted);
        assertThrows(AssertionError.class, () -> assertProjectionFilesMatch(renderRoot, manifest, projection));
    }
    @Test void malformedFactoryProducesEveryAuditedMutation() {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        final Map<String, ChunkRenderMalformedFactory.Mutation> mutations = ChunkRenderMalformedFactory.build(catalog);
        assertEquals(MALFORMED_IDS, List.copyOf(mutations.keySet()));
        for (final String id : MALFORMED_IDS) {
            final ChunkRenderMalformedFactory.Mutation mutation = mutations.get(id);
            final xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshot source = id.equals("malformed-neighbor-coordinate")
                ? catalog.wireSnapshot(catalog.cases().get("north-height-discontinuity").neighbors().get(new net.minecraft.world.level.ChunkPos(0, -1)))
                : catalog.cases().get(mutation.baseId()).wireSnapshot();
            assertEquals(expectedClassifier(id), mutation.classifier(), id);
            assertTrue(!mutation.changedField().isBlank(), id);
        }
    }
    private static void assertProjectionFilesMatch(final Path root, final Path manifest, final ChunkRenderFixtureDocument.Projection projection) throws Exception {
        for (final var entry : projection.files().entrySet()) {
            final Path resolved = safe(root, manifest, entry.getKey().toString());
            assertTrue(Files.isRegularFile(resolved));
            assertArrayEquals(entry.getValue(), Files.readAllBytes(resolved));
        }
    }
    private static String expectedClassifier(final String id) {
        return switch (id) {
            case "malformed-unknown-descriptor" -> "UnknownDescriptor(block)";
            case "malformed-heightmap" -> "HeightOutOfRange";
            case "malformed-palette" -> "PaletteLength(block)";
            case "malformed-bounds" -> "VerticalBounds";
            case "malformed-crc" -> "Crc";
            case "malformed-length" -> "DecompressedLength";
            case "malformed-section-count" -> "SectionCountMismatch";
            case "malformed-section-y" -> "SectionYOutOfRange";
            case "malformed-packed-width" -> "InvalidPacking(block length)";
            case "malformed-trailing-bits" -> "InvalidPacking(block index)";
            case "malformed-duplicate-palette" -> "DuplicatePalette(block)";
            case "malformed-protobuf-body" -> "Protobuf";
            case "malformed-generation" -> "RegistryMismatch";
            case "malformed-neighbor-coordinate" -> "CoordinateMismatch";
            default -> throw new AssertionError(id);
        };
    }
    private static void collect(final JsonObject row, final Set<Path> refs) { refs.add(Path.of(row.get("registry").getAsString())); refs.add(Path.of(row.get("chunk").getAsString())); for (final String side : new String[]{"north", "south"}) if (!row.get(side).isJsonNull()) refs.add(Path.of(row.get(side).getAsString())); for (final var source : row.getAsJsonArray("biome_sources")) refs.add(Path.of(source.getAsJsonObject().get("path").getAsString())); }
    private static Path safe(final Path root, final Path manifest, final String relative) throws Exception { final Path path = Path.of(relative); assertTrue(!path.isAbsolute()); for (final Path component : path) assertTrue(!component.toString().equals("..")); final Path candidate = manifest.getParent().resolve(path).normalize(); assertTrue(candidate.startsWith(root)); final Path canonicalRoot = root.toRealPath(); final Path canonical = candidate.toRealPath(); assertTrue(canonical.startsWith(canonicalRoot)); assertTrue(Files.isRegularFile(canonical)); return canonical; }
}
