package xyz.jpenilla.squaremap.common.bridge.snapshot;

import com.google.inject.Inject;
import com.google.inject.Singleton;
import java.util.Objects;
import java.util.Optional;
import java.util.concurrent.CancellationException;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentMap;
import java.util.function.Consumer;
import net.minecraft.server.level.ServerLevel;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.bridge.v1.ChunkMissing;
import xyz.jpenilla.squaremap.bridge.v1.ChunkMissingReason;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshot;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshotRequest;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.RegistryReplace;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;
import xyz.jpenilla.squaremap.common.WorldManager;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher;
import xyz.jpenilla.squaremap.common.bridge.state.WorldEpochRegistry;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;
import xyz.jpenilla.squaremap.common.util.chunksnapshot.ChunkSnapshotProviderFactory;

/** Handles authenticated sidecar snapshot requests using loader-safe copies and bounded bridge work. */
@Singleton
public final class SnapshotRequestHandler implements AutoCloseable {
    private final WorldManager worlds;
    private final ChunkSnapshotProviderFactory providers;
    private final WorldEpochRegistry epochs;
    private final SnapshotRequestService requests;
    private final ExecutorService worker;
    private final RegistryGate registryGate = new RegistryGate();

    @Inject
    public SnapshotRequestHandler(final WorldManager worlds, final ChunkSnapshotProviderFactory providers,
                                  final WorldEpochRegistry epochs) {
        this(worlds, providers, epochs, new SnapshotRequestService(), Executors.newSingleThreadExecutor(runnable -> {
            final Thread thread = new Thread(runnable, "squaremap-bridge-snapshot");
            thread.setDaemon(true);
            return thread;
        }));
    }

    SnapshotRequestHandler(final WorldManager worlds, final ChunkSnapshotProviderFactory providers,
                           final WorldEpochRegistry epochs, final SnapshotRequestService requests, final ExecutorService worker) {
        this.worlds = Objects.requireNonNull(worlds, "worlds");
        this.providers = Objects.requireNonNull(providers, "providers");
        this.epochs = Objects.requireNonNull(epochs, "epochs");
        this.requests = Objects.requireNonNull(requests, "requests");
        this.worker = Objects.requireNonNull(worker, "worker");
    }

    /** Routes one request and publishes either a snapshot or typed ChunkMissing response. */
    public void handle(final Envelope envelope, final Consumer<BridgeEvent> publish) {
        Objects.requireNonNull(envelope, "envelope");
        Objects.requireNonNull(publish, "publish");
        if (!envelope.hasChunkSnapshotRequest()) return;
        final ChunkSnapshotRequest request = envelope.getChunkSnapshotRequest();
        if (!request.hasWorld() || !request.hasCoordinate()) {
            publish(publish, envelope, missing(request, request.hasWorld() ? request.getWorld() : WorldIdentity.getDefaultInstance(),
                request.hasCoordinate() ? request.getCoordinate() : xyz.jpenilla.squaremap.bridge.v1.ChunkCoordinate.getDefaultInstance(),
                request.getRevision(), ChunkMissingReason.CHUNK_MISSING_REASON_INVALID));
            return;
        }
        final WorldIdentifier identifier = WorldIdentifier.create(request.getWorld().getNamespace(), request.getWorld().getValue());
        final Optional<MapWorldInternal> worldOptional = this.worlds.getWorldIfEnabled(identifier);
        if (worldOptional.isEmpty()) {
            publish(publish, envelope, this.requests.requestMissingChunk(request, ChunkMissingReason.CHUNK_MISSING_REASON_UNLOADED));
            return;
        }
        final MapWorldInternal world = worldOptional.get();
        final ServerLevel level = world.serverLevel();
        final long epoch = this.epochs.epoch(identifier, level);
        if (request.getWorld().getEpoch() != 0L && request.getWorld().getEpoch() != epoch) {
            publish(publish, envelope, this.requests.requestMissingChunk(request, ChunkMissingReason.CHUNK_MISSING_REASON_UNAVAILABLE));
            return;
        }
        final WorldIdentity identity = WorldIdentity.newBuilder().setNamespace(identifier.namespace()).setValue(identifier.value()).setEpoch(epoch).build();
        final CompletionStage<ChunkSnapshot> result;
        try {
            result = this.requests.requestWork(request, ignored -> {
                final CompletionStage<xyz.jpenilla.squaremap.common.util.chunksnapshot.ChunkSnapshot> upstream =
                    this.providers.createChunkSnapshotProvider(level).asyncSnapshot(request.getCoordinate().getX(), request.getCoordinate().getZ());
                final CompletionStage<ChunkSnapshot> encoded = upstream.thenComposeAsync(snapshot -> {
                    if (snapshot == null) return CompletableFuture.failedFuture(new SnapshotRequestService.ChunkMissingException(ChunkMissingReason.CHUNK_MISSING_REASON_UNLOADED));
                    final RegistryDescriptorExporter descriptors = RegistryDescriptorExporter.create(world, request.getRevision(), epoch);
                    final CompletionStage<Void> acknowledged = this.registryGate.ensure(descriptors.snapshot(), registry -> publish(publish, envelope, registry));
                    return acknowledged.thenApplyAsync(ignoredAck -> ChunkSnapshotEncoder.encodePortable(snapshot, descriptors, identity, request.getRevision()), this.worker);
                }, this.worker).toCompletableFuture();
                return new SnapshotRequestService.Work(encoded, upstream);
            });
        } catch (final RuntimeException failure) {
            return;
        }
        result.whenComplete((snapshot, failure) -> {
            if (failure == null) {
                publish(publish, envelope, snapshot);
                return;
            }
            final Throwable cause = unwrap(failure);
            if (cause instanceof SnapshotRequestService.DuplicateRequestException
                || cause instanceof SnapshotRequestService.SaturatedException
                || cause instanceof SnapshotRequestService.StaleEpochException
                || cause instanceof SnapshotRequestService.ClosedException
                || cause instanceof CancellationException) return;
            final ChunkMissingReason reason = cause instanceof SnapshotRequestService.ChunkMissingException missing
                ? missing.reason() : ChunkMissingReason.CHUNK_MISSING_REASON_UNAVAILABLE;
            publish(publish, envelope, this.requests.requestMissingChunk(request, reason));
        });
    }

