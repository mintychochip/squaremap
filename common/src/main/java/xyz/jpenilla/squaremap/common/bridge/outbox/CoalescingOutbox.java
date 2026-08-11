package xyz.jpenilla.squaremap.common.bridge.outbox;

import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.TreeMap;

/** One-lock, deterministic, bounded event coalescer. */
public final class CoalescingOutbox {
    public static final int MAX_DIRTY_KEYS = 65_536;

    private final int maxDirtyKeys;
    private final int maxReplacementKeys;
    private final Object lock = new Object();
    private final Map<String, BridgeEvent.ReplaceState> states = new TreeMap<>();
    private final Map<DirtyKey, BridgeEvent.DirtyChunk> dirty = new TreeMap<>();
    private final Map<WorldEpoch, BridgeEvent.ResyncWorld> resyncs = new TreeMap<>();

    public CoalescingOutbox() {
        this(MAX_DIRTY_KEYS, BridgePublisher.DEFAULT_MAX_REPLACEMENT_KEYS);
    }

    CoalescingOutbox(final int maxDirtyKeys, final int maxReplacementKeys) {
        if (maxDirtyKeys < 1 || maxReplacementKeys < 1) {
            throw new IllegalArgumentException("outbox bounds must be positive");
        }
        this.maxDirtyKeys = maxDirtyKeys;
        this.maxReplacementKeys = maxReplacementKeys;
    }

    int maxDirtyKeys() {
        return this.maxDirtyKeys;
    }
    public BridgePublisher.PublishResult offer(final BridgeEvent event) {
        Objects.requireNonNull(event, "event");
        synchronized (this.lock) {
            return this.offerLocked(event);
        }
    }
    public BridgePublisher.PublishResult markResync(final BridgeEvent.WorldKey world, final long epoch) {
        Objects.requireNonNull(world, "world");
        synchronized (this.lock) {
            return this.markResyncLocked(world, epoch);
        }
    }

    public List<BridgeEvent> drain() {
        synchronized (this.lock) {
            final List<BridgeEvent> result = new ArrayList<>(this.states.size() + this.dirty.size() + this.resyncs.size());
            result.addAll(this.states.values());
            result.addAll(this.dirty.values());
            result.addAll(this.resyncs.values());
            this.states.clear();
            this.dirty.clear();
            this.resyncs.clear();
            return result;
        }
    }

    public int dirtyKeyCount() {
        synchronized (this.lock) {
            return this.dirty.size();
        }
    }

    public boolean containsDirtyKey(final BridgeEvent.DirtyChunk event) {
        synchronized (this.lock) {
            return this.dirty.containsKey(DirtyKey.from(event));
        }
    }
    public boolean containsResync(final BridgeEvent.WorldKey world, final long epoch) {
        Objects.requireNonNull(world, "world");
        synchronized (this.lock) {
            return this.resyncs.containsKey(new WorldEpoch(world, epoch));
        }
    }

    public boolean isEmpty() {
        synchronized (this.lock) {
            return this.states.isEmpty() && this.dirty.isEmpty() && this.resyncs.isEmpty();
        }
    }

    void replaceWith(final Collection<BridgeEvent> events) {
        synchronized (this.lock) {
            this.states.clear();
            this.dirty.clear();
            this.resyncs.clear();
            for (final BridgeEvent event : events) {
                this.offerLocked(event);
            }
        }
    }
    void requeue(final Collection<BridgeEvent> events) {
        synchronized (this.lock) {
            for (final BridgeEvent event : events) {
                this.offerLocked(event);
            }
        }
    }

    private BridgePublisher.PublishResult offerLocked(final BridgeEvent event) {
        if (event instanceof BridgeEvent.ReplaceState state) {
            final BridgeEvent.ReplaceState previous = this.states.get(state.key());
            if (previous == null && this.states.size() >= this.maxReplacementKeys) {
                throw new IllegalStateException("replacement-state key bound exceeded");
            }
            this.states.put(state.key(), state);
            return previous == null ? BridgePublisher.PublishResult.ACCEPTED : BridgePublisher.PublishResult.COALESCED;
        }
        if (event instanceof BridgeEvent.ResyncWorld resync) {
            final WorldEpoch worldEpoch = new WorldEpoch(resync.world(), resync.epoch());
            this.dirty.entrySet().removeIf(entry -> entry.getKey().worldEpoch().equals(worldEpoch));
            final BridgeEvent.ResyncWorld previous = this.resyncs.put(worldEpoch, resync);
            return previous == null ? BridgePublisher.PublishResult.ACCEPTED : BridgePublisher.PublishResult.COALESCED;
        }

        final BridgeEvent.DirtyChunk chunk = (BridgeEvent.DirtyChunk) event;
        final WorldEpoch worldEpoch = new WorldEpoch(chunk.world(), chunk.epoch());
        if (this.resyncs.containsKey(worldEpoch)) {
            return BridgePublisher.PublishResult.COALESCED;
        }
        final DirtyKey key = DirtyKey.from(chunk);
        final BridgeEvent.DirtyChunk previous = this.dirty.get(key);
        if (previous != null) {
            if (chunk.revision() > previous.revision()) {
                this.dirty.put(key, chunk);
            }
            return BridgePublisher.PublishResult.COALESCED;
        }
        if (this.dirty.size() >= this.maxDirtyKeys) {
            return this.markResyncLocked(worldEpoch.world(), worldEpoch.epoch());
        }
        this.dirty.put(key, chunk);
        return BridgePublisher.PublishResult.ACCEPTED;
    }
    private BridgePublisher.PublishResult markResyncLocked(final BridgeEvent.WorldKey world, final long epoch) {
        final WorldEpoch worldEpoch = new WorldEpoch(world, epoch);
        this.dirty.entrySet().removeIf(entry -> entry.getKey().worldEpoch().equals(worldEpoch));
        final BridgeEvent.ResyncWorld previous = this.resyncs.put(worldEpoch, new BridgeEvent.ResyncWorld(world, epoch));
        return previous == null ? BridgePublisher.PublishResult.RESYNC_MARKED : BridgePublisher.PublishResult.COALESCED;
    }

    private record WorldEpoch(BridgeEvent.WorldKey world, long epoch) implements Comparable<WorldEpoch> {
        @Override
        public int compareTo(final WorldEpoch other) {
            final int worldResult = this.world.compareTo(other.world);
            return worldResult != 0 ? worldResult : Long.compare(this.epoch, other.epoch);
        }
    }

    private record DirtyKey(WorldEpoch worldEpoch, int x, int z) implements Comparable<DirtyKey> {
        static DirtyKey from(final BridgeEvent.DirtyChunk event) {
            return new DirtyKey(new WorldEpoch(event.world(), event.epoch()), event.x(), event.z());
        }

        @Override
        public int compareTo(final DirtyKey other) {
            final int worldResult = this.worldEpoch.compareTo(other.worldEpoch);
            if (worldResult != 0) {
                return worldResult;
            }
            final int xResult = Integer.compare(this.x, other.x);
            return xResult != 0 ? xResult : Integer.compare(this.z, other.z);
        }
    }
}
