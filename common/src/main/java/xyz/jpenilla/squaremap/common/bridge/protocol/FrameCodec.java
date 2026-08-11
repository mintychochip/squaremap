package xyz.jpenilla.squaremap.common.bridge.protocol;

import io.airlift.compress.zstd.ZstdDecompressor;
import com.google.protobuf.ByteString;
import com.google.protobuf.InvalidProtocolBufferException;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.channels.ReadableByteChannel;
import java.nio.channels.WritableByteChannel;
import java.util.zip.CRC32C;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshot;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshotBody;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;

/** Reads and writes bounded class/length-prefixed bridge envelopes. */
public final class FrameCodec {
    private static final int PREFIX_BYTES = 5;
    private static final ThreadLocal<ZstdDecompressor> ZSTD_DECOMPRESSOR =
        ThreadLocal.withInitial(ZstdDecompressor::new);
    private static final byte[] EMPTY_BYTES = new byte[0];

    private FrameCodec() {
    }

    public static Envelope read(final ReadableByteChannel channel) throws IOException {
        return read(channel, FrameLimits.DEFAULT);
    }

    public static Envelope read(
        final ReadableByteChannel channel,
        final FrameLimits limits
    ) throws IOException {
        final ByteBuffer prefix = ByteBuffer.allocate(PREFIX_BYTES);
        readFully(channel, prefix);
        prefix.flip();
        final int classByte = Byte.toUnsignedInt(prefix.get());
        final FrameClass frameClass = FrameClass.fromByte(classByte);
        final long length = Integer.toUnsignedLong(prefix.getInt());
        if (length == 0) {
            throw ProtocolException.malformedLength("zero length");
        }
        final long maximum = frameClass.maximum(limits);
        if (length > maximum) {
            throw ProtocolException.declaredLength(length, maximum);
        }
        if (length > Integer.MAX_VALUE) {
            throw ProtocolException.declaredLength(length, Integer.MAX_VALUE);
        }

        // The class-specific limit check above must precede this allocation.
        final ByteBuffer payload = ByteBuffer.allocate((int) length);
        readFully(channel, payload);
        final Envelope envelope;
        try {
            envelope = Envelope.parseFrom(payload.array());
        } catch (final InvalidProtocolBufferException exception) {
            throw ProtocolException.protobuf(exception);
        }
        final FrameClass payloadClass = FrameClass.forPayload(envelope);
        if (payloadClass != frameClass) {
            throw ProtocolException.classMismatch(frameClass, payloadClass);
        }
        if (envelope.getPayloadCase() == Envelope.PayloadCase.CHUNK_SNAPSHOT) {
            validateSnapshot(envelope.getChunkSnapshot(), limits);
        }
        return envelope;
    }

    public static void write(
        final WritableByteChannel channel,
        final Envelope envelope
    ) throws IOException {
        write(channel, envelope, FrameLimits.DEFAULT);
    }

    public static void write(
        final WritableByteChannel channel,
        final Envelope envelope,
        final FrameLimits limits
    ) throws IOException {
        final FrameClass frameClass = FrameClass.forPayload(envelope);
        if (envelope.getPayloadCase() == Envelope.PayloadCase.CHUNK_SNAPSHOT) {
            validateSnapshot(envelope.getChunkSnapshot(), limits);
        }
        final int length = envelope.getSerializedSize();
        if (length == 0) {
            throw ProtocolException.malformedLength("zero length");
        }
        final long maximum = frameClass.maximum(limits);
        if ((long) length > maximum) {
            throw ProtocolException.declaredLength(length, maximum);
        }
        final byte[] encoded = envelope.toByteArray();
        final ByteBuffer prefix = ByteBuffer.allocate(PREFIX_BYTES);
        prefix.put((byte) frameClass.value);
        prefix.putInt(length);
        prefix.flip();
        writeFully(channel, prefix);
        writeFully(channel, ByteBuffer.wrap(encoded));
    }

    private static void readFully(
        final ReadableByteChannel channel,
        final ByteBuffer buffer
    ) throws IOException {
        final int expected = buffer.remaining();
        while (buffer.hasRemaining()) {
            final int count = channel.read(buffer);
            if (count == 0) {
                throw ProtocolException.noProgress(expected, buffer.position());
            }
            if (count < 0) {
                throw ProtocolException.earlyEof(expected, buffer.position());
            }
        }
    }

    private static void writeFully(
        final WritableByteChannel channel,
        final ByteBuffer buffer
    ) throws IOException {
        while (buffer.hasRemaining()) {
            final int count = channel.write(buffer);
            if (count == 0) {
                throw ProtocolException.noProgress(buffer.limit(), buffer.position());
            }
            if (count < 0) {
                throw new IOException("channel closed while writing frame");
            }
        }
    }

