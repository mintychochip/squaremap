package xyz.jpenilla.squaremap.common.visibilitylimit;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.common.data.RegionCoordinate;
import xyz.jpenilla.squaremap.common.util.ColorBlender;
import xyz.jpenilla.squaremap.common.util.Colors;
import xyz.jpenilla.squaremap.common.util.Numbers;
import xyz.jpenilla.squaremap.common.util.RenderPrimitiveEngine;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

/** Compare-only Java oracle for Minecraft-independent render primitives. */
class RenderPrimitiveOracleTest {
    static final Path FIXTURE = Path.of("..", "testdata", "bridge", "v1", "render", "primitives.json");
    private static final int[] BOUNDARIES = {
        -513, -512, -511, -33, -32, -31, -17, -16, -15, -1, 0, 1, 15, 16, 17, 31, 32, 33, 511, 512, 513
    };

    @Test
    void productionOracleMatchesCheckedInFixture() throws Exception {
        assertEquals(JsonParser.parseString(Files.readString(FIXTURE)), document());
    }

    @Test
    void worldBorderFactoryRejectsOverflowAndNonFiniteRuntimeValues() {
        assertThrows(IllegalArgumentException.class, () -> RenderPrimitiveEngine.worldBorder(Integer.MAX_VALUE, 0, 1));
        assertThrows(IllegalArgumentException.class, () -> RenderPrimitiveEngine.worldBorder(Integer.MIN_VALUE, 0, 1));
        assertThrows(IllegalArgumentException.class, () -> RenderPrimitiveEngine.worldBorder(0, Integer.MAX_VALUE, 1));
        assertThrows(IllegalArgumentException.class, () -> RenderPrimitiveEngine.worldBorder(0, Integer.MIN_VALUE, 1));
        assertThrows(IllegalArgumentException.class, () -> RenderPrimitiveEngine.fromRuntime(Double.NaN, 0, 1));
        assertThrows(IllegalArgumentException.class, () -> RenderPrimitiveEngine.fromRuntime(Double.POSITIVE_INFINITY, 0, 1));
        assertThrows(IllegalArgumentException.class, () -> RenderPrimitiveEngine.fromRuntime(0, Double.NEGATIVE_INFINITY, 1));
        assertThrows(IllegalArgumentException.class, () -> RenderPrimitiveEngine.fromRuntime(0, 0, Double.NaN));
        assertThrows(IllegalArgumentException.class, () -> RenderPrimitiveEngine.fromRuntime(0, 0, Double.POSITIVE_INFINITY));
        assertThrows(IllegalArgumentException.class, () -> RenderPrimitiveEngine.fromRuntime(0, 0, -1));
        assertThrows(IllegalArgumentException.class, () -> RenderPrimitiveEngine.fromRuntime((double) Integer.MAX_VALUE + 1, 0, 0));
    }

    static JsonObject document() throws Exception {
        final JsonObject root = new JsonObject();
        root.addProperty("schema", 1);
        root.addProperty("generator", "render-primitives-java-v1");
        root.add("coordinates", coordinates());
        root.add("tiles", tiles());
        root.add("visibility", visibility());
        root.add("colors", colors());
        root.add("color_invalid", colorInvalid());
        root.add("fluid_classification", fluidClassification());
        return root;
    }

    private static JsonObject coordinates() {
        final JsonObject out = new JsonObject();
        final JsonArray conversions = new JsonArray();
        for (int i = 0; i < BOUNDARIES.length; i++) {
            final int n = BOUNDARIES[i];
            final JsonObject row = new JsonObject();
            row.addProperty("id", "c-" + i);
            row.addProperty("input", n);
            final JsonArray tuple = new JsonArray();
            tuple.add(Numbers.regionToBlock(n));
            tuple.add(Numbers.blockToRegion(n));
            tuple.add(Numbers.regionToChunk(n));
            tuple.add(Numbers.chunkToRegion(n));
            tuple.add(Numbers.chunkToBlock(n));
            tuple.add(Numbers.blockToChunk(n));
            row.add("tuple", tuple);
            conversions.add(row);
        }
        out.add("conversions", conversions);

        final JsonArray reverse = new JsonArray();
        reverse(reverse, "region_to_block", -4_194_304, "accepted");
        reverse(reverse, "region_to_block", 4_194_303, "accepted");
        reverse(reverse, "region_to_block", -4_194_305, "overflow");
        reverse(reverse, "region_to_block", 4_194_304, "overflow");
        reverse(reverse, "region_to_chunk", -67_108_864, "accepted");
        reverse(reverse, "region_to_chunk", 67_108_863, "accepted");
        reverse(reverse, "region_to_chunk", -67_108_865, "overflow");
        reverse(reverse, "region_to_chunk", 67_108_864, "overflow");
        reverse(reverse, "chunk_to_block", -134_217_728, "accepted");
        reverse(reverse, "chunk_to_block", 134_217_727, "accepted");
        reverse(reverse, "chunk_to_block", -134_217_729, "overflow");
        reverse(reverse, "chunk_to_block", 134_217_728, "overflow");
        out.add("reverse", reverse);

        final JsonArray malformed = new JsonArray();
        for (final String function : List.of("region_to_block", "region_to_chunk", "chunk_to_block")) {
            malformed(malformed, function, Integer.MIN_VALUE);
            malformed(malformed, function, Integer.MAX_VALUE);
        }
        out.add("malformed", malformed);
        return out;
    }

