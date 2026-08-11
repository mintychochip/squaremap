package xyz.jpenilla.squaremap.common.bridge.outbox;

import com.google.protobuf.ByteString;
import java.time.Duration;
import java.util.Arrays;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.function.Consumer;
import java.util.concurrent.Semaphore;
import xyz.jpenilla.squaremap.bridge.v1.Ack;
import xyz.jpenilla.squaremap.bridge.v1.AckStatus;
import xyz.jpenilla.squaremap.bridge.v1.ChunkCoordinate;
import xyz.jpenilla.squaremap.bridge.v1.ChunkDirty;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.ResyncReason;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;
import xyz.jpenilla.squaremap.bridge.v1.WorldResyncRequired;

/** Owns the only bridge writer worker and keeps durable values until acknowledgement. */
public final class BridgePublisher implements AutoCloseable {
    public static final int DEFAULT_MAX_REPLACEMENT_KEYS = 4_096;
    /** Replacement state is bounded to keep canonical snapshots in memory. */
    public static final int MAX_REPLACEMENT_KEYS = DEFAULT_MAX_REPLACEMENT_KEYS;
    public enum PublishResult {
        ACCEPTED,
        COALESCED,
        RESYNC_MARKED
    }
    public enum ControlDisposition {
        RECALLED,
        DISPATCHED
    }

    @FunctionalInterface
    public interface Writer extends AutoCloseable {
        void write(Sent sent) throws Exception;

        default void close() {}
    }

    public record Published(Object payload) {
        public Published {
            Objects.requireNonNull(payload, "payload");
        }
    }

    public record Sent(long sequence, Published event, Envelope envelope) {}

    private final Object lock = new Object();
    private final CoalescingOutbox outbox;
    private final Writer writer;
    private final Semaphore signal = new Semaphore(0);
    private final Thread worker;
    private final Map<Object, BridgeEvent> current = new HashMap<>();
    private final Map<Long, InFlight> inFlight = new HashMap<>();
    private final int maxDirtyKeys;
    private final int maxReplacementKeys;
    private volatile int policyMaxDirtyKeys = Integer.MAX_VALUE;
    private volatile int policyMaxReplacementKeys = Integer.MAX_VALUE;
    private boolean dispatching;
    private boolean signaled;
    private boolean closed;
    private boolean writerClosed;
    private Throwable failure;
    private Consumer<Throwable> failureListener = ignored -> {};
    private long sentCount;
    private long nextSequence;
    private long generation;
    private byte[] sessionId;

    public BridgePublisher(final byte[] sessionId, final Writer writer) {
        this(sessionId, writer, CoalescingOutbox.MAX_DIRTY_KEYS, DEFAULT_MAX_REPLACEMENT_KEYS);
    }
    BridgePublisher(final byte[] sessionId, final Writer writer, final int maxDirtyKeys, final int maxReplacementKeys) {
        if (maxReplacementKeys < 1) {
            throw new IllegalArgumentException("replacement-state bound must be positive");
        }
        this.maxDirtyKeys = maxDirtyKeys;
        this.maxReplacementKeys = maxReplacementKeys;
        this.outbox = new CoalescingOutbox(maxDirtyKeys, maxReplacementKeys);
        this.sessionId = validSession(sessionId);
        this.writer = Objects.requireNonNull(writer, "writer");
        this.worker = new Thread(this::runWriter, "squaremap-bridge-writer");
        this.worker.setDaemon(true);
        this.worker.start();
    }


    public PublishResult publish(final BridgeEvent event) {
        Objects.requireNonNull(event, "event");
        synchronized (this.lock) {
            this.ensureOpen();
            final PublishResult result = this.offerCurrent(event);
            this.signalLocked();
            return result;
        }
    }

    public byte[] sessionId() {
        synchronized (this.lock) {
            return this.sessionId.clone();
        }
    }

