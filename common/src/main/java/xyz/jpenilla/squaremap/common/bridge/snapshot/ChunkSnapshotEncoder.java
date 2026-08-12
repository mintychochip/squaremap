package xyz.jpenilla.squaremap.common.bridge.snapshot;

import com.google.protobuf.ByteString;
import java.util.Arrays;
import java.util.HashSet;
import java.util.Objects;
import java.util.Set;
import java.util.zip.CRC32C;
import io.airlift.compress.zstd.ZstdCompressor;
import xyz.jpenilla.squaremap.bridge.v1.ChunkCoordinate;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSection;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshot;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshotBody;
import xyz.jpenilla.squaremap.bridge.v1.Heightmap;
import xyz.jpenilla.squaremap.bridge.v1.RegistryReplace;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;

/** Packs immutable loader-safe section values and produces bounded zstd payloads. */
public final class ChunkSnapshotEncoder {
    public static final int BLOCK_ENTRIES = 4096;
    public static final int BIOME_ENTRIES = 64;
    public static final int HEIGHTMAP_ENTRIES = 256;
    public static final int MAX_BLOCK_PALETTE = 4096;
    public static final int MAX_BIOME_PALETTE = 64;
    public static final int MAX_UNCOMPRESSED_BODY_BYTES = 134_217_728;
    public static final int MAX_COMPRESSED_BODY_BYTES = 67_108_864;
    public static final int MAX_ABS_CHUNK_COORDINATE = 1 << 22;
    private ChunkSnapshotEncoder() {}

