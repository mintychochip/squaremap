package xyz.jpenilla.squaremap.common.bridge.outbox;

import com.google.protobuf.ByteString;
import java.time.Duration;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.Semaphore;
import java.util.concurrent.TimeUnit;
import java.util.function.Consumer;
import xyz.jpenilla.squaremap.bridge.v1.Ack;
import xyz.jpenilla.squaremap.bridge.v1.ChunkCoordinate;
import xyz.jpenilla.squaremap.bridge.v1.ChunkDirty;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.ResyncReason;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;
import xyz.jpenilla.squaremap.bridge.v1.WorldResyncRequired;

/** Owns the only bridge writer worker and keeps durable values until acknowledgement. */
public final class BridgePublisher implements AutoCloseable {
    public enum PublishResult {
        ACCEPTED,
        COALESCED,
        RESYNC_MARKED
    }

    public record Published(Object payload) {
        public Published {
            Objects.requireNonNull(payload, "payload");
        }
    }

    public record Sent(long sequence, Published event, Envelope envelope) {}

    private final Object lock = new Object();
    private final CoalescingOutbox outbox;
    private final Consumer<Sent> writer;
    private final Semaphore signal = new Semaphore(0);
    private final Thread worker;
    private final Map<Object, BridgeEvent> latest = new HashMap<>();
    private final Map<Long, InFlight> inFlight = new HashMap<>();
    private final Set<DirtyIdentity> inFlightDirty = new HashSet<>();
    private boolean signaled;
    private boolean closed;
    private long sentCount;
    private long nextSequence;
    private byte[] sessionId = sessionBytes();

    public BridgePublisher(final Consumer<Sent> writer) {
        this(new CoalescingOutbox(), writer);
    }

    public BridgePublisher(final CoalescingOutbox outbox, final Consumer<Sent> writer) {
        this.outbox = Objects.requireNonNull(outbox, "outbox");
        this.writer = Objects.requireNonNull(writer, "writer");
        this.worker = new Thread(this::runWriter, "squaremap-bridge-writer");
        this.worker.setDaemon(true);
        this.worker.start();
    }

    public PublishResult publish(final BridgeEvent event) {
        Objects.requireNonNull(event, "event");
        synchronized (this.lock) {
            if (this.closed) {
                throw new IllegalStateException("publisher is closed");
            }
            final PublishResult result;
            if (event instanceof BridgeEvent.DirtyChunk dirty
                && !this.inFlightDirty.contains(DirtyIdentity.from(dirty))
                && !this.outbox.containsDirtyKey(dirty)
                && this.outbox.dirtyKeyCount() + this.inFlightDirty.size() >= CoalescingOutbox.MAX_DIRTY_KEYS) {
                result = this.outbox.markResync(dirty.world(), dirty.epoch());
            } else {
                result = this.outbox.offer(event);
            }
            this.rememberLatest(event);
            this.signalLocked();
            return result;
        }
    }

    private void rememberLatest(final BridgeEvent event) {
        if (event instanceof BridgeEvent.ResyncWorld
            || event instanceof BridgeEvent.DirtyChunk dirty && this.outbox.containsResync(dirty.world(), dirty.epoch())) {
            final BridgeEvent.WorldKey world;
            final long epoch;
            if (event instanceof BridgeEvent.ResyncWorld resync) {
                world = resync.world();
                epoch = resync.epoch();
            } else {
                final BridgeEvent.DirtyChunk dirty = (BridgeEvent.DirtyChunk) event;
                world = dirty.world();
                epoch = dirty.epoch();
            }
            this.latest.entrySet().removeIf(entry -> entry.getKey() instanceof DirtyIdentity key
                && key.world().equals(world) && key.epoch() == epoch);
            this.latest.put(new WorldIdentityKey(world, epoch), new BridgeEvent.ResyncWorld(world, epoch));
            return;
        }
        this.latest.put(identity(event), event);
    }

