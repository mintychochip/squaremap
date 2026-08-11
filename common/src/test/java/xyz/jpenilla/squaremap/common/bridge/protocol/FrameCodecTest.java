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
import java.util.zip.CRC32C;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshot;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshotBody;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.Hello;

class FrameCodecTest {
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
        final byte[] body = ChunkSnapshotBody.getDefaultInstance().toByteArray();
        final CRC32C crc = new CRC32C();
        crc.update(body);
        final byte[] bytes = frame(1, snapshot(body.length, crc.getValue()).toByteArray());
        final Envelope decoded = FrameCodec.read(new OneByteReadable(bytes));
        assertEquals(Envelope.PayloadCase.CHUNK_SNAPSHOT, decoded.getPayloadCase());
        assertArrayEquals(bytes, write(decoded));
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