    public Throwable failure() {
        synchronized (this.lock) {
            return this.failure;
        }
    }
    public void setFailureListener(final Consumer<Throwable> listener) {
        this.failureListener = Objects.requireNonNull(listener, "listener");
    }
    public ControlDisposition cancelControl(final long correlationId) {
        synchronized (this.lock) {
            this.current.remove("control:" + correlationId);
            this.outbox.removeControl(correlationId);
            final boolean dispatched = this.inFlight.entrySet().removeIf(entry -> entry.getValue().event() instanceof BridgeEvent.Control control
                && control.correlationId() == correlationId);
            return dispatched ? ControlDisposition.DISPATCHED : ControlDisposition.RECALLED;
        }
    }
    public void applyPolicy(final xyz.jpenilla.squaremap.bridge.v1.BridgePolicyReplace policy) {
        this.policyMaxDirtyKeys = Math.max(1, policy.getMaxPendingDirtyChunks());
        this.policyMaxReplacementKeys = Math.max(1, policy.getSnapshotCredits());
    }

    public boolean hasPending() {
        synchronized (this.lock) {
            return !this.outbox.isEmpty() || !this.inFlight.isEmpty();
        }
    }

    public boolean awaitSent(final int count, final Duration timeout) {
        final long deadline = System.nanoTime() + timeout.toNanos();
        while (System.nanoTime() < deadline) {
            synchronized (this.lock) {
                if (this.sentCount >= count) {
                    return true;
                }
            }
            try {
                Thread.sleep(2L);
            } catch (final InterruptedException interrupted) {
                Thread.currentThread().interrupt();
                return false;
            }
        }
        return false;
    }

    /** Handles an authenticated Ack envelope from the currently active session. */
    public void acknowledge(final Envelope envelope) {
        Objects.requireNonNull(envelope, "envelope");
        if (!envelope.hasAck()) {
            return;
        }
        final Ack ack = envelope.getAck();
        synchronized (this.lock) {
            if (!java.util.Arrays.equals(this.sessionId, envelope.getSessionId().toByteArray())) {
                return;
            }
            if (ack.getStatus() != AckStatus.ACK_STATUS_ACCEPTED
                && ack.getStatus() != AckStatus.ACK_STATUS_DUPLICATE) {
                return;
            }
            final InFlight sent = this.inFlight.remove(ack.getAcknowledgedSequence());
            if (sent == null || sent.generation() != this.generation) {
                return;
            }
            final Object identity = identity(sent.event());
            if (Objects.equals(this.current.get(identity), sent.event())) {
                this.current.remove(identity);
            }
        }
    }

    /** Activates a caller-authenticated session after the old writer generation is idle. */
    public void reconnect(final byte[] newSessionId) {
        if (Thread.currentThread() == this.worker) {
            throw new IllegalStateException("reconnect cannot be called by bridge writer");
        }
        final byte[] validated = validSession(newSessionId);
        synchronized (this.lock) {
            if (Arrays.equals(this.sessionId, validated)) {
                throw new IllegalArgumentException("reconnect requires a fresh session ID");
            }
            while (this.dispatching) {
                try {
                    this.lock.wait();
                } catch (final InterruptedException interrupted) {
                    Thread.currentThread().interrupt();
                    throw new IllegalStateException("interrupted waiting for bridge writer", interrupted);
                }
            }
            this.ensureOpen();
            this.generation++;
            this.sessionId = validated;
            this.nextSequence = 0;
            this.inFlight.clear();
            this.outbox.replaceWith(this.current.values());
            this.signalLocked();
        }
    }

    public boolean isClosed() {
        synchronized (this.lock) {
            return this.closed;
        }
    }

    public boolean hasWriterThread() {
        return this.worker.isAlive();
    }

    @Override
    public void close() {
        synchronized (this.lock) {
            this.closed = true;
            this.signal.release();
        }
        this.closeWriterOnce();
        if (Thread.currentThread() == this.worker) {
            return;
        }
        boolean interrupted = false;
        for (;;) {
            try {
                this.worker.join();
                break;
            } catch (final InterruptedException ignored) {
                interrupted = true;
            }
        }
        if (interrupted) {
            Thread.currentThread().interrupt();
        }
    }

    private void closeWriterOnce() {
        synchronized (this.lock) {
            if (this.writerClosed) {
                return;
            }
            this.writerClosed = true;
        }
        try {
            this.writer.close();
        } catch (final Throwable closeFailure) {
            synchronized (this.lock) {
                if (this.failure == null) {
                    this.failure = closeFailure;
                }
            }
        }
    }

