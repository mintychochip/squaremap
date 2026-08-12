package xyz.jpenilla.squaremap.common.bridge.snapshot;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.google.protobuf.ByteString;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.atomic.AtomicInteger;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.bridge.v1.BiomeDescriptor;
import xyz.jpenilla.squaremap.bridge.v1.BlockStateDescriptor;
import xyz.jpenilla.squaremap.bridge.v1.BlockTransparency;
import xyz.jpenilla.squaremap.bridge.v1.ChunkCoordinate;
import xyz.jpenilla.squaremap.bridge.v1.ChunkMissing;
import xyz.jpenilla.squaremap.bridge.v1.ChunkMissingReason;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSection;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshot;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshotBody;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshotRequest;
import xyz.jpenilla.squaremap.bridge.v1.FluidClass;
import xyz.jpenilla.squaremap.bridge.v1.RegistryReplace;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;

final class ChunkSnapshotEncoderTest {
    private static final WorldIdentity WORLD = WorldIdentity.newBuilder()
        .setNamespace("minecraft").setValue("overworld").setEpoch(3).build();

    @Test
    void loaderHeightIsConvertedToInclusiveWireMaximum() {
        assertEquals(319, ChunkSnapshotEncoder.wireMaxY(-64, 384));
        assertEquals(24, 384 / 16);
        assertThrows(IllegalArgumentException.class, () -> ChunkSnapshotEncoder.wireMaxY(Integer.MAX_VALUE, 2));
    }

    @Test
    void encodesNegativeSectionsAndMatchesSharedFixture() throws Exception {
        final RegistryReplace registry = registry();
        final ChunkSnapshotEncoder.PalettedSection section = new ChunkSnapshotEncoder.PalettedSection(
            -4, new int[] {1, 2, 3, 4, 5}, bits(new int[] {0, 1, 2, 3, 4}, 4096, 3),
            new int[] {10, 11, 12, 13}, bits(new int[] {0, 1, 2, 3}, 64, 2));
        final ChunkSnapshotEncoder.PalettedSection[] sections = new ChunkSnapshotEncoder.PalettedSection[24];
        sections[0] = section;
        for (int i = 1; i < sections.length; i++) {
            sections[i] = new ChunkSnapshotEncoder.PalettedSection(-4 + i, new int[] {1}, new byte[0], new int[] {10}, new byte[0]);
        }
        final ChunkSnapshot snapshot = ChunkSnapshotEncoder.encode(
            new ChunkSnapshotEncoder.WorldInfo(WORLD, ChunkCoordinate.newBuilder().setX(-7).setZ(5).build(), -64, 319, false),
            sections, heightmap(-64), 42, registry
        );
        final byte[] bodyBytes = decompress(snapshot.getCompressedBody().toByteArray(), snapshot.getUncompressedLength());
        assertEquals(snapshot.getCrc32C(), crc(bodyBytes));
        final Path fixtureDirectory = Path.of("../testdata/bridge/v1");
        assertEquals(snapshot, ChunkSnapshot.parseFrom(Files.readAllBytes(fixtureDirectory.resolve("chunk_snapshot_valid.bin"))));
        assertEquals(registry, RegistryReplace.parseFrom(Files.readAllBytes(fixtureDirectory.resolve("registry_replace_valid.bin"))));
    }

    @Test
    void deterministicRegistryAndUnknownDescriptorFailClosed() {
        final RegistryReplace registry = registry();
        assertEquals(registry, RegistryDescriptorExporter.RegistrySnapshot.of(registry)
            .toRegistryReplace());
        final ChunkSnapshotEncoder.PalettedSection unknown = new ChunkSnapshotEncoder.PalettedSection(
            0, new int[] {999999}, new byte[0], new int[] {10}, new byte[0]);
        final IllegalStateException failure = assertThrows(IllegalStateException.class, () ->
            ChunkSnapshotEncoder.encode(
                new ChunkSnapshotEncoder.WorldInfo(WORLD, ChunkCoordinate.newBuilder().setX(0).setZ(0).build(), 0, 15, false),
                new ChunkSnapshotEncoder.PalettedSection[] {unknown}, new int[256], 1, registry));
        assertTrue(failure.getMessage().contains("999999"));
    }