    public static ChunkSnapshot encodePortable(final xyz.jpenilla.squaremap.common.util.chunksnapshot.ChunkSnapshot snapshot,
                                               final RegistryDescriptorExporter descriptors, final WorldIdentity identity, final long revision) {
        Objects.requireNonNull(snapshot); Objects.requireNonNull(descriptors); Objects.requireNonNull(identity);
        final int sectionCount = snapshot.getSectionsCount();
        final long span = snapshot.getHeight();
        if (span <= 0 || span % 16 != 0 || sectionCount != span / 16) throw new IllegalArgumentException("invalid snapshot section count");
        final int wireMaxY = wireMaxY(snapshot.getMinY(), snapshot.getHeight());
        final PalettedSection[] sections = new PalettedSection[sectionCount];
        for (int section = 0; section < sectionCount; section++) {
            final java.util.LinkedHashMap<Integer, Integer> blockPalette = new java.util.LinkedHashMap<>();
            final int[] blockValues = new int[BLOCK_ENTRIES];
            for (int index = 0; index < BLOCK_ENTRIES; index++) {
                final int id = descriptors.blockId(snapshot.blockStates(section).get(index));
                if (id <= 0) throw new IllegalStateException("missing block descriptor for snapshot value");
                blockValues[index] = blockPalette.computeIfAbsent(id, ignored -> blockPalette.size());
            }
            final java.util.LinkedHashMap<Integer, Integer> biomePalette = new java.util.LinkedHashMap<>();
            final int[] biomeValues = new int[BIOME_ENTRIES];
            int index = 0;
            for (int y = 0; y < 4; y++) for (int z = 0; z < 4; z++) for (int x = 0; x < 4; x++) {
                final int id = descriptors.biomeId(snapshot.biomeStates(section).get(x, y, z));
                if (id <= 0) throw new IllegalStateException("missing biome descriptor for snapshot value");
                biomeValues[index++] = biomePalette.computeIfAbsent(id, ignored -> biomePalette.size());
            }
            sections[section] = new PalettedSection(snapshot.sectionY(section), blockPalette.keySet().stream().mapToInt(Integer::intValue).toArray(),
                pack(blockValues, blockPalette.size()), biomePalette.keySet().stream().mapToInt(Integer::intValue).toArray(), pack(biomeValues, biomePalette.size()));
        }
        final int[] heights = new int[HEIGHTMAP_ENTRIES];
        for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++) heights[x + z * 16] = snapshot.getHeight(net.minecraft.world.level.levelgen.Heightmap.Types.WORLD_SURFACE, x, z) + 1;
        return encode(new WorldInfo(identity, ChunkCoordinate.newBuilder().setX(snapshot.pos().x()).setZ(snapshot.pos().z()).build(), snapshot.getMinY(), wireMaxY, snapshot.dimensionType().hasCeiling()), sections, heights, revision, descriptors.snapshot());
    }

    static int wireMaxY(final int minY, final int height) {
        if (height <= 0) throw new IllegalArgumentException("snapshot height must be positive");
        final long max = (long) minY + height - 1L;
        if (max < Integer.MIN_VALUE || max > Integer.MAX_VALUE) throw new IllegalArgumentException("snapshot vertical bounds overflow");
        return (int) max;
    }

    private static byte[] pack(final int[] values, final int paletteSize) {
        final int width = bits(paletteSize); final byte[] packed = new byte[packedBytes(values.length, width)];
        if (width == 0) return packed;
        for (int i = 0; i < values.length; i++) { if (values[i] >= paletteSize) throw new IllegalArgumentException("packed palette index out of range"); final int offset = i * width; for (int bit = 0; bit < width; bit++) if ((values[i] & (1 << bit)) != 0) packed[(offset + bit) >>> 3] |= (byte) (1 << ((offset + bit) & 7)); }
        return packed;
    }

    public static ChunkSnapshot encode(final WorldInfo world, final PalettedSection[] sections, final int[] heights, final long revision, final RegistryReplace registry) {
        return encode(world, sections, heights, revision, registry, MAX_UNCOMPRESSED_BODY_BYTES, MAX_COMPRESSED_BODY_BYTES);
    }
    public static ChunkSnapshot encode(final WorldInfo world, final PalettedSection[] sections, final int[] heights, final long revision,
                                       final RegistryReplace registry, final int maxUncompressed, final int maxCompressed) {
        Objects.requireNonNull(world); Objects.requireNonNull(sections); Objects.requireNonNull(heights); Objects.requireNonNull(registry);
        final int x = world.coordinate().getX(), z = world.coordinate().getZ();
        if (x < -MAX_ABS_CHUNK_COORDINATE || x > MAX_ABS_CHUNK_COORDINATE || z < -MAX_ABS_CHUNK_COORDINATE || z > MAX_ABS_CHUNK_COORDINATE) throw new IllegalArgumentException("chunk coordinate out of range");
        final long span = (long) world.maxY() - world.minY() + 1L;
        if (span <= 0 || span % 16 != 0 || sections.length != span / 16) throw new IllegalArgumentException("section count does not match vertical bounds");
        if (heights.length != HEIGHTMAP_ENTRIES) throw new IllegalArgumentException("heightmap must contain 256 values");
        for (int height : heights) if (height < world.minY() || (long) height > world.maxY() + 1L) throw new IllegalArgumentException("heightmap value out of bounds");
        final Set<Integer> blocks = new HashSet<>(), biomes = new HashSet<>();
        registry.getBlockStatesList().forEach(value -> blocks.add(value.getId())); registry.getBiomesList().forEach(value -> biomes.add(value.getId()));
        final ChunkSnapshotBody.Builder body = ChunkSnapshotBody.newBuilder();
        for (int index = 0; index < sections.length; index++) {
            final PalettedSection section = sections[index];
            if (section.sectionY() != Math.floorDiv(world.minY(), 16) + index) throw new IllegalArgumentException("section Y order mismatch");
            validatePalette(section.blockPalette(), blocks, "block"); validatePalette(section.biomePalette(), biomes, "biome");
            validatePacked(section.blockIndices(), section.blockPalette().length, BLOCK_ENTRIES, bits(section.blockPalette().length), "block");
            validatePacked(section.biomeIndices(), section.biomePalette().length, BIOME_ENTRIES, bits(section.biomePalette().length), "biome");
            body.addSections(ChunkSection.newBuilder().setSectionY(section.sectionY()).addAllBlockPalette(Arrays.stream(section.blockPalette()).boxed().toList()).setBlockIndices(ByteString.copyFrom(section.blockIndices())).addAllBiomePalette(Arrays.stream(section.biomePalette()).boxed().toList()).setBiomeIndices(ByteString.copyFrom(section.biomeIndices())));
        }
        body.setSurfaceHeightmap(Heightmap.newBuilder().addAllHeights(Arrays.stream(heights).boxed().toList()));
        final byte[] uncompressed = body.build().toByteArray();
        if (uncompressed.length > maxUncompressed) throw new IllegalArgumentException("snapshot body exceeds limit");
        final CRC32C crc = new CRC32C(); crc.update(uncompressed, 0, uncompressed.length);
        final ZstdCompressor compressor = new ZstdCompressor(); final int maxLength = compressor.maxCompressedLength(uncompressed.length);
        if (maxLength < 0 || maxLength > maxCompressed + 131_072) throw new IllegalArgumentException("compressed snapshot exceeds limit");
        final byte[] output = new byte[maxLength]; final int compressedLength = compressor.compress(uncompressed, 0, uncompressed.length, output, 0, output.length);
        if (compressedLength > maxCompressed) throw new IllegalArgumentException("compressed snapshot exceeds limit");
        return ChunkSnapshot.newBuilder().setWorld(world.identity()).setCoordinate(world.coordinate()).setMinY(world.minY()).setMaxY(world.maxY()).setCeiling(world.ceiling()).setRevision(revision).setUncompressedLength(uncompressed.length).setCrc32C((int) crc.getValue()).setCompressedBody(ByteString.copyFrom(output, 0, compressedLength)).build();
    }

    private static void validatePalette(final int[] palette, final Set<Integer> ids, final String kind) {
        final int maximum = kind.equals("block") ? MAX_BLOCK_PALETTE : MAX_BIOME_PALETTE;
        if (palette.length == 0 || palette.length > maximum) throw new IllegalArgumentException(kind + " palette length out of range");
        final Set<Integer> unique = new HashSet<>();
        for (int id : palette) { if (!ids.contains(id)) throw new IllegalStateException("unknown " + kind + " descriptor " + id); if (!unique.add(id)) throw new IllegalArgumentException("duplicate " + kind + " palette descriptor " + id); }
    }
    private static void validatePacked(final byte[] packed, final int paletteLength, final int entries, final int width, final String kind) {
        final int expected = packedBytes(entries, width); if (packed.length != expected) throw new IllegalArgumentException(kind + " packed width does not match palette"); if (width == 0) return;
        final int used = entries * width;
        for (int i = 0; i < entries; i++) { final int offset = i * width; int value = 0; for (int bit = 0; bit < width; bit++) if ((packed[(offset + bit) >>> 3] & (1 << ((offset + bit) & 7))) != 0) value |= 1 << bit; if (value >= paletteLength) throw new IllegalArgumentException(kind + " packed index exceeds palette"); }
        final int trailing = packed.length * 8 - used; if (trailing > 0 && (packed[packed.length - 1] & (0xFF << (8 - trailing))) != 0) throw new IllegalArgumentException(kind + " packed trailing bits are nonzero");
    }
    public static int bits(final int paletteLength) { return paletteLength <= 1 ? 0 : 32 - Integer.numberOfLeadingZeros(paletteLength - 1); }
    public static int packedBytes(final int entries, final int bits) { return (entries * bits + 7) / 8; }

    public record WorldInfo(WorldIdentity identity, ChunkCoordinate coordinate, int minY, int maxY, boolean ceiling) {
        public WorldInfo { Objects.requireNonNull(identity); Objects.requireNonNull(coordinate); final long span = (long) maxY - minY + 1L; if (span <= 0 || span % 16 != 0) throw new IllegalArgumentException("invalid vertical bounds"); }
    }
    public record PalettedSection(int sectionY, int[] blockPalette, byte[] blockIndices, int[] biomePalette, byte[] biomeIndices) {
        public PalettedSection { blockPalette = blockPalette.clone(); blockIndices = blockIndices.clone(); biomePalette = biomePalette.clone(); biomeIndices = biomeIndices.clone(); }
    }
}