    private PublishResult offerCurrent(final BridgeEvent event) {
        if (event instanceof BridgeEvent.Control control) {
            this.current.put(identity(control), control);
            return this.outbox.offer(control);
        }
        if (event instanceof BridgeEvent.ReplaceState state) {
            final Object identity = identity(state);
            if (!this.current.containsKey(identity) && this.replacementCount() >= Math.min(this.maxReplacementKeys, this.policyMaxReplacementKeys)) throw new IllegalStateException("replacement-state key bound exceeded");
            this.current.put(identity, state);
            return this.outbox.offer(state);
        }
        if (event instanceof BridgeEvent.ResyncWorld resync) {
            this.removeDirtyFor(resync.world(), resync.epoch());
            this.current.put(identity(resync), resync);
            return this.outbox.offer(resync);
        }

        final BridgeEvent.DirtyChunk dirty = (BridgeEvent.DirtyChunk) event;
        final WorldIdentityKey worldIdentity = new WorldIdentityKey(dirty.world(), dirty.epoch());
        if (this.current.get(worldIdentity) instanceof BridgeEvent.ResyncWorld) {
            return PublishResult.COALESCED;
        }
        final Object identity = identity(dirty);
        final BridgeEvent previous = this.current.get(identity);
        if (previous instanceof BridgeEvent.DirtyChunk old && dirty.revision() <= old.revision()) {
            return PublishResult.COALESCED;
        }
        if (previous == null && this.dirtyCount() >= Math.min(this.maxDirtyKeys, this.policyMaxDirtyKeys)) {
            this.removeDirtyFor(dirty.world(), dirty.epoch());
            final BridgeEvent.ResyncWorld marker = new BridgeEvent.ResyncWorld(dirty.world(), dirty.epoch());
            this.current.put(identity(marker), marker);
            this.outbox.markResync(dirty.world(), dirty.epoch());
            return PublishResult.RESYNC_MARKED;
        }
        this.current.put(identity, dirty);
        return this.outbox.offer(dirty);
    }

    private int replacementCount() {
        int count = 0;
        for (final BridgeEvent event : this.current.values()) {
            if (event instanceof BridgeEvent.ReplaceState) {
                count++;
            }
        }
        return count;
    }

    private int dirtyCount() {
        int count = 0;
        for (final BridgeEvent event : this.current.values()) {
            if (event instanceof BridgeEvent.DirtyChunk) {
                count++;
            }
        }
        return count;
    }

    private void removeDirtyFor(final BridgeEvent.WorldKey world, final long epoch) {
        this.current.entrySet().removeIf(entry -> entry.getKey() instanceof DirtyIdentity identity
            && identity.world().equals(world) && identity.epoch() == epoch);
    }

    private void ensureOpen() {
        if (this.closed) {
            throw new IllegalStateException("publisher is closed", this.failure);
        }
    }

    private void runWriter() {
        for (;;) {
            try {
                this.signal.acquire();
            } catch (final InterruptedException interrupted) {
                Thread.currentThread().interrupt();
                return;
            }
            synchronized (this.lock) {
                if (this.closed) {
                    return;
                }
                this.signaled = false;
            }
            this.dispatchBatch();
            synchronized (this.lock) {
                if (this.closed) {
                    return;
                }
                if (!this.outbox.isEmpty()) {
                    this.signalLocked();
                }
            }
        }
    }

    private void dispatchBatch() {
        final List<Sent> batch;
        final List<BridgeEvent> events;
        final long batchGeneration;
        synchronized (this.lock) {
            if (this.closed) {
                return;
            }
            events = this.outbox.drain();
            if (events.isEmpty()) {
                return;
            }
            if (events.size() > Long.MAX_VALUE - this.nextSequence) {
                this.outbox.replaceWith(this.current.values());
                this.failLocked(new IllegalStateException("bridge sequence exhausted"));
                return;
            }
            batchGeneration = this.generation;
            batch = new ArrayList<>(events.size());
            for (final BridgeEvent event : events) {
                final long sequence = ++this.nextSequence;
                final Sent sent = new Sent(sequence, new Published(event instanceof BridgeEvent.ReplaceState state ? state.payload() : event),
                    toEnvelope(event, sequence, this.sessionId));
                this.inFlight.put(sequence, new InFlight(event, batchGeneration));
                batch.add(sent);
            }
            this.dispatching = true;
        }

        for (final Sent sent : batch) {
            try {
                this.writer.write(sent);
            } catch (final Throwable writeFailure) {
                synchronized (this.lock) {
                    this.outbox.replaceWith(this.current.values());
                    this.inFlight.entrySet().removeIf(entry -> entry.getValue().generation() == batchGeneration);
                    this.dispatching = false;
                    this.failLocked(writeFailure);
                    this.lock.notifyAll();
                }
                this.closeWriterOnce();
                return;
            }
            synchronized (this.lock) {
                this.sentCount++;
                if (this.closed) {
                    this.dispatching = false;
                    this.lock.notifyAll();
                    return;
                }
            }
        }
        synchronized (this.lock) {
            this.dispatching = false;
            this.lock.notifyAll();
        }
    }

