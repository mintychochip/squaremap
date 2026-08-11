package xyz.jpenilla.squaremap.common.bridge.outbox;

import java.time.Duration;
import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

class CoalescingOutboxTest {
    private static final BridgeEvent.WorldKey WORLD = new BridgeEvent.WorldKey("minecraft", "overworld");

    @Test
    void repeatedCoordinateKeepsNewestRevision() {
        final CoalescingOutbox outbox = new CoalescingOutbox();
        for (long revision = 1; revision <= 100; revision++) {
            assertEquals(revision == 1 ? BridgePublisher.PublishResult.ACCEPTED : BridgePublisher.PublishResult.COALESCED,
                outbox.offer(new BridgeEvent.DirtyChunk(WORLD, 7, 4, -2, revision)));
        }
        final List<BridgeEvent> drained = outbox.drain();
        assertEquals(List.of(new BridgeEvent.DirtyChunk(WORLD, 7, 4, -2, 100)), drained);
    }

    @Test
    void replacementStateKeepsLatestPayload() {
        final CoalescingOutbox outbox = new CoalescingOutbox();
        final Envelope first = Envelope.newBuilder().setSequence(1).build();
        final Envelope latest = Envelope.newBuilder().setSequence(9).build();
        outbox.offer(new BridgeEvent.ReplaceState("players", first));
        assertEquals(BridgePublisher.PublishResult.COALESCED,
            outbox.offer(new BridgeEvent.ReplaceState("players", latest)));
        assertEquals(List.of(new BridgeEvent.ReplaceState("players", latest)), outbox.drain());
    }

    @Test
    void overflowCollapsesOnlyWorldEpochAndPostOverflowDirtiesDoNotRegrow() {
        final CoalescingOutbox outbox = new CoalescingOutbox();
        for (int index = 0; index < CoalescingOutbox.MAX_DIRTY_KEYS; index++) {
            outbox.offer(new BridgeEvent.DirtyChunk(WORLD, 3, index, 0, 1));
        }
        assertEquals(BridgePublisher.PublishResult.RESYNC_MARKED,
            outbox.offer(new BridgeEvent.DirtyChunk(WORLD, 3, CoalescingOutbox.MAX_DIRTY_KEYS, 0, 1)));
        for (int index = CoalescingOutbox.MAX_DIRTY_KEYS + 1; index < CoalescingOutbox.MAX_DIRTY_KEYS + 100; index++) {
            assertEquals(BridgePublisher.PublishResult.COALESCED,
                outbox.offer(new BridgeEvent.DirtyChunk(WORLD, 3, index, 0, 1)));
        }
        final List<BridgeEvent> drained = outbox.drain();
        assertEquals(1, drained.size());
        assertEquals(new BridgeEvent.ResyncWorld(WORLD, 3), drained.get(0));
        assertEquals(0, outbox.dirtyKeyCount());
    }

    @Test
    void drainAssignsMonotonicSequencesAndAckRetainsPendingUntilMatchingValue() {
        final List<BridgePublisher.Sent> sent = new CopyOnWriteArrayList<>();
        final BridgePublisher publisher = new BridgePublisher(sent::add);
        try {
            final Envelope first = Envelope.newBuilder().setSequence(11).build();
            final Envelope newer = Envelope.newBuilder().setSequence(12).build();
            publisher.publish(new BridgeEvent.ReplaceState("config", first));
            assertTrue(publisher.awaitSent(1, Duration.ofSeconds(2)));
            publisher.publish(new BridgeEvent.ReplaceState("config", newer));
            assertTrue(sent.get(0).event().payload() instanceof Envelope);
            publisher.acknowledge(sent.get(0).sequence());
            assertTrue(publisher.hasPending());
            publisher.drainNow();
            assertTrue(publisher.awaitSent(2, Duration.ofSeconds(2)));
            assertEquals(2, sent.get(1).sequence());
            publisher.acknowledge(sent.get(1).sequence());
            assertFalse(publisher.hasPending());
        } finally {
            publisher.close();
        }
    }

    @Test
    void staleAckCannotEraseNewerCoalescedRevision() {
        final List<BridgePublisher.Sent> sent = new CopyOnWriteArrayList<>();
        final BridgePublisher publisher = new BridgePublisher(sent::add);
        try {
            publisher.publish(new BridgeEvent.DirtyChunk(WORLD, 1, 0, 0, 1));
            assertTrue(publisher.awaitSent(1, Duration.ofSeconds(2)));
            publisher.publish(new BridgeEvent.DirtyChunk(WORLD, 1, 0, 0, 2));
            publisher.acknowledge(sent.get(0).sequence());
            assertTrue(publisher.hasPending());
            publisher.drainNow();
            assertTrue(publisher.awaitSent(2, Duration.ofSeconds(2)));
            assertEquals(2, ((BridgeEvent.DirtyChunk) sent.get(1).event().payload()).revision());
        } finally {
            publisher.close();
        }
    }

    @Test
    void reconnectUsesFreshSessionAndResendsCurrentBaseline() {
        final List<BridgePublisher.Sent> sent = new CopyOnWriteArrayList<>();
        final BridgePublisher publisher = new BridgePublisher(sent::add);
        try {
            final byte[] first = publisher.sessionId();
            publisher.publish(new BridgeEvent.ReplaceState("worlds", Envelope.getDefaultInstance()));
            assertTrue(publisher.awaitSent(1, Duration.ofSeconds(2)));
            publisher.reconnect();
            final byte[] second = publisher.sessionId();
            assertEquals(16, second.length);
            assertFalse(java.util.Arrays.equals(first, second));
            assertTrue(publisher.awaitSent(2, Duration.ofSeconds(2)));
            assertEquals(1, sent.get(1).sequence());
            assertEquals(sent.get(0).event().payload(), sent.get(1).event().payload());
        } finally {
            publisher.close();
        }
    }

    @Test
    void publishDoesNotPerformWriterIoAndCloseTerminatesOwnedWriter() throws Exception {
        final CountDownLatch writerEntered = new CountDownLatch(1);
        final AtomicInteger writes = new AtomicInteger();
        final BridgePublisher publisher = new BridgePublisher(sent -> {
            writes.incrementAndGet();
            writerEntered.countDown();
        });
        publisher.publish(new BridgeEvent.ResyncWorld(WORLD, 1));
        assertEquals(0, writes.get());
        assertTrue(writerEntered.await(2, TimeUnit.SECONDS));
        publisher.close();
        publisher.close();
        assertTrue(publisher.isClosed());
        assertFalse(publisher.hasWriterThread());
    }
}
