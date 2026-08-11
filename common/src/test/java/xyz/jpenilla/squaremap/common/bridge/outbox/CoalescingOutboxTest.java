package xyz.jpenilla.squaremap.common.bridge.outbox;

import java.lang.reflect.Field;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.bridge.v1.Ack;
import xyz.jpenilla.squaremap.bridge.v1.AckStatus;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.Hello;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

class CoalescingOutboxTest {
    private static final BridgeEvent.WorldKey WORLD = new BridgeEvent.WorldKey("minecraft", "overworld");
    private static final byte[] SESSION = bytes(1);

    @Test
    void repeatedCoordinateKeepsNewestRevision() {
        final CoalescingOutbox outbox = new CoalescingOutbox();
        for (long revision = 1; revision <= 100; revision++) {
            assertEquals(revision == 1 ? BridgePublisher.PublishResult.ACCEPTED : BridgePublisher.PublishResult.COALESCED,
                outbox.offer(new BridgeEvent.DirtyChunk(WORLD, 7, 4, -2, revision)));
        }
        assertEquals(List.of(new BridgeEvent.DirtyChunk(WORLD, 7, 4, -2, 100)), outbox.drain());
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
    void replacementBaselineDrainsWorldsBeforeEpochDependentViews() {
        final CoalescingOutbox outbox = new CoalescingOutbox();
        outbox.offer(new BridgeEvent.ReplaceState("players", Envelope.getDefaultInstance()));
        outbox.offer(new BridgeEvent.ReplaceState("markers:minecraft:overworld", Envelope.getDefaultInstance()));
        outbox.offer(new BridgeEvent.ReplaceState("icons", Envelope.getDefaultInstance()));
        outbox.offer(new BridgeEvent.ReplaceState("worlds", Envelope.getDefaultInstance()));
        assertEquals(List.of("worlds", "markers:minecraft:overworld", "players", "icons"),
            outbox.drain().stream().map(event -> ((BridgeEvent.ReplaceState) event).key()).toList());
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
        assertEquals(List.of(new BridgeEvent.ResyncWorld(WORLD, 3)), outbox.drain());
        assertEquals(0, outbox.dirtyKeyCount());
    }
    @Test
    void capacityCountsQueuedAndInFlightDirtyUnionOnce() throws Exception {
        final CountDownLatch entered = new CountDownLatch(1);
        final CountDownLatch release = new CountDownLatch(1);
        final BridgePublisher publisher = new BridgePublisher(SESSION, new BridgePublisher.Writer() {
            @Override
            public void write(final BridgePublisher.Sent value) throws Exception {
                if (value.sequence() == 1) {
                    entered.countDown();
                    release.await();
                }
            }
            @Override
            public void close() {
                release.countDown();
            }
        });
        try {
            publisher.publish(new BridgeEvent.DirtyChunk(WORLD, 5, 0, 0, 1));
            assertTrue(entered.await(2, TimeUnit.SECONDS));
            for (int index = 1; index < CoalescingOutbox.MAX_DIRTY_KEYS - 1; index++) {
                publisher.publish(new BridgeEvent.DirtyChunk(WORLD, 5, index, 0, 1));
            }
            for (int index = 0; index < CoalescingOutbox.MAX_DIRTY_KEYS - 1; index++) {
                publisher.publish(new BridgeEvent.DirtyChunk(WORLD, 5, index, 0, 2));
            }
            assertEquals(BridgePublisher.PublishResult.ACCEPTED,
                publisher.publish(new BridgeEvent.DirtyChunk(WORLD, 5, CoalescingOutbox.MAX_DIRTY_KEYS, 0, 1)));
        } finally {
            release.countDown();
            publisher.close();
        }
    }

    @Test
    void lowerRevisionCannotFollowHigherInFlightRevision() {
        final List<BridgePublisher.Sent> sent = new CopyOnWriteArrayList<>();
        final BridgePublisher publisher = publisher(sent);
        try {
            publisher.publish(new BridgeEvent.DirtyChunk(WORLD, 1, 0, 0, 2));
            assertTrue(publisher.awaitSent(1, Duration.ofSeconds(2)));
            publisher.publish(new BridgeEvent.DirtyChunk(WORLD, 1, 0, 0, 1));
            publisher.acknowledge(ack(sent.get(0), AckStatus.ACK_STATUS_ACCEPTED));
            assertFalse(publisher.hasPending());
            assertEquals(1, sent.size());
        } finally {
            publisher.close();
        }
    }

    @Test
    void drainAssignsMonotonicSequencesAndAckRetainsPendingUntilMatchingAck() {
        final List<BridgePublisher.Sent> sent = new CopyOnWriteArrayList<>();
        final BridgePublisher publisher = publisher(sent);
        try {
            publisher.publish(new BridgeEvent.ReplaceState("one", Envelope.getDefaultInstance()));
            publisher.publish(new BridgeEvent.ReplaceState("two", Envelope.getDefaultInstance()));
            assertTrue(publisher.awaitSent(2, Duration.ofSeconds(2)));
            assertEquals(1, sent.get(0).sequence());
            assertEquals(2, sent.get(1).sequence());
            publisher.acknowledge(ack(sent.get(0), AckStatus.ACK_STATUS_ACCEPTED));
            assertTrue(publisher.hasPending());
            publisher.acknowledge(ack(sent.get(1), AckStatus.ACK_STATUS_ACCEPTED));
            assertFalse(publisher.hasPending());
        } finally {
            publisher.close();
        }
    }
    @Test
    void ackRequiresCurrentSessionAndSuccessfulStatus() {
        final List<BridgePublisher.Sent> sent = new CopyOnWriteArrayList<>();
        final BridgePublisher publisher = publisher(sent);
        try {
            publisher.publish(new BridgeEvent.ReplaceState("config", Envelope.getDefaultInstance()));
            assertTrue(publisher.awaitSent(1, Duration.ofSeconds(2)));
            final BridgePublisher.Sent value = sent.get(0);
            publisher.acknowledge(Envelope.newBuilder().setSessionId(com.google.protobuf.ByteString.copyFrom(bytes(2)))
                .setSequence(value.sequence()).setAck(Ack.newBuilder().setAcknowledgedSequence(value.sequence())
                    .setStatus(AckStatus.ACK_STATUS_ACCEPTED)).build());
            assertTrue(publisher.hasPending());
            for (AckStatus status : List.of(AckStatus.ACK_STATUS_REJECTED, AckStatus.ACK_STATUS_UNSPECIFIED)) {
                publisher.acknowledge(ack(value, status));
                assertTrue(publisher.hasPending());
            }
            publisher.acknowledge(ack(value, AckStatus.ACK_STATUS_ACCEPTED));
            assertFalse(publisher.hasPending());
        } finally {
            publisher.close();
        }
    }

    @Test
    void reconnectUsesSuppliedFreshSessionAndResendsReplacementBaseline() {
        final List<BridgePublisher.Sent> sent = new CopyOnWriteArrayList<>();
        final BridgePublisher publisher = publisher(sent);
        try {
            publisher.publish(new BridgeEvent.ReplaceState("worlds", Envelope.newBuilder()
                .setCorrelationId(55).setHello(Hello.getDefaultInstance()).build()));
            assertTrue(publisher.awaitSent(1, Duration.ofSeconds(2)));
            publisher.reconnect(bytes(2));
            assertArrayNotEquals(SESSION, publisher.sessionId());
            assertTrue(publisher.awaitSent(2, Duration.ofSeconds(2)));
            assertEquals(1, sent.get(1).sequence());
            assertEquals(1, sent.get(1).envelope().getProtocolMajor());
            assertEquals(0, sent.get(1).envelope().getProtocolMinor());
            assertEquals(55, sent.get(1).envelope().getCorrelationId());
        } finally {
            publisher.close();
        }
    }

    @Test
    void acknowledgedDirtyHistoryDoesNotResendOnReconnect() {
        final List<BridgePublisher.Sent> sent = new CopyOnWriteArrayList<>();
        final BridgePublisher publisher = publisher(sent);
        try {
            for (int index = 0; index < 100; index++) {
                publisher.publish(new BridgeEvent.DirtyChunk(WORLD, 2, index, 0, 1));
                assertTrue(publisher.awaitSent(index + 1, Duration.ofSeconds(2)));
                publisher.acknowledge(ack(sent.get(index), AckStatus.ACK_STATUS_ACCEPTED));
            }
            publisher.reconnect(bytes(3));
            Thread.sleep(50L);
            assertEquals(100, sent.size());
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            throw new AssertionError(interrupted);
        } finally {
            publisher.close();
        }
    }

    @Test
    void resyncAckAllowsNewDirtyAndReconnectsThatDirty() {
        final List<BridgePublisher.Sent> sent = new CopyOnWriteArrayList<>();
        final BridgePublisher publisher = publisher(sent);
        try {
            publisher.publish(new BridgeEvent.ResyncWorld(WORLD, 4));
            assertTrue(publisher.awaitSent(1, Duration.ofSeconds(2)));
            publisher.acknowledge(ack(sent.get(0), AckStatus.ACK_STATUS_ACCEPTED));
            publisher.publish(new BridgeEvent.DirtyChunk(WORLD, 4, 1, 1, 9));
            assertTrue(publisher.awaitSent(2, Duration.ofSeconds(2)));
            publisher.reconnect(bytes(4));
            assertTrue(publisher.awaitSent(3, Duration.ofSeconds(2)));
            assertEquals(1, ((BridgeEvent.DirtyChunk) sent.get(2).event().payload()).x());
        } finally {
            publisher.close();
        }
    }

    @Test
    void writerFailurePreservesPendingAndExposesCause() {
        final RuntimeException failure = new RuntimeException("writer failed");
        final AtomicReference<BridgePublisher.Sent> first = new AtomicReference<>();
        final BridgePublisher publisher = new BridgePublisher(SESSION, new BridgePublisher.Writer() {
            @Override
            public void write(final BridgePublisher.Sent sent) {
                first.set(sent);
                throw failure;
            }
        });
        publisher.publish(new BridgeEvent.ReplaceState("config", Envelope.getDefaultInstance()));
        assertTrue(awaitClosed(publisher));
        assertEquals(failure, publisher.failure());
        assertTrue(publisher.hasPending());
        publisher.close();
    }

    @Test
    void blockedWriterCloseUnblocksAndTerminatesWorker() throws Exception {
        final CountDownLatch entered = new CountDownLatch(1);
        final CountDownLatch released = new CountDownLatch(1);
        final BridgePublisher publisher = new BridgePublisher(SESSION, new BridgePublisher.Writer() {
            @Override
            public void write(final BridgePublisher.Sent ignored) throws Exception {
                entered.countDown();
                released.await();
            }
            @Override
            public void close() {
                released.countDown();
            }
        });
        publisher.publish(new BridgeEvent.ResyncWorld(WORLD, 1));
        assertTrue(entered.await(2, TimeUnit.SECONDS));
        publisher.close();
        assertFalse(publisher.hasWriterThread());
    }

    @Test
    void writerRunsOnlyOnOwnedWorkerAndDispatchCannotBeReentered() throws Exception {
        final Thread caller = Thread.currentThread();
        final AtomicReference<Thread> writerThread = new AtomicReference<>();
        final CountDownLatch entered = new CountDownLatch(1);
        final BridgePublisher publisher = new BridgePublisher(SESSION, sent -> {
            writerThread.set(Thread.currentThread());
            entered.countDown();
        });
        try {
            publisher.publish(new BridgeEvent.ResyncWorld(WORLD, 1));
            assertTrue(entered.await(2, TimeUnit.SECONDS));
            assertNotEquals(caller, writerThread.get());
            assertThrows(NoSuchMethodException.class, () -> BridgePublisher.class.getMethod("drainNow"));
        } finally {
            publisher.close();
        }
    }

    @Test
    void reconnectWaitsForOldGenerationCallback() throws Exception {
        final CountDownLatch entered = new CountDownLatch(1);
        final CountDownLatch release = new CountDownLatch(1);
        final List<BridgePublisher.Sent> sent = new CopyOnWriteArrayList<>();
        final BridgePublisher publisher = new BridgePublisher(SESSION, new BridgePublisher.Writer() {
            @Override
            public void write(final BridgePublisher.Sent value) throws Exception {
                sent.add(value);
                if (value.sequence() == 1) {
                    entered.countDown();
                    release.await();
                }
            }
            @Override
            public void close() {
                release.countDown();
            }
        });
        try {
            publisher.publish(new BridgeEvent.ResyncWorld(WORLD, 1));
            assertTrue(entered.await(2, TimeUnit.SECONDS));
            final Thread reconnect = new Thread(() -> publisher.reconnect(bytes(5)), "reconnect-test");
            reconnect.start();
            Thread.sleep(20L);
            assertTrue(Arrays.equals(SESSION, publisher.sessionId()));
            release.countDown();
            reconnect.join(2000L);
            assertFalse(reconnect.isAlive());
            assertArrayNotEquals(SESSION, publisher.sessionId());
            publisher.publish(new BridgeEvent.ResyncWorld(WORLD, 2));
            assertTrue(publisher.awaitSent(2, Duration.ofSeconds(2)));
            assertEquals(1, sent.get(1).sequence());
            assertTrue(Arrays.equals(bytes(5), sent.get(1).envelope().getSessionId().toByteArray()));
        } finally {
            publisher.close();
        }
    }
    @Test
    void reconnectFromWriterIsRejectedWithoutDeadlock() {
        final AtomicReference<BridgePublisher> reference = new AtomicReference<>();
        final AtomicReference<Throwable> failure = new AtomicReference<>();
        final CountDownLatch written = new CountDownLatch(1);
        final BridgePublisher publisher = new BridgePublisher(SESSION, sent -> {
            try {
                reference.get().reconnect(bytes(6));
            } catch (final Throwable thrown) {
                failure.set(thrown);
            } finally {
                written.countDown();
            }
        });
        reference.set(publisher);
        try {
            publisher.publish(new BridgeEvent.ResyncWorld(WORLD, 1));
            assertTrue(written.await(2, TimeUnit.SECONDS));
            assertTrue(failure.get() instanceof IllegalStateException);
            assertTrue(publisher.awaitSent(1, Duration.ofSeconds(2)));
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            throw new AssertionError(interrupted);
        } finally {
            publisher.close();
        }
    }

    @Test
    void unacknowledgedOverflowMarkerSuppressesPostOverflowDirtiesOnReconnect() throws Exception {
        final CountDownLatch entered = new CountDownLatch(1);
        final CountDownLatch release = new CountDownLatch(1);
        final List<BridgePublisher.Sent> sent = new CopyOnWriteArrayList<>();
        final BridgePublisher publisher = new BridgePublisher(SESSION, new BridgePublisher.Writer() {
            @Override
            public void write(final BridgePublisher.Sent value) throws Exception {
                sent.add(value);
                if (value.sequence() == 1) {
                    entered.countDown();
                    release.await();
                }
            }
            @Override
            public void close() {
                release.countDown();
            }
        }, 2, 2);
        try {
            publisher.publish(new BridgeEvent.DirtyChunk(WORLD, 9, 0, 0, 1));
            assertTrue(entered.await(2, TimeUnit.SECONDS));
            publisher.publish(new BridgeEvent.DirtyChunk(WORLD, 9, 1, 0, 1));
            assertEquals(BridgePublisher.PublishResult.RESYNC_MARKED,
                publisher.publish(new BridgeEvent.DirtyChunk(WORLD, 9, 2, 0, 1)));
            for (int index = 3; index < 20; index++) {
                assertEquals(BridgePublisher.PublishResult.COALESCED,
                    publisher.publish(new BridgeEvent.DirtyChunk(WORLD, 9, index, 0, 1)));
            }
            assertEquals(0, currentDirtyCount(publisher));

            final Thread reconnect = new Thread(() -> publisher.reconnect(bytes(7)), "overflow-reconnect-test");
            reconnect.start();
            release.countDown();
            reconnect.join(2000L);
            assertFalse(reconnect.isAlive());
            assertTrue(publisher.awaitSent(3, Duration.ofSeconds(2)));
            assertTrue(sent.get(sent.size() - 1).event().payload() instanceof BridgeEvent.ResyncWorld);
        } finally {
            publisher.close();
        }
    }

    @Test
    void replacementKeysHaveDeterministicBoundAndCloseIsExactlyOnce() {
        final AtomicInteger closes = new AtomicInteger();
        final BridgePublisher publisher = new BridgePublisher(SESSION, new BridgePublisher.Writer() {
            @Override
            public void write(final BridgePublisher.Sent ignored) {}
            @Override
            public void close() {
                closes.incrementAndGet();
            }
        }, 4, 2);
        try {
            assertEquals(BridgePublisher.PublishResult.ACCEPTED,
                publisher.publish(new BridgeEvent.ReplaceState("one", Envelope.getDefaultInstance())));
            assertEquals(BridgePublisher.PublishResult.ACCEPTED,
                publisher.publish(new BridgeEvent.ReplaceState("two", Envelope.getDefaultInstance())));
            assertThrows(IllegalStateException.class,
                () -> publisher.publish(new BridgeEvent.ReplaceState("three", Envelope.getDefaultInstance())));
            assertFalse(publisher.isClosed());
        } finally {
            publisher.close();
            publisher.close();
        }
        assertEquals(1, closes.get());
    }

    @Test
    void sequenceExhaustionLeavesEventsPending() throws Exception {
        final List<BridgePublisher.Sent> sent = new CopyOnWriteArrayList<>();
        final BridgePublisher publisher = publisher(sent);
        try {
            final Field sequence = BridgePublisher.class.getDeclaredField("nextSequence");
            sequence.setAccessible(true);
            sequence.setLong(publisher, Long.MAX_VALUE);
            publisher.publish(new BridgeEvent.ResyncWorld(WORLD, 1));
            assertTrue(awaitClosed(publisher));
            assertTrue(publisher.hasPending());
            assertTrue(sent.isEmpty());
        } finally {
            publisher.close();
        }
    }

    private static BridgePublisher publisher(final List<BridgePublisher.Sent> sent) {
        return new BridgePublisher(SESSION, sent::add);
    }

    private static Envelope ack(final BridgePublisher.Sent sent, final AckStatus status) {
        return Envelope.newBuilder().setSessionId(sent.envelope().getSessionId()).setSequence(sent.sequence())
            .setAck(Ack.newBuilder().setAcknowledgedSequence(sent.sequence()).setStatus(status)).build();
    }

    private static boolean awaitClosed(final BridgePublisher publisher) {
        final long deadline = System.nanoTime() + Duration.ofSeconds(2).toNanos();
        while (System.nanoTime() < deadline && !publisher.isClosed()) {
            Thread.yield();
        }
        return publisher.isClosed();
    }

    private static int currentDirtyCount(final BridgePublisher publisher) throws Exception {
        final Field current = BridgePublisher.class.getDeclaredField("current");
        current.setAccessible(true);
        final Map<?, BridgeEvent> values = (Map<?, BridgeEvent>) current.get(publisher);
        int count = 0;
        for (final BridgeEvent event : values.values()) {
            if (event instanceof BridgeEvent.DirtyChunk) {
                count++;
            }
        }
        return count;
    }

    private static byte[] bytes(final int value) {
        final byte[] result = new byte[16];
        Arrays.fill(result, (byte) value);
        return result;
    }

    private static void assertArrayNotEquals(final byte[] expected, final byte[] actual) {
        assertFalse(Arrays.equals(expected, actual));
    }
}