    private static void reverse(final JsonArray rows, final String function, final int input, final String status) {
        final JsonObject row = new JsonObject();
        row.addProperty("function", function);
        row.addProperty("input", input);
        row.addProperty("status", status);
        if (status.equals("accepted")) {
            row.addProperty("java_result", switch (function) {
                case "region_to_block" -> Numbers.regionToBlock(input);
                case "region_to_chunk" -> Numbers.regionToChunk(input);
                case "chunk_to_block" -> Numbers.chunkToBlock(input);
                default -> throw new IllegalArgumentException(function);
            });
        }
        rows.add(row);
    }

    private static void malformed(final JsonArray rows, final String function, final int input) {
        final JsonObject row = new JsonObject();
        row.addProperty("function", function);
        row.addProperty("input", input);
        final int result = switch (function) {
            case "region_to_block" -> Numbers.regionToBlock(input);
            case "region_to_chunk" -> Numbers.regionToChunk(input);
            case "chunk_to_block" -> Numbers.chunkToBlock(input);
            default -> throw new IllegalArgumentException(function);
        };
        row.addProperty("java_result", result);
        row.addProperty("rust_error", "Overflow");
        rows.add(row);
    }

    private static JsonObject tiles() {
        final JsonObject out = new JsonObject();
        final JsonArray rows = new JsonArray();
        int id = 0;
        for (int zoom = 0; zoom <= 3; zoom++) {
            final int step = 1 << zoom;
            final int size = 512 / step;
            for (final int n : BOUNDARIES) {
                final JsonObject row = new JsonObject();
                row.addProperty("id", "t-" + id++);
                row.addProperty("region_x", n);
                row.addProperty("region_z", n);
                row.addProperty("zoom", zoom);
                row.addProperty("max_zoom", 3);
                final JsonObject tile = new JsonObject();
                tile.addProperty("level", 3 - zoom);
                tile.addProperty("x", (int) Math.floor((double) n / step));
                tile.addProperty("z", (int) Math.floor((double) n / step));
                tile.addProperty("origin_x", Math.floorMod(n * size, 512));
                tile.addProperty("origin_z", Math.floorMod(n * size, 512));
                row.add("tile", tile);
                rows.add(row);
            }
        }
        out.add("rows", rows);
        final JsonArray invalid = new JsonArray();
        invalidTile(invalid, 4, 3);
        invalidTile(invalid, 10, 10);
        invalidTile(invalid, 0, 10);
        out.add("invalid", invalid);
        return out;
    }

    private static void invalidTile(final JsonArray rows, final int zoom, final int maxZoom) {
        final JsonObject row = new JsonObject();
        row.addProperty("zoom", zoom);
        row.addProperty("max_zoom", maxZoom);
        row.addProperty("rust_error", "InvalidZoom");
        rows.add(row);
    }