    private static Throwable unwrap(final Throwable failure) {
        return failure instanceof java.util.concurrent.CompletionException completion && completion.getCause() != null ? completion.getCause() : failure;
    }

    public void acknowledge(final BridgePublisher.Sent sent) {
        final Object payload = sent.event().payload();
        if (payload instanceof Envelope envelope && envelope.hasRegistryReplace()) this.registryGate.acknowledge(envelope.getRegistryReplace());
    }

    public int invalidateWorld(final WorldIdentifier world, final long epoch) {
        final int cancelled = epoch == Long.MAX_VALUE ? this.requests.abortWorld(world) : this.requests.staleEpoch(world, epoch);
        if (epoch == Long.MAX_VALUE) this.registryGate.reset();
        return cancelled;
    }
    public void abortAll() { this.requests.abortAll(); this.registryGate.reset(); }

    private static ChunkMissing missing(final ChunkSnapshotRequest request, final WorldIdentity world,
                                        final xyz.jpenilla.squaremap.bridge.v1.ChunkCoordinate coordinate, final long revision,
                                        final ChunkMissingReason reason) {
        return ChunkMissing.newBuilder().setWorld(world).setCoordinate(coordinate).setRevision(revision).setReason(reason).build();
    }

    private static void publish(final Consumer<BridgeEvent> publisher, final Envelope request, final RegistryReplace registry) {
        publisher.accept(new BridgeEvent.ReplaceState("registry:" + registry.getWorld().getNamespace() + ":" + registry.getWorld().getValue(),
            Envelope.newBuilder().setRegistryReplace(registry).setCorrelationId(request.getCorrelationId()).build()));
    }
    private static void publish(final Consumer<BridgeEvent> publisher, final Envelope request, final ChunkSnapshot snapshot) {
        publisher.accept(new BridgeEvent.ReplaceState("snapshot:" + request.getChunkSnapshotRequest().getRequestId(),
            Envelope.newBuilder().setChunkSnapshot(snapshot).setCorrelationId(request.getCorrelationId()).build()));
    }
    private static void publish(final Consumer<BridgeEvent> publisher, final Envelope request, final ChunkMissing missing) {
        publisher.accept(new BridgeEvent.ReplaceState("snapshot:" + request.getChunkSnapshotRequest().getRequestId(),
            Envelope.newBuilder().setChunkMissing(missing).setCorrelationId(request.getCorrelationId()).build()));
    }

    @Override public void close() {
        this.requests.close();
        this.registryGate.reset();
        this.worker.shutdownNow();
    }

    private static final class RegistryGate {
        private final Object lock = new Object();
        private final ConcurrentMap<String, State> states = new ConcurrentHashMap<>();

        CompletionStage<Void> ensure(final RegistryReplace registry, final Consumer<RegistryReplace> publisher) {
            final String key = key(registry.getWorld());
            final State state = this.states.computeIfAbsent(key, ignored -> new State());
            final CompletableFuture<Void> wait;
            boolean publishNow = false;
            synchronized (this.lock) {
                if (state.acknowledged != null && state.acknowledged.equals(registry)) return CompletableFuture.completedFuture(null);
                if (state.pending != null) {
                    wait = state.completion;
                } else {
                    state.pending = registry;
                    state.completion = new CompletableFuture<>();
                    wait = state.completion;
                    publishNow = true;
                }
            }
            if (publishNow) {
                try { publisher.accept(registry); }
                catch (final Throwable failure) { acknowledgeFailure(key, state, failure); }
            }
            if (publishNow) return wait;
            return wait.thenCompose(ignored -> ensure(registry, publisher));
        }

        void acknowledge(final RegistryReplace registry) {
            final State state = this.states.get(key(registry.getWorld()));
            if (state == null) return;
            CompletableFuture<Void> completion = null;
            synchronized (this.lock) {
                if (state.pending != null && state.pending.equals(registry)) {
                    state.acknowledged = registry;
                    state.pending = null;
                    completion = state.completion;
                    state.completion = null;
                }
            }
            if (completion != null) completion.complete(null);
        }

        void reset() {
            for (State state : this.states.values()) {
                CompletableFuture<Void> completion;
                synchronized (this.lock) {
                    completion = state.completion;
                    state.pending = null;
                    state.completion = null;
                    state.acknowledged = null;
                }
                if (completion != null) completion.completeExceptionally(new CancellationException("registry gate reset"));
            }
            this.states.clear();
        }

        private void acknowledgeFailure(final String key, final State state, final Throwable failure) {
            synchronized (this.lock) {
                state.pending = null;
                state.completion.completeExceptionally(failure);
                state.completion = null;
                this.states.remove(key, state);
            }
        }
        private static String key(final WorldIdentity world) { return world.getNamespace() + "\u0000" + world.getValue() + "\u0000" + world.getEpoch(); }
        private static final class State {
            private RegistryReplace acknowledged;
            private RegistryReplace pending;
            private CompletableFuture<Void> completion;
        }
    }
}
