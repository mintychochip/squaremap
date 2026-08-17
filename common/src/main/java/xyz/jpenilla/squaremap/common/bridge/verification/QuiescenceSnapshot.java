package xyz.jpenilla.squaremap.common.bridge.verification;

import java.util.Objects;

/** Immutable observation of all state required before declaring a lifecycle barrier quiet. */
public record QuiescenceSnapshot(
    boolean ready,
    boolean bridgeIdle,
    boolean renderIdle,
    boolean dirtyIdle,
    boolean outputsStable,
    boolean temporaryFilesAbsent,
    boolean recorderFlushed,
    boolean lifecycleComplete,
    boolean metricsAvailable,
    String outputHash,
    long outputRevision,
    String failure
) {
    public QuiescenceSnapshot {
        outputHash = Objects.requireNonNull(outputHash, "outputHash");
        failure = Objects.requireNonNull(failure, "failure");
    }

    public static QuiescenceSnapshot complete(final String outputHash, final long outputRevision) {
        return new QuiescenceSnapshot(true, true, true, true, true, true, true, true, true,
            outputHash, outputRevision, "");
    }

    public static QuiescenceSnapshot incomplete(final String failure) {
        return new QuiescenceSnapshot(false, false, false, false, false, false, false, false, false,
            "", -1L, failure);
    }

    public boolean isComplete() {
        return this.ready && this.bridgeIdle && this.renderIdle && this.dirtyIdle
            && this.outputsStable && this.temporaryFilesAbsent && this.recorderFlushed
            && this.lifecycleComplete && this.metricsAvailable && this.failure.isEmpty();
    }
}