    private static JsonObject visibility() {
        final JsonObject out = new JsonObject();
        final JsonArray shapes = new JsonArray();
        final RenderPrimitiveEngine.Rectangle rectangle = RenderPrimitiveEngine.rectangle(-17, -17, 17, 17);
        addShape(shapes, "rectangle", "rectangle", new int[]{-17, -17, 17, 17},
            new int[][]{{-18, 0}, {-17, -17}, {17, 17}, {18, 17}},
            new int[][]{{-3, 0}, {-2, 0}, {1, 0}, {2, 0}},
            new int[][]{{-2, 0}, {-1, -1}, {0, 0}, {1, 0}},
            (x, z) -> RenderPrimitiveEngine.rectangleBlock(rectangle, x, z),
            (x, z) -> RenderPrimitiveEngine.rectangleChunk(rectangle, x, z),
            (x, z) -> RenderPrimitiveEngine.rectangleRegion(rectangle, x, z),
            (x, z) -> RenderPrimitiveEngine.rectangleCount(rectangle, x, z));

        final RenderPrimitiveEngine.Rectangle aligned = RenderPrimitiveEngine.rectangle(0, 0, 16, 16);
        addShape(shapes, "rectangle-aligned", "rectangle", new int[]{0, 0, 16, 16},
            new int[][]{{0, 0}, {16, 16}, {-1, 0}, {17, 0}},
            new int[][]{{0, 0}, {1, 0}, {0, 1}, {1, 1}, {-1, 0}},
            new int[][]{{0, 0}, {1, 0}, {0, 1}},
            (x, z) -> RenderPrimitiveEngine.rectangleBlock(aligned, x, z),
            (x, z) -> RenderPrimitiveEngine.rectangleChunk(aligned, x, z),
            (x, z) -> RenderPrimitiveEngine.rectangleRegion(aligned, x, z),
            (x, z) -> RenderPrimitiveEngine.rectangleCount(aligned, x, z));

        addCircle(shapes, "circle-1", 1,
            new int[][]{{0, 0}, {1, 0}, {0, 1}, {1, 1}},
            new int[][]{{-1, 0}, {0, 0}, {1, 0}},
            new int[][]{{0, 0}}, new int[][]{{0, 0}});
        addCircle(shapes, "circle-16", 16,
            new int[][]{{-17, 0}, {-16, 0}, {0, 0}, {16, 0}, {17, 0}},
            new int[][]{{-2, 0}, {-1, 0}, {0, 0}, {1, 0}, {2, 0}},
            new int[][]{{-1, 0}, {0, 0}, {1, 0}}, new int[][]{{0, 0}});
        addCircle(shapes, "circle-17", 17,
            new int[][]{{-17, 0}, {-16, 0}, {16, 0}, {17, 0}},
            new int[][]{{-2, 0}, {-1, 0}, {0, 0}, {1, 0}},
            new int[][]{{-1, 0}, {0, 0}, {1, 0}}, new int[][]{{0, 0}});


        addCircle(shapes, "circle-512", 512,
            new int[][]{{-512, 0}, {0, 0}, {512, 0}},
            new int[][]{{-1, 0}, {0, 0}, {1, 0}},
            new int[][]{{-1, 0}, {0, 0}, {1, 0}}, new int[][]{{0, 0}});
        final RenderPrimitiveEngine.Polygon triangle = RenderPrimitiveEngine.polygon(new int[][]{{0, 0}, {10, 0}, {0, 10}});
        addShape(shapes, "polygon-triangle", "polygon", new int[]{0, 0, 10, 0, 0, 10},
            new int[][]{{0, 0}, {5, 0}, {0, 5}, {5, 5}, {10, 0}, {0, 10}, {-1, 0}},
            new int[][]{{-1, 0}, {0, 0}, {1, 0}},
            new int[][]{{-1, 0}, {0, 0}, {1, 0}},
            (x, z) -> RenderPrimitiveEngine.polygonBlock(triangle, x, z),
            (x, z) -> RenderPrimitiveEngine.polygonChunk(triangle, x, z),
            (x, z) -> RenderPrimitiveEngine.polygonRegion(triangle, x, z),
            (x, z) -> RenderPrimitiveEngine.polygonCount(triangle, x, z));
        final RenderPrimitiveEngine.Polygon reversed = RenderPrimitiveEngine.polygon(new int[][]{{0, 10}, {10, 0}, {0, 0}});
        addShape(shapes, "polygon-reversed", "polygon", new int[]{0, 10, 10, 0, 0, 0},
            new int[][]{{0, 0}, {5, 0}, {0, 5}, {5, 5}, {10, 0}, {0, 10}},
            new int[][]{{0, 0}}, new int[][]{{0, 0}},
            (x, z) -> RenderPrimitiveEngine.polygonBlock(reversed, x, z),
            (x, z) -> RenderPrimitiveEngine.polygonChunk(reversed, x, z),
            (x, z) -> RenderPrimitiveEngine.polygonRegion(reversed, x, z),
            (x, z) -> RenderPrimitiveEngine.polygonCount(reversed, x, z));
        final RenderPrimitiveEngine.Polygon duplicate = RenderPrimitiveEngine.polygon(new int[][]{{0, 0}, {10, 0}, {10, 10}, {0, 10}, {0, 0}});
        addShape(shapes, "polygon-duplicate", "polygon", new int[]{0, 0, 10, 0, 10, 10, 0, 10, 0, 0},
            new int[][]{{0, 0}, {5, 5}, {9, 9}, {10, 0}, {-1, 0}}, new int[][]{{0, 0}, {1, 0}}, new int[][]{{0, 0}},
            (x, z) -> RenderPrimitiveEngine.polygonBlock(duplicate, x, z), (x, z) -> RenderPrimitiveEngine.polygonChunk(duplicate, x, z),
            (x, z) -> RenderPrimitiveEngine.polygonRegion(duplicate, x, z), (x, z) -> RenderPrimitiveEngine.polygonCount(duplicate, x, z));
        final RenderPrimitiveEngine.Polygon collinear = RenderPrimitiveEngine.polygon(new int[][]{{0, 0}, {5, 0}, {10, 0}, {10, 10}, {0, 10}});
        addShape(shapes, "polygon-collinear", "polygon", new int[]{0, 0, 5, 0, 10, 0, 10, 10, 0, 10},
            new int[][]{{0, 0}, {5, 0}, {5, 5}, {9, 9}, {10, 0}}, new int[][]{{0, 0}, {1, 0}}, new int[][]{{0, 0}},
            (x, z) -> RenderPrimitiveEngine.polygonBlock(collinear, x, z), (x, z) -> RenderPrimitiveEngine.polygonChunk(collinear, x, z),
            (x, z) -> RenderPrimitiveEngine.polygonRegion(collinear, x, z), (x, z) -> RenderPrimitiveEngine.polygonCount(collinear, x, z));
        addBorder(shapes, "world-border-32", 0, 0, 16,
            new int[][]{{-17, 0}, {-16, 0}, {-1, 0}, {0, 0}, {15, 0}, {16, 0}},
            new int[][]{{-2, 0}, {-1, 0}, {0, 0}, {1, 0}, {2, 0}},
            new int[][]{{-1, 0}, {0, 0}, {1, 0}}, new int[][]{{0, 0}});
        addBorder(shapes, "world-border-33", 16, -16, 17,
            new int[][]{{-1, 0}, {0, 0}, {32, -16}, {33, -16}},
            new int[][]{{-2, -2}, {-1, -2}, {0, -1}, {1, -1}, {2, -1}},
            new int[][]{{-1, -1}, {0, -1}, {1, -1}}, new int[][]{{0, -1}});
        addBorder(shapes, "world-border-zero", -16, 16, 0,
            new int[][]{{-16, 16}, {-15, 16}}, new int[][]{{-1, 1}, {0, 1}}, new int[][]{{-1, 0}, {0, 0}}, new int[][]{{0, 0}});
        out.add("shapes", shapes);
        final JsonArray degenerate = new JsonArray();
        for (int pointCount = 0; pointCount <= 2; pointCount++) {
            final int[][] points = new int[pointCount][2];
            final int[] params = new int[pointCount * 2];
            for (int i = 0; i < pointCount; i++) {
                points[i][0] = i;
                points[i][1] = i;
                params[i * 2] = i;
                params[i * 2 + 1] = i;
            }
            final RenderPrimitiveEngine.Polygon polygon = RenderPrimitiveEngine.polygon(points);
            addShape(degenerate, "polygon-degenerate-" + pointCount, "polygon", params,
                new int[][]{{0, 0}, {1, 1}, {-1, 0}},
                new int[][]{{0, 0}, {-1, -1}}, new int[][]{{0, 0}},
                (x, z) -> RenderPrimitiveEngine.polygonBlock(polygon, x, z),
                (x, z) -> RenderPrimitiveEngine.polygonChunk(polygon, x, z),
                (x, z) -> RenderPrimitiveEngine.polygonRegion(polygon, x, z),
                (x, z) -> RenderPrimitiveEngine.polygonCount(polygon, x, z));
        }
        out.add("degenerate", degenerate);

        final JsonObject empty = new JsonObject();
        empty.addProperty("contains_block", true);
        empty.addProperty("contains_chunk", true);
        empty.addProperty("contains_region", true);
        empty.addProperty("count_chunks", 1024);
        final JsonArray runtime = new JsonArray();
        addRuntimeBorder(runtime, "runtime-32", 16.9, -16.9, 32.0);
        addRuntimeBorder(runtime, "runtime-33-negative", -16.9, 16.9, 33.0);
        addRuntimeBorder(runtime, "runtime-zero", 0.0, 0.0, 0.0);
        addRuntimeBorder(runtime, "runtime-33", 0.0, 0.0, 33.0);
        out.add("runtime", runtime);
        out.add("empty", empty);
        final JsonArray invalid = new JsonArray();
        invalidShape(invalid, "rectangle", "InvalidRectangle");
        invalidShape(invalid, "circle", "InvalidCircle");
        invalidShape(invalid, "polygon", "InvalidPolygon");
        invalidShape(invalid, "world_border", "WorldBorderRuntimeRequired");
        invalidBorder(invalid, "wire_extra_fields", "InvalidWorldBorder");
        invalidBorder(invalid, "runtime_overflow", "QueryOverflow");
        invalidBorder(invalid, "runtime_nan", "QueryOverflow");
        out.add("invalid", invalid);
        return out;
    }

