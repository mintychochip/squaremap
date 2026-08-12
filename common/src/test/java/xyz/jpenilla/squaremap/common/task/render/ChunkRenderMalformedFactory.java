package xyz.jpenilla.squaremap.common.task.render;

import com.google.protobuf.ByteString;
import io.airlift.compress.zstd.ZstdCompressor;
import io.airlift.compress.zstd.ZstdDecompressor;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.zip.CRC32C;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSection;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshot;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshotBody;
final class ChunkRenderMalformedFactory {
    record Mutation(String baseId, byte[] bytes, String classifier, String changedField) {}
    private ChunkRenderMalformedFactory() {}
    static Map<String, Mutation> build(final ChunkRenderFixtureCatalog catalog) {
        final var flat = catalog.cases().get("flat-solid");
        final ChunkSnapshot base = flat.wireSnapshot();
        final ChunkSnapshotBody body = parse(decompress(base));
        final ChunkSection first = body.getSections(0);
        final Map<String, Mutation> out = new LinkedHashMap<>();
        out.put("malformed-unknown-descriptor", bodyMutation(flat.id(), base, body.toBuilder().setSections(0, first.toBuilder().setBlockPalette(0, 1_000_000)).build(), "UnknownDescriptor(block)", "block_palette[0]"));
        out.put("malformed-heightmap", bodyMutation(flat.id(), base, body.toBuilder().setSurfaceHeightmap(body.getSurfaceHeightmap().toBuilder().setHeights(0, base.getMaxY() + 2)).build(), "HeightOutOfRange", "heightmap[0]"));
        out.put("malformed-palette", bodyMutation(flat.id(), base, body.toBuilder().setSections(0, first.toBuilder().clearBlockPalette()).build(), "PaletteLength(block)", "block_palette"));
        out.put("malformed-bounds", header(flat.id(), base.toBuilder().setMaxY(base.getMinY()), "VerticalBounds", "max_y"));
        out.put("malformed-crc", header(flat.id(), base.toBuilder().setCrc32C(base.getCrc32C() ^ 1), "Crc", "crc32c"));
        out.put("malformed-length", header(flat.id(), base.toBuilder().setUncompressedLength(base.getUncompressedLength() - 1), "DecompressedLength", "uncompressed_length"));
        out.put("malformed-section-count", bodyMutation(flat.id(), base, body.toBuilder().addSections(first).build(), "SectionCountMismatch", "sections"));
        out.put("malformed-section-y", bodyMutation(flat.id(), base, body.toBuilder().setSections(0, first.toBuilder().setSectionY(first.getSectionY() + 1)).build(), "SectionYOutOfRange", "section_y"));
        final byte[] packed = first.getBlockIndices().toByteArray();
        out.put("malformed-packed-width", bodyMutation(flat.id(), base, body.toBuilder().setSections(0, first.toBuilder().setBlockIndices(ByteString.copyFrom(Arrays.copyOf(packed, packed.length + 1)))).build(), "InvalidPacking(block length)", "block_indices"));
        final Map.Entry<String, ChunkRenderFixtureCatalog.CaseModel> width = catalog.cases().entrySet().stream()
            .filter(entry -> hasThreePalette(entry.getValue().wireSnapshot())).findFirst()
            .orElseThrow(() -> new IllegalStateException("trailing-bits prerequisite requires canonical section with exactly three block descriptors"));
        final ChunkSnapshot widthBase = width.getValue().wireSnapshot();
        final ChunkSnapshotBody widthBody = parse(decompress(widthBase));
        final int widthSectionIndex = findThreePaletteSection(widthBody);
        final ChunkSection widthSection = widthBody.getSections(widthSectionIndex);
        final byte[] trailing = widthSection.getBlockIndices().toByteArray();
        final int trailingIndex = 4095;
        trailing[trailingIndex >>> 2] &= (byte) ~(0x3 << ((trailingIndex & 3) * 2));
        trailing[trailingIndex >>> 2] |= (byte) (0x3 << ((trailingIndex & 3) * 2));
        out.put("malformed-trailing-bits", bodyMutation(width.getKey(), widthBase, widthBody.toBuilder().setSections(widthSectionIndex, widthSection.toBuilder().setBlockIndices(ByteString.copyFrom(trailing))).build(), "InvalidPacking(block index)", "block_indices[index=3]"));
        final int duplicate = first.getBlockPalette(0);
        out.put("malformed-duplicate-palette", bodyMutation(flat.id(), base, body.toBuilder().setSections(0, first.toBuilder().addBlockPalette(duplicate)).build(), "DuplicatePalette(block)", "block_palette[last]"));
        final byte[] protobuf = Arrays.copyOf(decompress(base), decompress(base).length + 2);
        protobuf[protobuf.length - 2] = 0x08; protobuf[protobuf.length - 1] = 0x00;
        out.put("malformed-protobuf-body", framed(flat.id(), base, protobuf, "Protobuf", "body"));
        out.put("malformed-generation", header(flat.id(), base.toBuilder().setRevision(base.getRevision() + 1), "RegistryMismatch", "revision"));
        final var neighborModel = catalog.cases().get("north-height-discontinuity");
        final ChunkSnapshot neighborBase = catalog.wireSnapshot(neighborModel.neighbors().get(new net.minecraft.world.level.ChunkPos(0, -1)));
        out.put("malformed-neighbor-coordinate", header("north-neighbor", neighborBase.toBuilder().setCoordinate(neighborBase.getCoordinate().toBuilder().setX(neighborBase.getCoordinate().getX() + 1)), "CoordinateMismatch", "coordinate"));
        return out;
    }
    private static ChunkSnapshotBody parse(final byte[] raw) {
        try { return ChunkSnapshotBody.parseFrom(raw); } catch (com.google.protobuf.InvalidProtocolBufferException exception) { throw new IllegalStateException(exception); }
    }
    private static int findThreePaletteSection(final ChunkSnapshotBody body) {
        for (int index = 0; index < body.getSectionsCount(); index++) {
            final ChunkSection section = body.getSections(index);
            if (section.getBlockPaletteCount() == 3) return index;
        }
        throw new IllegalStateException("missing three-palette section sizes=" + body.getSectionsList().stream().map(section -> section.getBlockPaletteCount() + "/" + section.getBlockIndices().size()).toList());
    }
    private static boolean hasThreePalette(final ChunkSnapshot snapshot) {
        final ChunkSnapshotBody body = parse(decompress(snapshot));
        return body.getSectionsList().stream().anyMatch(section -> section.getBlockPaletteCount() == 3);
    }

