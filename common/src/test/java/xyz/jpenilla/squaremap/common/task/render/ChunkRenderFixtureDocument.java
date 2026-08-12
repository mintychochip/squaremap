package xyz.jpenilla.squaremap.common.task.render;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonPrimitive;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.Map;
import net.minecraft.world.level.ChunkPos;
import xyz.jpenilla.squaremap.common.util.chunksnapshot.ChunkSnapshot;

final class ChunkRenderFixtureDocument {
    private static final String REGISTRY_PATH = "registry/registry.bin";
    private static final String[] MALFORMED_IDS = {"unknown-descriptor", "heightmap", "palette", "bounds", "crc", "length", "section-count", "section-y", "packed-width", "trailing-bits", "duplicate-palette", "protobuf-body", "generation", "neighbor-coordinate"};
    private static final String[] MALFORMED_CLASSIFIERS = {"UnknownDescriptor(block)", "HeightOutOfRange", "PaletteLength(block)", "VerticalBounds", "Crc", "DecompressedLength", "SectionCountMismatch", "SectionYOutOfRange", "InvalidPacking(block length)", "InvalidPacking(block index)", "DuplicatePalette(block)", "Protobuf", "RegistryMismatch", "CoordinateMismatch"};
    private ChunkRenderFixtureDocument() {}
    record Projection(JsonObject document, Map<Path, byte[]> files) {}

    static Projection build(final ChunkRenderFixtureCatalog catalog) {
        final JsonObject document = new JsonObject(); document.addProperty("schema_version", 2); document.addProperty("generator", "chunk-render-java-v2"); document.addProperty("biome_zoom_seed", catalog.biomeZoomSeed());
        final Map<Path, byte[]> files = new LinkedHashMap<>();
        files.put(Path.of(REGISTRY_PATH), catalog.cases().values().iterator().next().registry().toByteArray());
        final JsonArray valid = new JsonArray();
        for (final var entry : catalog.cases().entrySet()) {
            final String id = entry.getKey(); final var model = entry.getValue();
            final String centerPath = path("chunks/" + id + "/center.bin"); files.put(Path.of(centerPath), catalog.wireSnapshot(model.center()).toByteArray());
            final JsonObject row = new JsonObject(); row.addProperty("id", id); row.addProperty("registry", REGISTRY_PATH); row.addProperty("chunk", centerPath);
            row.add("north", side(files, catalog, model.neighbors().get(new ChunkPos(0, -1)), "chunks/" + id + "/north.bin")); row.add("south", side(files, catalog, model.neighbors().get(new ChunkPos(0, 1)), "chunks/" + id + "/south.bin"));
            final JsonArray sources = new JsonArray(); for (final var source : model.biomeSources().entrySet()) { final String sourcePath = "chunks/" + id + "/biome-" + source.getKey().x() + "-" + source.getKey().z() + ".bin"; files.put(Path.of(sourcePath), catalog.wireSnapshot(source.getValue()).toByteArray()); final JsonObject sourceRow = new JsonObject(); sourceRow.addProperty("x", source.getKey().x()); sourceRow.addProperty("z", source.getKey().z()); sourceRow.addProperty("path", sourcePath); sources.add(sourceRow); } row.add("biome_sources", sources);
            final var settings = model.settings(); row.addProperty("max_height", settings.maxHeight); row.addProperty("iterate_up", settings.iterateUp); row.addProperty("glass_clear", settings.glassClear); row.addProperty("water_checkerboard", settings.waterCheckerboard); row.addProperty("water_clear", settings.waterClear); row.addProperty("lava_checkerboard", settings.lavaCheckerboard); row.addProperty("biome_enabled", settings.biomeEnabled); row.addProperty("biome_blend", settings.biomeBlend); row.addProperty("ceiling", model.ceiling()); row.addProperty("invisible_id", model.invisibleId()); row.addProperty("iterate_up_base_id", model.iterateUpBaseId());
            final var result = ChunkRenderFixtureCatalog.engine(catalog, model, settings.biomeBlend).renderChunkResult(model.neighbors().get(new ChunkPos(0, -1)), model.center(), model.neighbors().get(new ChunkPos(0, 1))); row.add("pixels", unsigned(result.pixels())); row.add("south_edge", signed(result.southEdge()));
            final JsonArray samples = new JsonArray(); if (id.equals("biome-grass-radius-0")) for (int x = 0; x < 16; x++) for (int z = 0; z < 16; z++) addGrassSample(samples, catalog, model, x, 15, z); else if (id.equals("biome-blend-radius-3") || id.equals("biome-blend-cross-boundary")) for (int x = -3; x <= 17; x++) for (int z = -3; z <= 17; z++) addGrassSample(samples, catalog, model, x, 15, z); row.add("grass_samples", samples); valid.add(row);
        }
        document.add("valid", valid); final JsonArray malformed = new JsonArray(); final var mutations = ChunkRenderMalformedFactory.build(catalog);
        for (final String id : MALFORMED_IDS) { final var mutation = mutations.get("malformed-" + id); final String path = "malformed/" + id + ".bin"; files.put(Path.of(path), mutation.bytes()); final JsonObject row = new JsonObject(); row.addProperty("id", "malformed-" + id); row.addProperty("path", path); row.addProperty("mutation", mutation.changedField()); row.addProperty("classifier", mutation.classifier()); malformed.add(row); } document.add("malformed", malformed);
        return new Projection(document, files);
    }
    private static String path(final String value) { return value; }
    private static com.google.gson.JsonElement side(final Map<Path, byte[]> files, final ChunkRenderFixtureCatalog catalog, final ChunkSnapshot snapshot, final String path) { return snapshot == null ? com.google.gson.JsonNull.INSTANCE : new JsonPrimitive(path(files, catalog, snapshot, path)); }
    private static String path(final Map<Path, byte[]> files, final ChunkRenderFixtureCatalog catalog, final ChunkSnapshot snapshot, final String path) { files.put(Path.of(path), catalog.wireSnapshot(snapshot).toByteArray()); return path; }
    private static void addGrassSample(final JsonArray samples, final ChunkRenderFixtureCatalog catalog, final ChunkRenderFixtureCatalog.CaseModel model, final int x, final int y, final int z) {
        final var selected = catalog.selectedBiome(model, x, y, z);
        final JsonObject sample = new JsonObject();
        sample.addProperty("block_x", x); sample.addProperty("block_y", y); sample.addProperty("block_z", z); sample.addProperty("biome_id", catalog.descriptors().biomeId(selected));
        sample.addProperty("resolved_grass_argb", Integer.toUnsignedLong(selected.value().getSpecialEffects().grassColorModifier().modifyColor(x, z, catalog.colorData().grassColor(selected.value()))));
        samples.add(sample);
    }
    private static JsonArray unsigned(final int[] values) { final JsonArray out = new JsonArray(); for (final int value : values) out.add(Integer.toUnsignedLong(value)); return out; }
    private static JsonArray signed(final int[] values) { final JsonArray out = new JsonArray(); for (final int value : values) out.add(value); return out; }
}