    private static void addCircle(final JsonArray shapes, final String id, final int radius, final int[][] blocks,
                                  final int[][] chunks, final int[][] regions, final int[][] counts) {
        final RenderPrimitiveEngine.Circle circle = RenderPrimitiveEngine.circle(0, 0, radius);
        addShape(shapes, id, "circle", new int[]{0, 0, radius}, blocks, chunks, regions,
            (x, z) -> RenderPrimitiveEngine.circleBlock(circle, x, z),
            (x, z) -> RenderPrimitiveEngine.circleChunk(circle, x, z),
            (x, z) -> RenderPrimitiveEngine.circleRegion(circle, x, z),
            (x, z) -> RenderPrimitiveEngine.circleCount(circle, x, z));
    }

    private static void addBorder(final JsonArray shapes, final String id, final int centerX, final int centerZ, final int radius,
                                  final int[][] blocks, final int[][] chunks, final int[][] regions, final int[][] counts) {
        final RenderPrimitiveEngine.WorldBorder border = RenderPrimitiveEngine.worldBorder(centerX, centerZ, radius);
        addShape(shapes, id, "world_border", new int[]{centerX, centerZ, radius}, blocks, chunks, regions,
            (x, z) -> RenderPrimitiveEngine.worldBorderBlock(border, x, z),
            (x, z) -> RenderPrimitiveEngine.worldBorderChunk(border, x, z),
            (x, z) -> RenderPrimitiveEngine.worldBorderRegion(border, x, z),
            (x, z) -> RenderPrimitiveEngine.worldBorderCount(border, x, z));
    }

