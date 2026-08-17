package xyz.jpenilla.squaremap.common.bridge.process;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

final class InboundSequenceTrackerTest {
    @Test
    void acceptsStrictlyIncreasingSequencesAndRejectsDuplicatesOrGaps() {
        final InboundSequenceTracker tracker = new InboundSequenceTracker();
        assertTrue(tracker.accept(4));
        assertTrue(tracker.accept(5));
        assertFalse(tracker.accept(5));
        assertFalse(tracker.accept(7));
    }

    @Test
    void enforcesHandshakeDerivedInitialSequence() {
        final InboundSequenceTracker tracker = new InboundSequenceTracker(8L);
        assertFalse(tracker.accept(7L));
        assertTrue(tracker.accept(8L));
    }

    @Test
    void rejectsOverflowAfterMaximumSequence() {
        final InboundSequenceTracker tracker = new InboundSequenceTracker(Long.MAX_VALUE);
        assertFalse(tracker.accept(Long.MIN_VALUE));
    }

    @Test
    void rejectsHandshakeSequenceOverflow() {
        assertThrows(IllegalArgumentException.class, () -> new InboundSequenceTracker(-1L));
    }
}
