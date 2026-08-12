package xyz.jpenilla.squaremap.common.bridge.snapshot;

import java.util.Objects;
import java.util.concurrent.CancellationException;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentMap;
import java.util.concurrent.Semaphore;
import java.util.function.Function;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.bridge.v1.ChunkMissing;
import xyz.jpenilla.squaremap.bridge.v1.ChunkMissingReason;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshot;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshotRequest;

/** Strictly bounded, cancellable admission for snapshot work. */
public final class SnapshotRequestService implements AutoCloseable {
    public static final int DEFAULT_MAX_IN_FLIGHT = 96;
    private final Object lock = new Object();
    private final Semaphore permits;
    private final ConcurrentMap<Long, Active> active = new ConcurrentHashMap<>();
    private final ConcurrentMap<String, Long> currentEpochs = new ConcurrentHashMap<>();
    private boolean closed;

    public SnapshotRequestService() { this(DEFAULT_MAX_IN_FLIGHT); }
    public SnapshotRequestService(final int maxInFlight) {
        if (maxInFlight < 1) throw new IllegalArgumentException("maxInFlight must be positive");
        this.permits = new Semaphore(maxInFlight);
    }

    public record Work(CompletionStage<ChunkSnapshot> stage, CompletionStage<?> upstream) {
        public Work {
            Objects.requireNonNull(stage, "stage");
            Objects.requireNonNull(upstream, "upstream");
        }
        public static Work of(final CompletionStage<ChunkSnapshot> stage) { return new Work(stage, stage); }
    }

    public CompletableFuture<ChunkSnapshot> request(final ChunkSnapshotRequest request,
        final Function<ChunkSnapshotRequest, ? extends CompletionStage<ChunkSnapshot>> producer) {
        Objects.requireNonNull(producer, "producer");
        return requestWork(request, ignored -> Work.of(producer.apply(request)));
    }

    public CompletableFuture<ChunkSnapshot> requestWork(final ChunkSnapshotRequest request,
        final Function<ChunkSnapshotRequest, Work> producer) {
        Objects.requireNonNull(request, "request");
        Objects.requireNonNull(producer, "producer");
        final CompletableFuture<ChunkSnapshot> result = new CompletableFuture<>();
        final Active entry;
        synchronized (this.lock) {
            if (this.closed) return failed(result, new ClosedException());
            final String world = worldKey(request);
            final long current = this.currentEpochs.getOrDefault(world, 0L);
            if (request.getWorld().getEpoch() < current) return failed(result, new StaleEpochException(request.getWorld().getEpoch(), current));
            if (this.active.containsKey(request.getRequestId())) return failed(result, new DuplicateRequestException(request.getRequestId()));
            if (!this.permits.tryAcquire()) return failed(result, new SaturatedException());
            entry = new Active(world, request.getWorld().getEpoch(), result);
            this.active.put(request.getRequestId(), entry);
        }
        try {
            final Work work = producer.apply(request);
            if (work == null) throw new ProducerFailure("producer returned null");
            entry.stage = work.stage();
            entry.upstream = work.upstream();
            work.stage().whenComplete((snapshot, failure) -> {
                if (failure == null) result.complete(snapshot); else result.completeExceptionally(failure);
                entry.stageDone = true;
                maybeRelease(request.getRequestId(), entry);
            });
            work.upstream().whenComplete((ignored, failure) -> {
                entry.upstreamDone = true;
                maybeRelease(request.getRequestId(), entry);
            });
            if (entry.cancelRequested) {
                cancelStage(work.stage());
                cancelStage(work.upstream());
            }
        } catch (final Throwable failure) {
            result.completeExceptionally(failure instanceof java.util.concurrent.RejectedExecutionException
                ? new ProducerFailure("executor rejected snapshot", failure) : failure);
            entry.stageDone = true;
            entry.upstreamDone = true;
            maybeRelease(request.getRequestId(), entry);
        }
        return result;
    }

    private static <T> CompletableFuture<T> failed(final CompletableFuture<T> future, final RuntimeException error) {
        future.completeExceptionally(error);
        return future;
    }

    private static String worldKey(final ChunkSnapshotRequest request) {
        return worldKey(request.getWorld().getNamespace(), request.getWorld().getValue());
    }
    private static String worldKey(final WorldIdentifier world) { return worldKey(world.namespace(), world.value()); }
    private static String worldKey(final String namespace, final String value) { return namespace + '\u0000' + value; }
    private static void cancelStage(final CompletionStage<?> stage) { if (stage instanceof CompletableFuture<?> future) future.cancel(true); }

