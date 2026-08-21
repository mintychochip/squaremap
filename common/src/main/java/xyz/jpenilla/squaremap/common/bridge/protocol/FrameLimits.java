package xyz.jpenilla.squaremap.common.bridge.protocol;

import java.util.Objects;
import xyz.jpenilla.squaremap.bridge.v1.BridgePolicyReplace;

/** Bounded sizes shared by the Java and Rust bridge codecs. */
public final class FrameLimits {
    public static final long MAX_CONTROL_BYTES = 1_048_576L;
    public static final long MAX_SNAPSHOT_BYTES = 67_108_864L;
    public static final long MAX_UNCOMPRESSED_SNAPSHOT_BYTES = 134_217_728L;
    public static final long MAX_SNAPSHOT_DECOMPRESSION_RATIO = 4096L;
    /** Maximum zstd back-reference window shared with the Rust decoder. */
    public static final int MAX_ZSTD_WINDOW_LOG = 23;
    public static final long MAX_ZSTD_WINDOW_BYTES = 1L << MAX_ZSTD_WINDOW_LOG;

    public static final FrameLimits DEFAULT = new FrameLimits(
        MAX_CONTROL_BYTES,
        MAX_SNAPSHOT_BYTES,
        MAX_UNCOMPRESSED_SNAPSHOT_BYTES
    );

    /** Adopts peer-advertised budgets, clamped to the codec maxima; zero means the codec maximum. */
    public static FrameLimits fromPeerPolicy(final BridgePolicyReplace policy) {
        Objects.requireNonNull(policy, "policy");
        return new FrameLimits(
            clampPeerBudget(policy.getMaxControlFrameBytes(), MAX_CONTROL_BYTES),
            clampPeerBudget(policy.getMaxSnapshotFrameBytes(), MAX_SNAPSHOT_BYTES),
            clampPeerBudget(policy.getMaxUncompressedSnapshotBytes(), MAX_UNCOMPRESSED_SNAPSHOT_BYTES)
        );
    }

    private static long clampPeerBudget(final long peerValue, final long maximum) {
        if (peerValue == 0) {
            return maximum;
        }
        return Math.min(peerValue, maximum);
    }

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