    private interface Predicate { boolean test(int x, int z); }
    private interface Counter { int count(int x, int z); }

    private static void addShape(final JsonArray shapes, final String id, final String kind, final int[] params,
                                 final int[][] blocks, final int[][] chunks, final int[][] regions, final Predicate block,
                                 final Predicate chunk, final Predicate region, final Counter count) {
        final JsonObject row = new JsonObject();
        row.addProperty("id", id);
        row.addProperty("kind", kind);
        final JsonArray p = new JsonArray();
        for (final int value : params) p.add(value);
        row.add("params", p);
        row.add("blocks", queries(blocks, block));
        row.add("chunks", queries(chunks, chunk));
        row.add("regions", queries(regions, region));
        final JsonArray counts = new JsonArray();
        for (final int[] query : regions) {
            final JsonObject value = new JsonObject();
            value.addProperty("x", query[0]);
            value.addProperty("z", query[1]);
            value.addProperty("result", count.count(query[0], query[1]));
            counts.add(value);
        }
        row.add("count_chunks", counts);
        shapes.add(row);
    }

    private static void addRuntimeBorder(final JsonArray rows, final String id, final double centerX, final double centerZ, final double size) {
        final RenderPrimitiveEngine.WorldBorder border = RenderPrimitiveEngine.fromRuntime(centerX, centerZ, size);
        final JsonObject row = new JsonObject();
        row.addProperty("id", id);
        row.addProperty("center_x", centerX);
        row.addProperty("center_z", centerZ);
        row.addProperty("size", size);
        final JsonObject resolved = new JsonObject();
        resolved.addProperty("center_x", border.centerX());
        resolved.addProperty("center_z", border.centerZ());
        resolved.addProperty("radius", border.radius());
        row.add("resolved", resolved);
        row.add("blocks", queries(new int[][]{{border.minX() - 1, border.minZ()}, {border.minX(), border.minZ()}, {border.maxX() - 1, border.maxZ() - 1}, {border.maxX(), border.maxZ()}},
            (x, z) -> RenderPrimitiveEngine.worldBorderBlock(border, x, z)));
        row.add("chunks", queries(new int[][]{{Numbers.blockToChunk(border.minX()), Numbers.blockToChunk(border.minZ())}, {Numbers.blockToChunk(border.maxX()), Numbers.blockToChunk(border.maxZ())}},
            (x, z) -> RenderPrimitiveEngine.worldBorderChunk(border, x, z)));
        row.add("regions", queries(new int[][]{{Numbers.blockToRegion(border.minX()), Numbers.blockToRegion(border.minZ())}, {Numbers.blockToRegion(border.maxX()), Numbers.blockToRegion(border.maxZ())}},
            (x, z) -> RenderPrimitiveEngine.worldBorderRegion(border, x, z)));
        final JsonArray counts = new JsonArray();
        final JsonObject count = new JsonObject();
        count.addProperty("x", 0); count.addProperty("z", 0);
        count.addProperty("result", RenderPrimitiveEngine.worldBorderCount(border, 0, 0));
        counts.add(count); row.add("count_chunks", counts);
        rows.add(row);
    }
    private static JsonArray fluidClassification() {
        final JsonArray rows = new JsonArray();
        classifyFluid(rows, 0xFFFFFFFF, false, false);
        classifyFluid(rows, 0x00FFFFFF, false, false);
        classifyFluid(rows, 0xFFFFFFFF, true, false);
        classifyFluid(rows, 0xFFFFFFFF, false, true);
        return rows;
    }