    public ChunkMissing requestMissingChunk(final ChunkSnapshotRequest request, final ChunkMissingReason reason) {
        Objects.requireNonNull(request); return ChunkMissing.newBuilder().setWorld(request.getWorld()).setCoordinate(request.getCoordinate())
            .setRevision(request.getRevision()).setReason(Objects.requireNonNull(reason)).build();
    }
    public int inFlightCount() { return this.active.size(); }

    public boolean cancel(final long requestId) {
        final Active entry;
        synchronized (this.lock) {
            entry = this.active.get(requestId);
            if (entry == null) return false;
            entry.cancelRequested = true;
        }
        final CompletionStage<?> stage = entry.stage;
        final CompletionStage<?> upstream = entry.upstream;
        if (stage != null) cancelStage(stage);
        if (upstream != null && upstream != stage) cancelStage(upstream);
        entry.result.completeExceptionally(new CancellationException("snapshot request cancelled"));
        maybeRelease(requestId, entry);
        return true;
    }

    public int staleEpoch(final WorldIdentifier world, final long nextEpoch) {
        final String key = worldKey(world);
        final long previous = this.currentEpochs.getOrDefault(key, 0L);
        if (nextEpoch <= previous) return 0;
        this.currentEpochs.put(key, nextEpoch);
        int cancelled = 0;
        for (var item : this.active.entrySet()) {
            if (item.getValue().world.equals(key) && item.getValue().epoch < nextEpoch && cancel(item.getKey())) cancelled++;
        }
        return cancelled;
    }

    /** Compatibility invalidation for callers that manage one global epoch. */
    public int staleEpoch(final long nextEpoch) {
        int cancelled = 0;
        for (var item : this.active.entrySet()) if (item.getValue().epoch < nextEpoch && cancel(item.getKey())) cancelled++;
        return cancelled;
    }

    public void awaitIdle() { while (!this.active.isEmpty()) Thread.yield(); }
    public int abortWorld(final WorldIdentifier world) {
        final String key = worldKey(world);
        int cancelled = 0;
        for (var item : this.active.entrySet()) {
            if (item.getValue().world.equals(key) && cancel(item.getKey())) cancelled++;
        }
        return cancelled;
    }

    private void maybeRelease(final long id, final Active entry) {
        if (entry.stageDone && entry.upstreamDone && entry.released.compareAndSet(false, true)) {
            this.active.remove(id, entry);
            this.permits.release();
        }
    }

    public void abortAll() {
        for (Long id : this.active.keySet()) cancel(id);
    }

    @Override public void close() {
        synchronized (this.lock) { this.closed = true; }
        abortAll();
    }

    public static final class ChunkMissingException extends RuntimeException {
        private final ChunkMissingReason reason;
        public ChunkMissingException(final ChunkMissingReason reason) { super("chunk is unavailable: " + reason); this.reason = Objects.requireNonNull(reason, "reason"); }
        public ChunkMissingReason reason() { return this.reason; }
    }
    public static final class SaturatedException extends RuntimeException { public SaturatedException() { super("snapshot admission is saturated"); } }
    public static final class ClosedException extends RuntimeException { public ClosedException() { super("snapshot admission is closed"); } }
    public static final class StaleEpochException extends RuntimeException { public StaleEpochException(long oldEpoch, long current) { super("stale world epoch " + oldEpoch + " < " + current); } }
    public static final class DuplicateRequestException extends RuntimeException { public DuplicateRequestException(long id) { super("duplicate snapshot request " + id); } }
    public static final class ProducerFailure extends RuntimeException { public ProducerFailure(String message) { super(message); } public ProducerFailure(String message, Throwable cause) { super(message, cause); } }

    private static final class Active {
        private final String world;
        private final long epoch;
        private final CompletableFuture<ChunkSnapshot> result;
        private final java.util.concurrent.atomic.AtomicBoolean released = new java.util.concurrent.atomic.AtomicBoolean();
        private volatile CompletionStage<ChunkSnapshot> stage;
        private volatile CompletionStage<?> upstream;
        private volatile boolean cancelRequested;
        private volatile boolean stageDone;
        private volatile boolean upstreamDone;
        private Active(final String world, final long epoch, final CompletableFuture<ChunkSnapshot> result) {
            this.world = world; this.epoch = epoch; this.result = result;
        }
    }
}
