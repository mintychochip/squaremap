package xyz.jpenilla.squaremap.common.bridge.protocol;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import io.airlift.compress.zstd.ZstdCompressor;
import com.google.protobuf.ByteString;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.channels.ReadableByteChannel;
import java.nio.channels.WritableByteChannel;
import java.util.ArrayList;
import java.util.Base64;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.zip.CRC32C;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSection;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshot;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshotBody;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.Hello;

class FrameCodecTest {
    private static final String HIGH_WINDOW_ZSTD_BASE64 =
        "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";
    @Test
    void rejectsUnknownFrameClass() {
        assertProtocolFailure(new byte[] {2, 0, 0, 0, 1, 0});
    }

    @Test
    void rejectsZeroLength() {
        assertProtocolFailure(new byte[] {0, 0, 0, 0, 0});
    }

    @Test
    void rejectsControlFrameOverLimitBeforePayloadAllocation() {
        final byte[] frame = prefix(0, 1_048_577L);
        assertProtocolFailure(frame);
    }

    @Test
    void rejectsSnapshotFrameOverLimitBeforePayloadAllocation() {
        final byte[] frame = prefix(1, 67_108_865L);
        assertProtocolFailure(frame);
    }

    @Test
    void rejectsUnsignedLengthBeforeConvertingToInt() {
        assertProtocolFailure(prefix(0, 0xffff_ffffL));
    }

    @Test
    void rejectsClassPayloadMismatchInBothDirections() {
        final byte[] controlPayload = hello().toByteArray();
        final byte[] snapshotClass = frame(1, controlPayload);
        assertProtocolFailure(snapshotClass);

        final byte[] snapshotPayload = snapshot(0, 1).toByteArray();
        final byte[] controlClass = frame(0, snapshotPayload);
        assertProtocolFailure(controlClass);
    }

    @Test
    void rejectsEarlyEofWithExpectedAndActualCounts() {
        final FrameCodec.ProtocolException exception = assertThrows(
            FrameCodec.ProtocolException.class,
            () -> FrameCodec.read(new ByteArrayReadable(new byte[] {0, 0}))
        );
        assertEquals("early EOF", exception.reason());
        assertEquals(5, exception.expected());
        assertEquals(2, exception.actual());

        final byte[] complete = frame(0, hello().toByteArray());
        final byte[] truncated = java.util.Arrays.copyOf(complete, complete.length - 1);
        assertProtocolFailure(truncated);
    }

    @Test
    void rejectsTrailingBytesAndInvalidProtobuf() {
        final byte[] payload = hello().toByteArray();
        final byte[] trailing = frame(0, java.util.Arrays.copyOf(payload, payload.length + 1));
        assertProtocolFailure(trailing);
        assertProtocolFailure(new byte[] {0, 0, 0, 0, 1, (byte) 0xff});
    }

    @Test
    void rejectsSnapshotDecompressionAboveAbsoluteLimit() {
        assertProtocolFailure(frame(1, snapshot(134_217_729, 0).toByteArray()));
    }

    @Test
    void rejectsSnapshotDecompressionRatioAbove4096() {
        final byte[] compressed = compress(ChunkSnapshotBody.getDefaultInstance().toByteArray());
        final long declaredLength = compressed.length * 4096L + 1;
        assertProtocolFailure(frame(1, snapshotWithCompressed(declaredLength, 0, compressed).toByteArray()));
    }

    @Test
    void rejectsEmptyOrInvalidZstdBody() {
        assertProtocolFailure(frame(1, snapshotWithCompressed(0, 0, new byte[0]).toByteArray()));
        assertProtocolFailure(frame(1, snapshotWithCompressed(0, 0, new byte[] {0x01, 0x02, 0x03}).toByteArray()));
    }

    @Test
    void acceptsEmptySnapshotBody() throws Exception {
        final byte[] compressed = compress(ChunkSnapshotBody.getDefaultInstance().toByteArray());
        final Envelope decoded = FrameCodec.read(new ByteArrayReadable(frame(
            1,
            snapshotWithCompressed(0, 0, compressed).toByteArray()
        )));
        assertEquals(Envelope.PayloadCase.CHUNK_SNAPSHOT, decoded.getPayloadCase());
        assertEquals(0, decoded.getChunkSnapshot().getUncompressedLength());
    }

