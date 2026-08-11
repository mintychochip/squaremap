package xyz.jpenilla.squaremap.common.bridge.protocol;

/** Bounded sizes shared by the Java and Rust bridge codecs. */
public final class FrameLimits {
    public static final long MAX_CONTROL_BYTES = 1_048_576L;
    public static final long MAX_SNAPSHOT_BYTES = 67_108_864L;
    public static final long MAX_UNCOMPRESSED_SNAPSHOT_BYTES = 134_217_728L;
    public static final long MAX_SNAPSHOT_DECOMPRESSION_RATIO = 4096L;

    public static final FrameLimits DEFAULT = new FrameLimits(
        MAX_CONTROL_BYTES,
        MAX_SNAPSHOT_BYTES,
        MAX_UNCOMPRESSED_SNAPSHOT_BYTES
    );

    private final long maxControlBytes;
    private final long maxSnapshotBytes;
    private final long maxUncompressedSnapshotBytes;

    public FrameLimits() {
        this(MAX_CONTROL_BYTES, MAX_SNAPSHOT_BYTES, MAX_UNCOMPRESSED_SNAPSHOT_BYTES);
    }

    public FrameLimits(
        final long maxControlBytes,
        final long maxSnapshotBytes,
        final long maxUncompressedSnapshotBytes
    ) {
        if (maxControlBytes < 1 || maxControlBytes > 0xffff_ffffL
            || maxSnapshotBytes < 1 || maxSnapshotBytes > 0xffff_ffffL
            || maxUncompressedSnapshotBytes < 1 || maxUncompressedSnapshotBytes > 0xffff_ffffL) {
            throw new IllegalArgumentException("frame limits must fit unsigned 32-bit lengths");
        }
        this.maxControlBytes = maxControlBytes;
        this.maxSnapshotBytes = maxSnapshotBytes;
        this.maxUncompressedSnapshotBytes = maxUncompressedSnapshotBytes;
    }

    public long maxControlBytes() {
        return this.maxControlBytes;
    }

    public long maxSnapshotBytes() {
        return this.maxSnapshotBytes;
    }

    public long maxUncompressedSnapshotBytes() {
        return this.maxUncompressedSnapshotBytes;
    }
}