    private static Mutation header(final String baseId, final ChunkSnapshot.Builder builder, final String classifier, final String field) {
        return new Mutation(baseId, builder.build().toByteArray(), classifier, field);
    }
    private static Mutation bodyMutation(final String baseId, final ChunkSnapshot base, final ChunkSnapshotBody body, final String classifier, final String field) {
        return framed(baseId, base, body.toByteArray(), classifier, field);
    }
    private static Mutation framed(final String baseId, final ChunkSnapshot base, final byte[] raw, final String classifier, final String field) {
        final ZstdCompressor compressor = new ZstdCompressor();
        final byte[] compressed = new byte[compressor.maxCompressedLength(raw.length)];
        final int length = compressor.compress(raw, 0, raw.length, compressed, 0, compressed.length);
        final CRC32C crc = new CRC32C();
        crc.update(raw);
        return new Mutation(baseId, base.toBuilder().setCompressedBody(ByteString.copyFrom(compressed, 0, length))
            .setUncompressedLength(raw.length).setCrc32C((int) crc.getValue()).build().toByteArray(), classifier, field);
    }
    static byte[] decompress(final ChunkSnapshot snapshot) {
        final byte[] raw = new byte[snapshot.getUncompressedLength()];
        final int size = new ZstdDecompressor().decompress(snapshot.getCompressedBody().toByteArray(), 0, snapshot.getCompressedBody().size(), raw, 0, raw.length);
        if (size != raw.length) throw new IllegalStateException("zstd length");
        return raw;
    }
}