    @Test
    void rejectsSkippableAndConcatenatedZstdFrames() {
        final byte[] standard = compress(ChunkSnapshotBody.getDefaultInstance().toByteArray());
        final byte[] skippable = new byte[9 + standard.length];
        skippable[0] = 0x50;
        skippable[1] = 0x2a;
        skippable[2] = 0x4d;
        skippable[3] = 0x18;
        skippable[4] = 1;
        System.arraycopy(standard, 0, skippable, 9, standard.length);
        final byte[] concatenated = new byte[standard.length * 2];
        System.arraycopy(standard, 0, concatenated, 0, standard.length);
        System.arraycopy(standard, 0, concatenated, standard.length, standard.length);
        final byte[] trailing = java.util.Arrays.copyOf(standard, standard.length + 1);
        trailing[trailing.length - 1] = 0;
        for (final byte[] compressed : new byte[][] {skippable, concatenated, trailing}) {
            assertProtocolFailure(frame(1, snapshotWithCompressed(0, 0, compressed).toByteArray()));
        }
    }

    @Test
    void rejectsTruncatedMagicPrefixedZeroLengthSnapshot() {
        final byte[] truncated = {(byte) 0x28, (byte) 0xb5, 0x2f, (byte) 0xfd, 0x00};
        assertThrows(
            FrameCodec.ProtocolException.class,
            () -> FrameCodec.read(new ByteArrayReadable(frame(
                1,
                snapshotWithCompressed(0, 0, truncated).toByteArray()
            )))
        );
    }
    @Test
    void rejectsSnapshotFrameWithWindowAboveSharedPolicy() {
        final byte[] compressed = Base64.getDecoder().decode(HIGH_WINDOW_ZSTD_BASE64);
        assertProtocolFailure(frame(
            1,
            snapshotWithCompressed(6_000_000, 0, compressed).toByteArray()
        ));
    }

    @Test
    void acceptsLargeSnapshotWithCompliantWindow() throws Exception {
        final byte[] body = ordinaryLargeBody();
        final CRC32C crc = new CRC32C();
        crc.update(body);
        final Envelope envelope = snapshotWithCompressed(body.length, crc.getValue(), compress(body));
        final Envelope decoded = FrameCodec.read(new ByteArrayReadable(frame(1, envelope.toByteArray())));
        assertEquals(body.length, decoded.getChunkSnapshot().getUncompressedLength());
    }
    @Test
    void rejectsUnsignedSnapshotLengthBeforeAllocation() {
        final byte[] compressed = compress(ChunkSnapshotBody.getDefaultInstance().toByteArray());
        assertProtocolFailure(frame(
            1,
            snapshotWithCompressed(0xffff_ffffL, 0, compressed).toByteArray()
        ));
    }

    @Test
    void rejectsReadableChannelWithoutProgress() {
        final FrameCodec.ProtocolException exception = assertThrows(
            FrameCodec.ProtocolException.class,
            () -> FrameCodec.read(new ZeroThenEofReadable())
        );
        assertEquals("no progress", exception.reason());
        assertEquals(0, exception.actual());
    }

    @Test
    void rejectsWritableChannelWithoutProgress() {
        final FrameCodec.ProtocolException exception = assertThrows(
            FrameCodec.ProtocolException.class,
            () -> FrameCodec.write(new ZeroThenEofWritable(), hello())
        );
        assertEquals("no progress", exception.reason());
        assertEquals(0, exception.actual());
    }

    @Test
    void rejectsSnapshotCrcMismatch() {
        assertProtocolFailure(frame(1, snapshot(0, 1).toByteArray()));
    }

