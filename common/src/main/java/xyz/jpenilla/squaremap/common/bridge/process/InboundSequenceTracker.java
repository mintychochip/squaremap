package xyz.jpenilla.squaremap.common.bridge.process;

/** Validates the independent monotonically increasing sequence emitted by Rust. */
final class InboundSequenceTracker {
    private long last;
    private boolean initialized;

    InboundSequenceTracker() {
    }

    InboundSequenceTracker(final long expected) {
        if (expected <= 0L) throw new IllegalArgumentException("sequence must be positive");
        this.last = expected - 1L;
        this.initialized = true;
    }
    boolean accept(final long sequence) {
        if (!this.initialized) {
            this.last = sequence;
            this.initialized = true;
            return true;
        }
        if (this.last == Long.MAX_VALUE || sequence != this.last + 1L) return false;
        this.last = sequence;
        return true;
    }
}
