package xyz.jpenilla.squaremap.common.task.render;

import com.google.protobuf.ByteString;
import java.util.Arrays;
import java.util.Map;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSection;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshot;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshotBody;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;

final class ChunkRenderMalformedFactoryTest {
    @Test void mutationsChangeOnlyAuditedFieldAndMaintainFraming() throws Exception {
        final ChunkRenderFixtureCatalog catalog = ChunkRenderFixtureCatalog.create();
        final Map<String, ChunkRenderMalformedFactory.Mutation> mutations = ChunkRenderMalformedFactory.build(catalog);
        final ChunkSnapshot flat = catalog.cases().get("flat-solid").wireSnapshot();
        final ChunkSnapshotBody flatBody = ChunkSnapshotBody.parseFrom(ChunkRenderMalformedFactory.decompress(flat));
        final ChunkSection flatSection = flatBody.getSections(0);
        for (final var entry : mutations.entrySet()) {
            final String id = entry.getKey();
            final ChunkRenderMalformedFactory.Mutation mutation = entry.getValue();
            final ChunkSnapshot base = id.equals("malformed-neighbor-coordinate")
                ? catalog.wireSnapshot(catalog.cases().get("north-height-discontinuity").neighbors().get(new net.minecraft.world.level.ChunkPos(0, -1)))
                : catalog.cases().get(mutation.baseId()).wireSnapshot();
            final ChunkSnapshot changed = ChunkSnapshot.parseFrom(mutation.bytes());
            assertEquals(mutation.classifier(), expectedClassifier(id));
            if (id.equals("malformed-crc")) assertEquals(base.getCrc32C() ^ 1, changed.getCrc32C());
            else if (id.equals("malformed-length")) assertEquals(base.getUncompressedLength() - 1, changed.getUncompressedLength());
            else if (id.equals("malformed-bounds")) assertEquals(base.getMinY(), changed.getMaxY());
            else if (id.equals("malformed-generation")) assertEquals(base.getRevision() + 1, changed.getRevision());
            else if (id.equals("malformed-neighbor-coordinate")) assertEquals(base.getCoordinate().getX() + 1, changed.getCoordinate().getX());
            else {
                assertEquals(base.getMinY(), changed.getMinY());
                assertEquals(base.getMaxY(), changed.getMaxY());
                assertEquals(base.getRevision(), changed.getRevision());
                final byte[] raw = ChunkRenderMalformedFactory.decompress(changed);
                assertEquals(raw.length, changed.getUncompressedLength());
                final ChunkSnapshotBody changedBody = id.equals("malformed-protobuf-body") ? null : ChunkSnapshotBody.parseFrom(raw);
                if (id.equals("malformed-unknown-descriptor")) assertEquals(1_000_000, changedBody.getSections(0).getBlockPalette(0));
                if (id.equals("malformed-heightmap")) assertEquals(base.getMaxY() + 2, changedBody.getSurfaceHeightmap().getHeights(0));
                if (id.equals("malformed-palette")) assertEquals(0, changedBody.getSections(0).getBlockPaletteCount());
                if (id.equals("malformed-section-count")) assertEquals(ChunkSnapshotBody.parseFrom(ChunkRenderMalformedFactory.decompress(base)).getSectionsCount() + 1, changedBody.getSectionsCount());
                if (id.equals("malformed-section-y")) assertEquals(flatBody.getSections(0).getSectionY() + 1, changedBody.getSections(0).getSectionY());
                if (id.equals("malformed-packed-width")) assertEquals(flatSection.getBlockIndices().size() + 1, changedBody.getSections(0).getBlockIndices().size());
                final java.util.zip.CRC32C crc = new java.util.zip.CRC32C(); crc.update(raw); assertEquals((int) crc.getValue(), changed.getCrc32C());
            }
        }
        final ChunkRenderMalformedFactory.Mutation duplicate = mutations.get("malformed-duplicate-palette");
        final ChunkSection duplicateSection = ChunkSnapshotBody.parseFrom(ChunkRenderMalformedFactory.decompress(ChunkSnapshot.parseFrom(duplicate.bytes()))).getSections(0);
        assertEquals(duplicateSection.getBlockPalette(0), duplicateSection.getBlockPalette(duplicateSection.getBlockPaletteCount() - 1));
        final byte[] protobufRaw = ChunkRenderMalformedFactory.decompress(ChunkSnapshot.parseFrom(mutations.get("malformed-protobuf-body").bytes()));
        assertEquals(8, protobufRaw[protobufRaw.length - 2]);
        assertEquals(0, protobufRaw[protobufRaw.length - 1]);
        final ChunkSnapshotBody protobufBody = ChunkSnapshotBody.parseFrom(protobufRaw);
        assertEquals(ChunkSnapshotBody.parseFrom(ChunkRenderMalformedFactory.decompress(catalog.cases().get("flat-solid").wireSnapshot())).getSectionsCount(), protobufBody.getSectionsCount());
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
}