    private void failLocked(final Throwable cause) {
        if (this.failure != null) return;
        this.failure = cause;
        this.closed = true;
        this.signal.release();
        this.failureListener.accept(cause);
    }

    private void signalLocked() {
        if (!this.signaled) {
            this.signaled = true;
            this.signal.release();
        }
    }

    private static Object identity(final BridgeEvent event) {
        return switch (event) {
            case BridgeEvent.ReplaceState state -> "state:" + state.key();
            case BridgeEvent.Control control -> "control:" + control.correlationId();
            case BridgeEvent.DirtyChunk dirty -> new DirtyIdentity(dirty.world(), dirty.epoch(), dirty.x(), dirty.z());
            case BridgeEvent.ResyncWorld resync -> new WorldIdentityKey(resync.world(), resync.epoch());
        };
    }
    private static Envelope toEnvelope(final BridgeEvent event, final long sequence, final byte[] sessionId) {
        final byte[] session = sessionId.clone();
        if (event instanceof BridgeEvent.Control control) {
            return control.payload().toBuilder()
                .setProtocolMajor(1)
                .setProtocolMinor(0)
                .setSessionId(com.google.protobuf.ByteString.copyFrom(session))
                .setSequence(sequence)
                .setCorrelationId(control.correlationId())
                .build();
        }
        final Envelope.Builder builder = Envelope.newBuilder()
            .setProtocolMajor(1)
            .setProtocolMinor(0)
            .setSessionId(com.google.protobuf.ByteString.copyFrom(session))
            .setSequence(sequence);
        if (event instanceof BridgeEvent.ReplaceState state) {
            return state.payload().toBuilder()
                .setProtocolMajor(1)
                .setProtocolMinor(0)
                .setSessionId(com.google.protobuf.ByteString.copyFrom(session))
                .setSequence(sequence)
                .build();
        }
        final WorldIdentity world = WorldIdentity.newBuilder()
            .setNamespace(event instanceof BridgeEvent.DirtyChunk dirty ? dirty.world().namespace() : ((BridgeEvent.ResyncWorld) event).world().namespace())
            .setValue(event instanceof BridgeEvent.DirtyChunk dirty ? dirty.world().value() : ((BridgeEvent.ResyncWorld) event).world().value())
            .setEpoch(event instanceof BridgeEvent.DirtyChunk dirty ? dirty.epoch() : ((BridgeEvent.ResyncWorld) event).epoch())
            .build();
        if (event instanceof BridgeEvent.DirtyChunk dirty) {
            return builder.setChunkDirty(ChunkDirty.newBuilder()
                .setWorld(world)
                .setCoordinate(ChunkCoordinate.newBuilder().setX(dirty.x()).setZ(dirty.z()))
                .setRevision(dirty.revision())).build();
        }
        return builder.setWorldResyncRequired(WorldResyncRequired.newBuilder()
            .setWorld(world).setReason(ResyncReason.RESYNC_REASON_QUEUE_FULL)).build();
    }

    private static byte[] validSession(final byte[] sessionId) {
        Objects.requireNonNull(sessionId, "sessionId");
        if (sessionId.length != 16) {
            throw new IllegalArgumentException("session ID must contain exactly 16 bytes");
        }
        return sessionId.clone();
    }

    private record InFlight(BridgeEvent event, long generation) {}
    private record DirtyIdentity(BridgeEvent.WorldKey world, long epoch, int x, int z) {}
    private record WorldIdentityKey(BridgeEvent.WorldKey world, long epoch) {}
}