    public byte[] sessionId() {
        synchronized (this.lock) {
            return this.sessionId.clone();
        }
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

    public void acknowledge(final long sequence) {
        synchronized (this.lock) {
            final InFlight sent = this.inFlight.remove(sequence);
            if (sent == null) {
                return;
            }
            if (sent.event() instanceof BridgeEvent.DirtyChunk dirty) {
                this.inFlightDirty.remove(DirtyIdentity.from(dirty));
            }
        }
    }
    public void acknowledge(final Ack ack) {
        Objects.requireNonNull(ack, "ack");
        this.acknowledge(ack.getAcknowledgedSequence());
    }

    public void reconnect() {
        synchronized (this.lock) {
            if (this.closed) {
                return;
            }
            this.sessionId = sessionBytes();
            this.nextSequence = 0;
            this.inFlight.clear();
            this.inFlightDirty.clear();
            this.outbox.replaceWith(this.latest.values());
            this.signalLocked();
        }
    }

    public void drainNow() {
        final List<Sent> sent = new ArrayList<>();
        synchronized (this.lock) {
            if (this.closed) {
                return;
            }
            for (final BridgeEvent event : this.outbox.drain()) {
                if (this.nextSequence == Long.MAX_VALUE) {
                    this.closed = true;
                    throw new IllegalStateException("bridge sequence exhausted");
                }
                final long sequence = ++this.nextSequence;
                final Envelope envelope = toEnvelope(event, sequence, this.sessionId);
                this.inFlight.put(sequence, new InFlight(event));
                if (event instanceof BridgeEvent.DirtyChunk dirty) {
                    this.inFlightDirty.add(DirtyIdentity.from(dirty));
                }
                this.sentCount++;
                sent.add(new Sent(sequence, new Published(event instanceof BridgeEvent.ReplaceState state ? state.payload() : event), envelope));
            }
        }
        for (final Sent value : sent) {
            this.writer.accept(value);
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
            if (this.closed) {
                this.signal.release();
            } else {
                this.closed = true;
                this.signal.release();
            }
        }
        if (Thread.currentThread() != this.worker) {
            try {
                this.worker.join(TimeUnit.SECONDS.toMillis(2));
            } catch (final InterruptedException interrupted) {
                Thread.currentThread().interrupt();
            }
        }
    }

    private void runWriter() {
        while (true) {
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
            this.drainNow();
            synchronized (this.lock) {
                if (!this.outbox.isEmpty() && !this.signaled && !this.closed) {
                    this.signaled = true;
                    continue;
                }
            }
        }
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
            case BridgeEvent.DirtyChunk dirty -> new DirtyIdentity(dirty.world(), dirty.epoch(), dirty.x(), dirty.z());
            case BridgeEvent.ResyncWorld resync -> new WorldIdentityKey(resync.world(), resync.epoch());
        };
    }

    private static Envelope toEnvelope(final BridgeEvent event, final long sequence, final byte[] sessionId) {
        final Envelope.Builder builder = Envelope.newBuilder()
            .setProtocolMajor(1)
            .setProtocolMinor(0)
            .setSessionId(ByteString.copyFrom(sessionId))
            .setSequence(sequence);
        if (event instanceof BridgeEvent.ReplaceState state) {
            return state.payload().toBuilder()
                .setSessionId(ByteString.copyFrom(sessionId))
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
                .setRevision(dirty.revision()))
                .build();
        }
        return builder.setWorldResyncRequired(WorldResyncRequired.newBuilder()
            .setWorld(world)
            .setReason(ResyncReason.RESYNC_REASON_QUEUE_FULL))
            .build();
    }

    private static byte[] sessionBytes() {
        final UUID uuid = UUID.randomUUID();
        final byte[] bytes = new byte[16];
        for (int index = 0; index < Long.BYTES; index++) {
            bytes[index] = (byte) (uuid.getMostSignificantBits() >>> (56 - index * 8));
            bytes[8 + index] = (byte) (uuid.getLeastSignificantBits() >>> (56 - index * 8));
        }
        return bytes;
    }

    private record InFlight(BridgeEvent event) {}
    private record DirtyIdentity(BridgeEvent.WorldKey world, long epoch, int x, int z) {
        static DirtyIdentity from(final BridgeEvent.DirtyChunk event) {
            return new DirtyIdentity(event.world(), event.epoch(), event.x(), event.z());
        }
    }
    private record WorldIdentityKey(BridgeEvent.WorldKey world, long epoch) {}
}