    @Test
    void acceptsValidFrameSplitAcrossOneByteReads() throws Exception {
        final byte[] body = ChunkSnapshotBody.newBuilder()
            .addSections(ChunkSection.newBuilder().setSectionY(0).build())
            .build()
            .toByteArray();
        final CRC32C crc = new CRC32C();
        crc.update(body);
        final byte[] bytes = frame(
            1,
            snapshotWithCompressed(body.length, crc.getValue(), compress(body)).toByteArray()
        );
        final Envelope decoded = FrameCodec.read(new OneByteReadable(bytes));
        assertEquals(Envelope.PayloadCase.CHUNK_SNAPSHOT, decoded.getPayloadCase());
        assertArrayEquals(bytes, write(decoded));
    }

    @Test
    void validatesDistinctBodiesConcurrently() throws Exception {
        final List<Envelope> envelopes = new ArrayList<>();
        for (int id = 1; id <= 8; id++) {
            final byte[] body = ChunkSnapshotBody.newBuilder()
                .addSections(ChunkSection.newBuilder()
                    .setSectionY(id)
                    .addBlockPalette(id)
                    .setBlockIndices(ByteString.copyFrom(new byte[] {(byte) id, (byte) (id + 1)}))
                    .build())
                .build()
                .toByteArray();
            final CRC32C crc = new CRC32C();
            crc.update(body);
            envelopes.add(snapshotWithCompressed(body.length, crc.getValue(), compress(body)));
        }

        final ExecutorService executor = Executors.newFixedThreadPool(envelopes.size());
        final CountDownLatch ready = new CountDownLatch(envelopes.size());
        final CountDownLatch start = new CountDownLatch(1);
        try {
            final List<Future<?>> futures = new ArrayList<>();
            for (final Envelope envelope : envelopes) {
                futures.add(executor.submit(() -> {
                    ready.countDown();
                    start.await();
                    final long expectedCrc = Integer.toUnsignedLong(
                        envelope.getChunkSnapshot().getCrc32C()
                    );
                    for (int iteration = 0; iteration < 128; iteration++) {
                        final ByteArrayWritable writable = new ByteArrayWritable();
                        FrameCodec.write(writable, envelope);
                        final Envelope decoded = FrameCodec.read(new ByteArrayReadable(writable.bytes()));
                        assertEquals(
                            expectedCrc,
                            Integer.toUnsignedLong(decoded.getChunkSnapshot().getCrc32C())
                        );
                    }
                    return null;
                }));
            }
            ready.await();
            start.countDown();
            for (final Future<?> future : futures) {
                future.get();
            }
        } finally {
            executor.shutdownNow();
        }
    }

    @Test
    void writeRejectsInvalidSnapshotBeforeWriting() {
        final ByteArrayWritable sink = new ByteArrayWritable();
        assertThrows(
            FrameCodec.ProtocolException.class,
            () -> FrameCodec.write(sink, snapshot(0, 1))
        );
        assertEquals(0, sink.bytes().length);
    }

    private static Envelope hello() {
        return Envelope.newBuilder()
            .setHello(Hello.getDefaultInstance())
            .build();
    }

    private static byte[] compress(final byte[] body) {
        final ZstdCompressor compressor = new ZstdCompressor();
        final byte[] compressed = new byte[compressor.maxCompressedLength(body.length)];
        final int length = compressor.compress(
            body,
            0,
            body.length,
            compressed,
            0,
            compressed.length
        );
        return java.util.Arrays.copyOf(compressed, length);
    }

    private static byte[] ordinaryLargeBody() {
        final int payloadLength = 9_000_000;
        final byte[] body = new byte[6 + payloadLength];
        body[0] = (byte) 0xa2;
        body[1] = 0x06;
        body[2] = (byte) 0xc0;
        body[3] = (byte) 0xa8;
        body[4] = (byte) 0xa5;
        body[5] = 0x04;
        int state = 0x1234_5678;
        for (int index = 0; index < payloadLength; index++) {
            state = state * 1_664_525 + 1_013_904_223;
            body[6 + index] = (byte) (state >>> 24);
        }
        return body;
    }