    private static void classifyFluid(final JsonArray rows, final int color, final boolean nativeWater, final boolean nativeLava) {
        final JsonObject row = new JsonObject();
        row.addProperty("color", color);
        row.addProperty("native_water", nativeWater);
        row.addProperty("native_lava", nativeLava);
        row.addProperty("result", RenderPrimitiveEngine.classifyUnknownFluid(color, nativeWater, nativeLava).name().toLowerCase(java.util.Locale.ROOT));
        rows.add(row);
    }


    private static JsonArray queries(final int[][] queries, final Predicate predicate) {
        final JsonArray rows = new JsonArray();
        for (final int[] query : queries) {
            final JsonObject row = new JsonObject();
            row.addProperty("x", query[0]);
            row.addProperty("z", query[1]);
            row.addProperty("result", predicate.test(query[0], query[1]));
            rows.add(row);
        }
        return rows;
    }

    private static void invalidShape(final JsonArray rows, final String kind, final String error) {
        final JsonObject row = new JsonObject();
        row.addProperty("kind", kind);
        row.addProperty("rust_error", error);
        try {
            switch (kind) {
                case "rectangle" -> RenderPrimitiveEngine.rectangle(1, 1, 1, 2);
                case "circle" -> RenderPrimitiveEngine.circle(0, 0, 0);
                case "polygon" -> RenderPrimitiveEngine.polygon(new int[][]{{0, 0}, {1, 1}});
                case "world_border" -> throw new IllegalArgumentException("wire requires runtime");
                default -> throw new IllegalArgumentException(kind);
            }
            row.addProperty("java_result", "accepted");
        } catch (final RuntimeException ex) {
            row.addProperty("java_exception", ex.getClass().getSimpleName());
        }
        rows.add(row);
    }

