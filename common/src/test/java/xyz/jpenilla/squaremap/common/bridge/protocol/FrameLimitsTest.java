package xyz.jpenilla.squaremap.common.bridge.protocol;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.bridge.v1.BridgePolicyReplace;

class FrameLimitsTest {
    @Test
    void fromPeerPolicyClampsAboveCodecMaxima() {
        final BridgePolicyReplace policy = BridgePolicyReplace.newBuilder()
            .setMaxControlFrameBytes(Integer.MAX_VALUE)
            .setMaxSnapshotFrameBytes(Integer.MAX_VALUE)
            .setMaxUncompressedSnapshotBytes(Integer.MAX_VALUE)
            .build();
        final FrameLimits limits = FrameLimits.fromPeerPolicy(policy);
        assertEquals(FrameLimits.MAX_CONTROL_BYTES, limits.maxControlBytes());
        assertEquals(FrameLimits.MAX_SNAPSHOT_BYTES, limits.maxSnapshotBytes());
        assertEquals(FrameLimits.MAX_UNCOMPRESSED_SNAPSHOT_BYTES, limits.maxUncompressedSnapshotBytes());
    }

    @Test
    void fromPeerPolicyKeepsLowerPeerBudgets() {
        final BridgePolicyReplace policy = BridgePolicyReplace.newBuilder()
            .setMaxControlFrameBytes(4096)
            .setMaxSnapshotFrameBytes(8192)
            .setMaxUncompressedSnapshotBytes(16384)
            .build();
        final FrameLimits limits = FrameLimits.fromPeerPolicy(policy);
        assertEquals(4096, limits.maxControlBytes());
        assertEquals(8192, limits.maxSnapshotBytes());
        assertEquals(16384, limits.maxUncompressedSnapshotBytes());
    }

    @Test
    void fromPeerPolicyTreatsZeroAsDefaultMaxima() {
        final FrameLimits limits = FrameLimits.fromPeerPolicy(BridgePolicyReplace.getDefaultInstance());
        assertEquals(FrameLimits.DEFAULT.maxControlBytes(), limits.maxControlBytes());
        assertEquals(FrameLimits.DEFAULT.maxSnapshotBytes(), limits.maxSnapshotBytes());
        assertEquals(FrameLimits.DEFAULT.maxUncompressedSnapshotBytes(), limits.maxUncompressedSnapshotBytes());
        assertTrue(limits.maxControlBytes() >= 1);
    }
}
