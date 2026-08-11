package xyz.jpenilla.squaremap.common.bridge.process;

/** Validates the independent monotonically increasing sequence emitted by Rust. */
final class InboundSequenceTracker {
    private long last;
    private boolean initialized;

    InboundSequenceTracker() {
    }

    InboundSequenceTracker(final long expected) {
        this.last = expected - 1L;
        this.initialized = true;
    }
    boolean accept(final long sequence) {
        if (!this.initialized) {
            this.last = sequence;
            this.initialized = true;
            return true;
        }
        if (sequence != this.last + 1L) return false;
        this.last = sequence;
        return true;
    }
}