    private static void invalidBorder(final JsonArray rows, final String testCase, final String error) {
        final JsonObject row = new JsonObject();
        row.addProperty("kind", "world_border");
        row.addProperty("case", testCase);
        row.addProperty("rust_error", error);
        try {
            switch (testCase) {
                case "runtime_overflow" -> RenderPrimitiveEngine.fromRuntime((double) Integer.MAX_VALUE + 1, 0, 0);
                case "runtime_nan" -> RenderPrimitiveEngine.fromRuntime(Double.NaN, 0, 0);
                case "wire_extra_fields" -> throw new IllegalArgumentException("no Java wire path");
                default -> throw new IllegalArgumentException(testCase);
            }
            row.addProperty("java_result", "accepted");
        } catch (final RuntimeException ex) {
            row.addProperty("java_exception", ex.getClass().getSimpleName());
        }
        rows.add(row);
    }
    private static JsonArray colors() {
        final JsonArray rows = new JsonArray();
        color(rows, "remove_alpha", 0x12345678, Colors.removeAlpha(0x12345678));
        color(rows, "abgr_to_argb", 0x12345678, Colors.abgrToArgb(0x12345678));
        color(rows, "argb_to_rgba", 0x12345678, Colors.argbToRgba(0x12345678));
        color(rows, "rgba_to_argb", 0x12345678, Colors.rgbaToArgb(0x12345678));
        color(rows, "remove_alpha", 0x00FFFFFF, Colors.removeAlpha(0x00FFFFFF));
        color(rows, "remove_alpha", 0xFFFFFFFF, Colors.removeAlpha(0xFFFFFFFF));
        mix(rows, 0x00102030, 0xFF908070, 0.5F);
        mix(rows, 0x12345678, 0x80A0B0C0, 1.0F);
        mix(rows, 0x12345678, 0x80A0B0C0, 0.0F);
        mix(rows, 0x00000001, 0xFFFFFFFF, 0.25F);
        shadeLevel(rows, 0x12345678, 0);
        shadeLevel(rows, 0x12345678, 1);
        shadeLevel(rows, 0x12345678, 2);
        shadeFactor(rows, 0x12345678, 0.5F);
        shadeFactor(rows, 0xFFFFFFFF, 0.85F);
        parity(rows, -1, 0);
        parity(rows, -2, 0);
        terrain(rows, 5, 4, 0x12345678, 0);
        terrain(rows, 5, 4, 0x12345678, 1);
        for (int d = 1; d <= 11; d++) {
            depth(rows, d, 0x12345678, 0);
            depth(rows, d, 0x12345678, 1);
        }
        glass(rows, 0x00102030, 0xFF908070, 0.25F);
        glass(rows, 0xFF102030, 0x00102030, 0.5F);
        final ColorBlender blender = new ColorBlender();
        blender.addColor(0);
        blender.addColor(0xFF010203);
        blender.addColor(0x80A0B0C0);
        values(rows, "average_argb", List.of(0, 0xFF010203, 0x80A0B0C0), blender.result());
        final ColorBlender biomeOne = new ColorBlender();
        biomeOne.addColor(0x20304050);
        values(rows, "water_biome_blend", List.of(0x0F102030, 0x20304050), Colors.mix(0x0F102030, biomeOne.result(), 0.8F));
        final ColorBlender biomeTwo = new ColorBlender();
        biomeTwo.addColor(0x20304050);
        biomeTwo.addColor(0x60402010);
        values(rows, "water_biome_blend", List.of(0x0F102030, 0x20304050, 0x60402010), Colors.mix(0x0F102030, biomeTwo.result(), 0.8F));
        fluid(rows, 3, 0x12345678, 0xFF010203, true, true, false, false, 1);
        fluid(rows, 3, 0x12345678, 0xFF010203, true, false, true, false, 1);
        fluid(rows, 3, 0x12345678, 0xFF010203, false, false, false, true, 1);
        fluid(rows, 3, 0x12112233, 0xFF010203, true, false, false, false, 1);
        return rows;
    }

    private static void color(final JsonArray rows, final String op, final int input, final int result) {
        final JsonObject row = new JsonObject();
        row.addProperty("op", op);
        row.addProperty("input", input);
        row.addProperty("result", hex(result));
        rows.add(row);
    }

    private static void values(final JsonArray rows, final String op, final List<Integer> inputs, final int result) {
        final JsonObject row = new JsonObject();
        row.addProperty("op", op);
        final JsonArray values = new JsonArray();
        inputs.forEach(values::add);
        row.add("values", values);
        row.addProperty("result", hex(result));
        rows.add(row);
    }

    private static void mix(final JsonArray rows, final int c1, final int c2, final float ratio) {
        final JsonObject row = new JsonObject();
        row.addProperty("op", "mix");
        row.addProperty("c1", hex(c1));
        row.addProperty("c2", hex(c2));
        row.addProperty("ratio_bits", Float.floatToRawIntBits(ratio));
        row.addProperty("result", hex(Colors.mix(c1, c2, ratio)));
        rows.add(row);
    }

    private static void glass(final JsonArray rows, final int under, final int glass, final float alpha) {
        final JsonObject row = new JsonObject();
        row.addProperty("op", "glass");
        row.addProperty("under", under);
        row.addProperty("glass", glass);
        row.addProperty("alpha_bits", Float.floatToRawIntBits(alpha));
        row.addProperty("result", hex(RenderPrimitiveEngine.glass(under, glass, alpha)));
        rows.add(row);
    }

    private static void shadeLevel(final JsonArray rows, final int color, final int level) {
        final JsonObject row = new JsonObject();
        row.addProperty("op", "shade_level");
        row.addProperty("color", color);
        row.addProperty("level", level);
        row.addProperty("result", hex(Colors.shade(color, level)));
        rows.add(row);
    }

    private static void shadeFactor(final JsonArray rows, final int color, final float factor) {
        final JsonObject row = new JsonObject();
        row.addProperty("op", "shade_factor");
        row.addProperty("color", color);
        row.addProperty("factor_bits", Float.floatToRawIntBits(factor));
        row.addProperty("result", hex(Colors.shade(color, factor)));
        rows.add(row);
    }

    private static void parity(final JsonArray rows, final int x, final int z) {
        final JsonObject row = new JsonObject();
        row.addProperty("op", "checkerboard_parity");
        row.addProperty("x", x);
        row.addProperty("z", z);
        row.addProperty("result", RenderPrimitiveEngine.parity(x, z));
        rows.add(row);
    }