    @Test
    void admissionIsBoundedAndMissingIsTyped() {
        final SnapshotRequestService service = new SnapshotRequestService(96);
        final AtomicInteger admitted = new AtomicInteger();
        final List<CompletableFuture<ChunkSnapshot>> gates = new ArrayList<>();
        final List<CompletableFuture<ChunkSnapshot>> results = new ArrayList<>();
        for (int i = 0; i < 100; i++) {
            final CompletableFuture<ChunkSnapshot> gate = new CompletableFuture<>();
            gates.add(gate);
            results.add(service.request(request(i + 1), ignored -> {
                admitted.incrementAndGet();
                return gate;
            }));
        }
        assertEquals(96, admitted.get());
        assertEquals(96, service.inFlightCount());
        assertTrue(results.get(96).isCompletedExceptionally());
        assertTrue(results.get(97).isCompletedExceptionally());
        gates.subList(0, 96).forEach(gate -> gate.complete(null));
        service.awaitIdle();
        assertEquals(0, service.inFlightCount());
        final ChunkMissing missing = service.requestMissingChunk(request(101), ChunkMissingReason.CHUNK_MISSING_REASON_UNLOADED);
        assertEquals(ChunkMissingReason.CHUNK_MISSING_REASON_UNLOADED, missing.getReason());
        assertEquals(0, service.inFlightCount());
    }

    @Test
    void cancellationCancelsProducerAndReleasesPermit() {
        final SnapshotRequestService service = new SnapshotRequestService(1);
        final CompletableFuture<ChunkSnapshot> producer = new CompletableFuture<>();
        final CompletableFuture<ChunkSnapshot> result = service.request(request(200), ignored -> producer);
        assertTrue(service.cancel(200));
        assertTrue(producer.isCancelled());
        assertTrue(result.isCompletedExceptionally());
        assertEquals(0, service.inFlightCount());
    }

    private static ChunkSnapshotRequest request(final long id) {
        return ChunkSnapshotRequest.newBuilder().setWorld(WORLD)
            .setCoordinate(ChunkCoordinate.newBuilder().setX((int) id).setZ((int) id))
            .setRevision(1).setRequestId(id).build();
    }

    private static RegistryReplace registry() {
        final RegistryReplace.Builder builder = RegistryReplace.newBuilder().setRevision(42).setWorld(WORLD);
        for (int i = 1; i <= 32; i++) {
            final int mapColor = i == 1 ? 0 : 0xff000000 | i;
            final BlockStateDescriptor.Builder descriptor = BlockStateDescriptor.newBuilder().setId(i).setMapColor(mapColor)
                .setTransparency(BlockTransparency.BLOCK_TRANSPARENCY_OPAQUE).setFluid(FluidClass.FLUID_CLASS_NONE);
            if (i == 1) descriptor.setTransparency(BlockTransparency.BLOCK_TRANSPARENCY_INVISIBLE);
            if (i == 2) descriptor.setTransparency(BlockTransparency.BLOCK_TRANSPARENCY_TRANSLUCENT).setFluid(FluidClass.FLUID_CLASS_WATER);
            if (i == 3) descriptor.setTransparency(BlockTransparency.BLOCK_TRANSPARENCY_TRANSLUCENT).setGlass(true).setGlassAlphaPercent(25);
            builder.addBlockStates(descriptor);
        }
        for (int i = 10; i <= 13; i++) {
            builder.addBiomes(BiomeDescriptor.newBuilder().setId(i).setGrassColor(i)
                .setFoliageColor(i).setWaterColor(i));
        }
        return builder.build();
    }

    private static int[] heightmap(final int minY) {
        final int[] result = new int[256];
        for (int i = 0; i < result.length; i++) result[i] = minY + 100 + i % 4;
        return result;
    }

    private static byte[] bits(final int[] prefix, final int entries, final int width) {
        final byte[] out = new byte[(entries * width + 7) / 8];
        for (int i = 0; i < entries; i++) {
            final int value = prefix[i % prefix.length];
            final int bit = i * width;
            for (int b = 0; b < width; b++) if ((value & (1 << b)) != 0) out[(bit + b) >>> 3] |= 1 << ((bit + b) & 7);
        }
        return out;
    }

    private static byte[] decompress(final byte[] compressed, final int declared) {
        final byte[] out = new byte[declared];
        final int actual = new io.airlift.compress.zstd.ZstdDecompressor().decompress(compressed, 0, compressed.length, out, 0, out.length);
        if (actual != declared) throw new AssertionError("decompressed length mismatch");
        return out;
    }

    private static int crc(final byte[] bytes) {
        final java.util.zip.CRC32C crc = new java.util.zip.CRC32C();
        crc.update(bytes);
        return (int) crc.getValue();
    }
}