    private static Envelope snapshot(final long uncompressedLength, final long crc32c) {
        final byte[] body = ChunkSnapshotBody.getDefaultInstance().toByteArray();
        return snapshotWithCompressed(uncompressedLength, crc32c, compress(body));
    }

    private static Envelope snapshotWithCompressed(
        final long uncompressedLength,
        final long crc32c,
        final byte[] compressed
    ) {
        return Envelope.newBuilder()
            .setChunkSnapshot(ChunkSnapshot.newBuilder()
                .setUncompressedLength((int) uncompressedLength)
                .setCrc32C((int) crc32c)
                .setCompressedBody(ByteString.copyFrom(compressed))
                .build())
            .build();
    }

    private static byte[] frame(final int frameClass, final byte[] payload) {
        final byte[] prefix = prefix(frameClass, payload.length);
        final byte[] result = java.util.Arrays.copyOf(prefix, prefix.length + payload.length);
        System.arraycopy(payload, 0, result, prefix.length, payload.length);
        return result;
    }

    private static byte[] prefix(final int frameClass, final long length) {
        return new byte[] {
            (byte) frameClass,
            (byte) (length >>> 24),
            (byte) (length >>> 16),
            (byte) (length >>> 8),
            (byte) length
        };
    }

    private static void assertProtocolFailure(final byte[] bytes) {
        assertThrows(
            FrameCodec.ProtocolException.class,
            () -> FrameCodec.read(new ByteArrayReadable(bytes))
        );
    }

    private static byte[] write(final Envelope envelope) throws Exception {
        final ByteArrayWritable writable = new ByteArrayWritable();
        FrameCodec.write(writable, envelope);
        return writable.bytes();
    }

    private static class ByteArrayReadable implements ReadableByteChannel {
        private final byte[] bytes;
        private int position;
        private boolean open = true;

        private ByteArrayReadable(final byte[] bytes) {
            this.bytes = bytes;
        }

        @Override
        public int read(final ByteBuffer destination) {
            if (this.position == this.bytes.length) {
                return -1;
            }
            final int count = Math.min(destination.remaining(), this.bytes.length - this.position);
            destination.put(this.bytes, this.position, count);
            this.position += count;
            return count;
        }

        @Override
        public boolean isOpen() {
            return this.open;
        }

        @Override
        public void close() {
            this.open = false;
        }
    }

    private static final class ZeroThenEofReadable implements ReadableByteChannel {
        private boolean first = true;

        @Override
        public int read(final ByteBuffer destination) {
            if (this.first) {
                this.first = false;
                return 0;
            }
            return -1;
        }

        @Override
        public boolean isOpen() {
            return true;
        }

        @Override
        public void close() {
        }
    }

    private static final class OneByteReadable extends ByteArrayReadable {
        private OneByteReadable(final byte[] bytes) {
            super(bytes);
        }

        @Override
        public int read(final ByteBuffer destination) {
            if (!destination.hasRemaining()) {
                return 0;
            }
            final int oldLimit = destination.limit();
            destination.limit(destination.position() + 1);
            final int count = super.read(destination);
            destination.limit(oldLimit);
            return count;
        }
    }

    private static final class ByteArrayWritable implements WritableByteChannel {
        private final ByteArrayOutputStream output = new ByteArrayOutputStream();
        private boolean open = true;

        @Override
        public int write(final ByteBuffer source) {
            final int count = source.remaining();
            final byte[] bytes = new byte[count];
            source.get(bytes);
            this.output.writeBytes(bytes);
            return count;
        }

        @Override
        public boolean isOpen() {
            return this.open;
        }

        @Override
        public void close() {
            this.open = false;
        }

        private byte[] bytes() {
            return this.output.toByteArray();
        }
    }

    private static final class ZeroThenEofWritable implements WritableByteChannel {
        private boolean first = true;

        @Override
        public int write(final ByteBuffer source) {
            if (this.first) {
                this.first = false;
                return 0;
            }
            return -1;
        }

        @Override
        public boolean isOpen() {
            return true;
        }

        @Override
        public void close() throws IOException {
        }
    }
}