    private static void validateSnapshot(
        final ChunkSnapshot snapshot,
        final FrameLimits limits
    ) throws ProtocolException {
        final ByteString compressedBody = snapshot.getCompressedBody();
        final int compressedLength = compressedBody.size();
        if (compressedLength == 0) {
            throw ProtocolException.snapshot("compressed body is empty");
        }
        if ((long) compressedLength > limits.maxSnapshotBytes()) {
            throw ProtocolException.snapshot("compressed body exceeds snapshot limit");
        }

        final long declaredLength = Integer.toUnsignedLong(snapshot.getUncompressedLength());
        if (declaredLength > limits.maxUncompressedSnapshotBytes()) {
            throw ProtocolException.snapshot("declared uncompressed length exceeds absolute limit");
        }
        final long ratioLimit;
        try {
            ratioLimit = Math.multiplyExact(
                (long) compressedLength,
                FrameLimits.MAX_SNAPSHOT_DECOMPRESSION_RATIO
            );
        } catch (final ArithmeticException exception) {
            throw ProtocolException.snapshot("decompression ratio overflow");
        }
        if (declaredLength > ratioLimit) {
            throw ProtocolException.snapshot("decompression ratio exceeds 4096");
        }
        if (declaredLength > Integer.MAX_VALUE) {
            throw ProtocolException.snapshot("declared length cannot be represented");
        }

        final byte[] compressed = compressedBody.toByteArray();
        validateSingleStandardZstdFrame(compressed);
        final byte[] uncompressed = declaredLength == 0
            ? EMPTY_BYTES
            : new byte[(int) declaredLength];
        // Aircompressor skips input parsing for a zero-capacity output. A one-byte
        // probe forces complete frame validation while preserving a logical empty body.
        final byte[] decompressionOutput = declaredLength == 0 ? new byte[1] : uncompressed;
        final int decompressedLength;
        try {
            decompressedLength = ZSTD_DECOMPRESSOR.get().decompress(
                compressed,
                0,
                compressed.length,
                decompressionOutput,
                0,
                decompressionOutput.length
            );
        } catch (final RuntimeException exception) {
            throw ProtocolException.snapshot("invalid zstd body: " + exception.getMessage());
        }
        if (decompressedLength != declaredLength) {
            throw ProtocolException.snapshot("decompressed length does not match declaration");
        }

        final CRC32C crc32c = new CRC32C();
        crc32c.update(uncompressed, 0, uncompressed.length);
        final long actualCrc = crc32c.getValue();
        final long expectedCrc = Integer.toUnsignedLong(snapshot.getCrc32C());
        if (actualCrc != expectedCrc) {
            throw ProtocolException.snapshot("CRC32C does not match declaration");
        }
        try {
            ChunkSnapshotBody.parseFrom(uncompressed);
        } catch (final InvalidProtocolBufferException exception) {
            throw ProtocolException.snapshot("invalid snapshot body protobuf: " + exception.getMessage());
        }
    }

    private static void validateSingleStandardZstdFrame(
        final byte[] compressed
    ) throws ProtocolException {
        if (compressed.length < 5
            || compressed[0] != 0x28
            || (compressed[1] & 0xff) != 0xb5
            || (compressed[2] & 0xff) != 0x2f
            || (compressed[3] & 0xff) != 0xfd) {
            throw ProtocolException.snapshot("invalid zstd body: bad frame magic");
        }

        int cursor = 4;
        final int descriptor = compressed[cursor++] & 0xff;
        if ((descriptor & 0x18) != 0) {
            throw ProtocolException.snapshot("invalid zstd body: reserved frame-header bit");
        }
        final boolean singleSegment = (descriptor & 0x20) != 0;
        final boolean checksum = (descriptor & 0x04) != 0;
        final int contentSizeFlag = descriptor >>> 6;

        if (!singleSegment) {
            cursor = advanceZstdCursor(compressed, cursor, 1);
            final int windowDescriptor = compressed[cursor - 1] & 0xff;
            final int exponent = windowDescriptor >>> 3;
            final int mantissa = windowDescriptor & 0x07;
            final long windowBase = 1L << (10 + exponent);
            final long windowSize = windowBase + (windowBase >>> 3) * mantissa;
            if (windowSize > FrameLimits.MAX_ZSTD_WINDOW_BYTES) {
                throw ProtocolException.snapshot("zstd window exceeds shared limit");
            }
        }

        final int dictionaryLength = switch (descriptor & 0x03) {
            case 0 -> 0;
            case 1 -> 1;
            case 2 -> 2;
            case 3 -> 4;
            default -> throw new AssertionError();
        };
        cursor = advanceZstdCursor(compressed, cursor, dictionaryLength);
        final int contentSizeLength = switch (contentSizeFlag) {
            case 0 -> singleSegment ? 1 : 0;
            case 1 -> 2;
            case 2 -> 4;
            case 3 -> 8;
            default -> throw new AssertionError();
        };
        final int contentSizeOffset = cursor;
        cursor = advanceZstdCursor(compressed, cursor, contentSizeLength);
        if (singleSegment) {
            long contentSize = littleEndianAtMost(
                compressed,
                contentSizeOffset,
                contentSizeLength,
                FrameLimits.MAX_ZSTD_WINDOW_BYTES
            );
            if (contentSizeFlag == 1) {
                contentSize += 256;
            }
            if (contentSize > FrameLimits.MAX_ZSTD_WINDOW_BYTES) {
                throw ProtocolException.snapshot("zstd window exceeds shared limit");
            }
        }

        boolean lastBlock;
        do {
            final int blockHeaderOffset = cursor;
            cursor = advanceZstdCursor(compressed, cursor, 3);
            final int blockHeader = (compressed[blockHeaderOffset] & 0xff)
                | (compressed[blockHeaderOffset + 1] & 0xff) << 8
                | (compressed[blockHeaderOffset + 2] & 0xff) << 16;
            lastBlock = (blockHeader & 1) != 0;
            final int blockType = (blockHeader >>> 1) & 0x03;
            final int blockSize = blockHeader >>> 3;
            final int storedSize = switch (blockType) {
                case 0, 2 -> blockSize;
                case 1 -> 1;
                default -> throw ProtocolException.snapshot("invalid zstd body: reserved block type");
            };
            cursor = advanceZstdCursor(compressed, cursor, storedSize);
        } while (!lastBlock);

        if (checksum) {
            cursor = advanceZstdCursor(compressed, cursor, 4);
        }
        if (cursor != compressed.length) {
            throw ProtocolException.snapshot("invalid zstd body: trailing or concatenated frame");
        }
    }