    private static void terrain(final JsonArray rows, final int current, final int previous, final int color, final int odd) {
        final JsonObject row = new JsonObject();
        row.addProperty("op", "terrain");
        row.addProperty("current", current);
        row.addProperty("previous", previous);
        row.addProperty("color", color);
        row.addProperty("odd", odd);
        row.addProperty("result", hex(RenderPrimitiveEngine.terrain(current, previous, color, odd)));
        rows.add(row);
    }

    private static void depth(final JsonArray rows, final int depth, final int color, final int odd) {
        final JsonObject row = new JsonObject();
        row.addProperty("op", "depth_checkerboard");
        row.addProperty("depth", depth);
        row.addProperty("color", color);
        row.addProperty("odd", odd);
        row.addProperty("result", hex(RenderPrimitiveEngine.depth(depth, color, odd)));
        rows.add(row);
    }

    private static void fluid(final JsonArray rows, final int depth, final int color, final int under,
                              final boolean water, final boolean waterChecker, final boolean waterClear,
                              final boolean lavaChecker, final int odd) {
        final JsonObject row = new JsonObject();
        row.addProperty("op", "fluid");
        row.addProperty("depth", depth);
        row.addProperty("color", color);
        row.addProperty("under", under);
        row.addProperty("water", water);
        row.addProperty("water_checker", waterChecker);
        row.addProperty("water_clear", waterClear);
        row.addProperty("lava_checker", lavaChecker);
        row.addProperty("odd", odd);
        row.addProperty("result", hex(RenderPrimitiveEngine.fluid(depth, color, water, under, waterChecker, waterClear, lavaChecker, odd)));
        rows.add(row);
    }


    private static JsonArray colorInvalid() {
        final JsonArray rows = new JsonArray();
        invalidColor(rows, "mix_nan", "NonFiniteFloat");
        invalidColor(rows, "mix_pos_inf", "NonFiniteFloat");
        invalidColor(rows, "mix_neg_inf", "NonFiniteFloat");
        invalidColor(rows, "shade_nan", "NonFiniteFloat");
        invalidColor(rows, "shade_out_of_range", "OutOfRangeFactor");
        invalidColor(rows, "average_empty", "EmptyBlend");
        invalidColor(rows, "fluid_depth_zero", "InvalidDepth");
        return rows;
    }

    private static void invalidColor(final JsonArray rows, final String op, final String error) {
        final JsonObject row = new JsonObject();
        row.addProperty("op", op);
        row.addProperty("rust_error", error);
        try {
            switch (op) {
                case "mix_nan" -> {
                    row.addProperty("c1", hex(0x12345678)); row.addProperty("c2", hex(0x80A0B0C0));
                    row.addProperty("java_result", hex(Colors.mix(0x12345678, 0x80A0B0C0, Float.NaN)));
                }
                case "mix_pos_inf" -> {
                    row.addProperty("c1", hex(0x12345678)); row.addProperty("c2", hex(0x80A0B0C0));
                    row.addProperty("java_result", hex(Colors.mix(0x12345678, 0x80A0B0C0, Float.POSITIVE_INFINITY)));
                }
                case "mix_neg_inf" -> {
                    row.addProperty("c1", hex(0x12345678)); row.addProperty("c2", hex(0x80A0B0C0));
                    row.addProperty("java_result", hex(Colors.mix(0x12345678, 0x80A0B0C0, Float.NEGATIVE_INFINITY)));
                }
                case "shade_nan" -> {
                    row.addProperty("color", hex(0x12345678));
                    row.addProperty("java_result", hex(Colors.shade(0x12345678, Float.NaN)));
                }
                case "shade_out_of_range" -> {
                    row.addProperty("color", hex(0x12345678)); row.addProperty("factor_bits", Float.floatToRawIntBits(1.1F));
                    row.addProperty("java_result", hex(Colors.shade(0x12345678, 1.1F)));
                }
                case "average_empty" -> {
                    final ColorBlender blender = new ColorBlender();
                    blender.result();
                }
                case "fluid_depth_zero" -> row.addProperty("java_result", hex(RenderPrimitiveEngine.fluid(0, 0x12345678, true, 0, false, false, false, 0)));
                default -> throw new IllegalArgumentException(op);
            }
        } catch (final RuntimeException ex) {
            row.addProperty("java_exception", ex.getClass().getSimpleName());
        }
        rows.add(row);
    }

    private static String hex(final int color) { return String.format("0x%08X", color); }
}