    private static int advanceZstdCursor(
        final byte[] compressed,
        final int cursor,
        final int count
    ) throws ProtocolException {
        if (count < 0 || cursor < 0 || cursor > compressed.length || count > compressed.length - cursor) {
            throw ProtocolException.snapshot("invalid zstd body: truncated frame");
        }
        return cursor + count;
    }

    private static long littleEndianAtMost(
        final byte[] bytes,
        final int offset,
        final int length,
        final long maximum
    ) {
        long value = 0;
        for (int index = 0; index < length; index++) {
            final int next = bytes[offset + index] & 0xff;
            if (index >= 4 && next != 0) {
                return maximum + 1;
            }
            if (index < 4) {
                value |= (long) next << (index * 8);
            }
        }
        return value > maximum ? maximum + 1 : value;
    }

    private enum FrameClass {
        CONTROL(0),
        CHUNK_SNAPSHOT(1);

        private final int value;

        FrameClass(final int value) {
            this.value = value;
        }

        private static FrameClass fromByte(final int value) throws ProtocolException {
            return switch (value) {
                case 0 -> CONTROL;
                case 1 -> CHUNK_SNAPSHOT;
                default -> throw ProtocolException.malformedClass(value);
            };
        }

        private static FrameClass forPayload(final Envelope envelope) {
            return envelope.getPayloadCase() == Envelope.PayloadCase.CHUNK_SNAPSHOT
                ? CHUNK_SNAPSHOT
                : CONTROL;
        }

        private long maximum(final FrameLimits limits) {
            return this == CONTROL ? limits.maxControlBytes() : limits.maxSnapshotBytes();
        }
    }

    /** Checked protocol failure with a machine-readable category and byte counts. */
    public static final class ProtocolException extends IOException {
        private final String reason;
        private final long expected;
        private final long actual;

        private ProtocolException(
            final String reason,
            final String message,
            final long expected,
            final long actual,
            final Throwable cause
        ) {
            super(message, cause);
            this.reason = reason;
            this.expected = expected;
            this.actual = actual;
        }

        public String reason() {
            return this.reason;
        }

        public long expected() {
            return this.expected;
        }

        public long actual() {
            return this.actual;
        }

        private static ProtocolException malformedClass(final int value) {
            return new ProtocolException("malformed class", "unknown frame class " + value, 0, 0, null);
        }

        private static ProtocolException malformedLength(final String detail) {
            return new ProtocolException("malformed length", detail, 0, 0, null);
        }

        private static ProtocolException declaredLength(final long length, final long maximum) {
            return new ProtocolException(
                "declared length",
                "declared frame length " + length + " exceeds " + maximum,
                maximum,
                length,
                null
            );
        }

        private static ProtocolException earlyEof(final long expected, final long actual) {
            return new ProtocolException("early EOF", "early EOF", expected, actual, null);
        }


        private static ProtocolException noProgress(final long expected, final long actual) {
            return new ProtocolException("no progress", "channel made no progress", expected, actual, null);
        }

        private static ProtocolException protobuf(final Throwable cause) {
            return new ProtocolException("protobuf", "invalid envelope protobuf", 0, 0, cause);
        }

        private static ProtocolException classMismatch(
            final FrameClass frameClass,
            final FrameClass payloadClass
        ) {
            return new ProtocolException(
                "class mismatch",
                "frame class " + frameClass + " mismatches payload " + payloadClass,
                0,
                0,
                null
            );
        }

        private static ProtocolException snapshot(final String detail) {
            return new ProtocolException("snapshot validation", detail, 0, 0, null);
        }
    }
}
