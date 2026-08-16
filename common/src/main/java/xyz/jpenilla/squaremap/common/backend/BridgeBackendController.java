package xyz.jpenilla.squaremap.common.backend;

import com.google.inject.Inject;
import com.google.inject.Provider;
import com.google.inject.Singleton;
import com.google.protobuf.ByteString;
import java.security.MessageDigest;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.List;
import java.util.Objects;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentMap;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.bridge.v1.BackendResultCode;
import xyz.jpenilla.squaremap.bridge.v1.BackendSubstitution;
import xyz.jpenilla.squaremap.bridge.v1.BridgePolicyReplace;
import xyz.jpenilla.squaremap.bridge.v1.ControlKind;
import xyz.jpenilla.squaremap.bridge.v1.ControlRequest;
import xyz.jpenilla.squaremap.bridge.v1.ControlResult;
import xyz.jpenilla.squaremap.bridge.v1.DirtyReplayItem;
import xyz.jpenilla.squaremap.bridge.v1.DirtyReplayRequest;
import xyz.jpenilla.squaremap.bridge.v1.DirtyResyncComplete;
import xyz.jpenilla.squaremap.bridge.v1.DirtyResyncStatus;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.ReplayStart;
import xyz.jpenilla.squaremap.bridge.v1.ResumeWatermark;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;
import xyz.jpenilla.squaremap.bridge.v1.WorldResyncRequired;
import xyz.jpenilla.squaremap.common.ServerAccess;
import xyz.jpenilla.squaremap.common.Logging;
import xyz.jpenilla.squaremap.common.SquaremapCommon;
import xyz.jpenilla.squaremap.common.WorldManager;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher;
import xyz.jpenilla.squaremap.common.bridge.process.BackendLifecyclePolicy;
import xyz.jpenilla.squaremap.common.bridge.process.BackendMode;
import xyz.jpenilla.squaremap.common.bridge.process.BridgeBootstrapConfig;
import xyz.jpenilla.squaremap.common.bridge.process.BridgeConnection;
import xyz.jpenilla.squaremap.common.bridge.process.SidecarSupervisor;
import xyz.jpenilla.squaremap.common.bridge.snapshot.SnapshotRequestHandler;
import xyz.jpenilla.squaremap.common.bridge.state.WorldEpochRegistry;
import xyz.jpenilla.squaremap.common.config.ConfigBridgeExporter;
import xyz.jpenilla.squaremap.common.data.ChunkCoordinate;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;

/** Rust control implementation with bounded correlation and timeout handling. */
@Singleton
public final class BridgeBackendController implements AutoCloseable {
    public static final Duration REQUEST_TIMEOUT = Duration.ofSeconds(10);
    private static final long MAX_CORRELATION_ID = Long.MAX_VALUE;
    private static final int MAX_PENDING = 1_024;
    private static final int MAX_REPLAY_ITEMS = 1_024;

    private final SidecarSupervisor supervisor;
    private final ScheduledExecutorService scheduler;
    private final Duration requestTimeout;
    private final BackendMode mode;
    private final ConfigBridgeExporter configExporter;
    private final Provider<SquaremapCommon> common;
    private final Provider<WorldManager> worldManager;
    private final EpochResolver epochs;
    private final SnapshotRequestHandler snapshotHandler;

    private final ConcurrentMap<Long, PendingControl> pending = new ConcurrentHashMap<>();
    private final ConcurrentMap<Long, ScheduledFuture<?>> timeouts = new ConcurrentHashMap<>();
    private final ConcurrentMap<ReplayKey, ReplayState> replayStates = new ConcurrentHashMap<>();
    private final AtomicLong nextCorrelation = new AtomicLong(0L);
    private final AtomicLong nextReplayId = new AtomicLong(0L);
    private final AtomicReference<BridgePolicyReplace> policy = new AtomicReference<>();

    private volatile int readyHttpPort;
    private volatile boolean readyHttpEnabled;
    private volatile CompletableFuture<BackendResult> configPending;
    private volatile ScheduledFuture<?> configTimeout;
    private volatile long configPendingRevision;
    private volatile long configGeneration;
    private volatile BridgeConnection listeningConnection;
    private long connectionGeneration;

    @Inject
    public BridgeBackendController(
        final BridgeBootstrapConfig config,
        final SidecarSupervisor supervisor,
        final ConfigBridgeExporter configExporter,
        final Provider<SquaremapCommon> common,
        final Provider<WorldManager> worldManager,
        final ServerAccess serverAccess,
        final WorldEpochRegistry epochs,
        final SnapshotRequestHandler snapshotHandler
    ) {
        this(
            supervisor,
            defaultScheduler(),
            REQUEST_TIMEOUT,
            config.backendMode(),
            configExporter,
            common,
            worldManager,
            world -> {
                final net.minecraft.server.level.ServerLevel level = serverAccess.level(world);
                return level == null ? 0L : epochs.epoch(world, level);
            },
            snapshotHandler
        );
    }

    private static ScheduledExecutorService defaultScheduler() {
        return Executors.newSingleThreadScheduledExecutor(runnable -> {
            final Thread thread = new Thread(runnable, "squaremap-backend-control");
            thread.setDaemon(true);
            return thread;
        });
    }

    public BridgeBackendController(final SidecarSupervisor supervisor, final ScheduledExecutorService scheduler) {
        this(supervisor, scheduler, REQUEST_TIMEOUT, BackendMode.RUST, null, null, null, world -> 0L, null);
    }

    public BridgeBackendController(final BridgeConnection connection, final ScheduledExecutorService scheduler) {
        this(connection, scheduler, REQUEST_TIMEOUT, world -> 0L);
    }

    public BridgeBackendController(
        final BridgeConnection connection,
        final ScheduledExecutorService scheduler,
        final Duration requestTimeout,
        final EpochResolver epochs
    ) {
        this(null, scheduler, requestTimeout, BackendMode.RUST, null, null, null, epochs, null);
        this.attach(connection);
    }

    BridgeBackendController(
        final BridgeConnection connection,
        final ScheduledExecutorService scheduler,
        final Duration requestTimeout,
        final EpochResolver epochs,
        final Provider<WorldManager> worldManager
    ) {
        this(null, scheduler, requestTimeout, BackendMode.RUST, null, null, worldManager, epochs, null);
        this.attach(connection);
    }

    private BridgeBackendController(
        final SidecarSupervisor supervisor,
        final ScheduledExecutorService scheduler,
        final Duration requestTimeout,
        final BackendMode mode,
        final ConfigBridgeExporter configExporter,
        final Provider<SquaremapCommon> common,
        final Provider<WorldManager> worldManager,
        final EpochResolver epochs,
        final SnapshotRequestHandler snapshotHandler
    ) {
        this.supervisor = supervisor;
        this.snapshotHandler = snapshotHandler;
        this.scheduler = Objects.requireNonNull(scheduler);
        this.requestTimeout = Objects.requireNonNull(requestTimeout);
        this.mode = Objects.requireNonNull(mode);
        this.configExporter = configExporter;
        this.common = common;
        this.worldManager = worldManager;
        if (supervisor != null) supervisor.setReconnectListener(this::attach);
        this.epochs = Objects.requireNonNull(epochs);
    }

    @FunctionalInterface
    interface EpochResolver {
        long epoch(WorldIdentifier world);
    }

    public BridgePublisher.PublishResult publishDirty(
        final MapWorldInternal world,
        final ChunkCoordinate coordinate,
        final long revision
    ) {
        final BridgeConnection connection = this.connection();
        if (connection == null || connection.isClosed()) return BridgePublisher.PublishResult.COALESCED;
        final WorldIdentifier identifier = world.identifier();
        return connection.publish(new BridgeEvent.DirtyChunk(
            new BridgeEvent.WorldKey(identifier.namespace(), identifier.value()),
            this.epochs.epoch(identifier),
            coordinate.x(),
            coordinate.z(),
            revision
        ));
    }

    CompletionStage<BackendResult> execute(final BackendController.BackendRequest request) {
        if (request instanceof BackendController.Reload) {
            if (this.common == null) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.INVALID_REQUEST));
            this.common.get().reload();
            return CompletableFuture.completedFuture(new BackendResult(
                BackendResult.Code.RELOADED,
                List.of(new BackendResult.Substitution("version", new BackendResult.Text(this.common.get().version())))
            ));
        }
        if (request instanceof BackendController.ConfigSync sync) return this.publishConfig(sync.config());
        if (request instanceof BackendController.RestartProgressLogging) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.INVALID_REQUEST));
        final ConnectionAdmission admission;
        synchronized (this) {
            final BridgeConnection connection = this.connection();
            if (connection == null || connection.isClosed()) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
            if (request instanceof BackendController.RadiusRender radius && radius.radius() < 1) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.INVALID_REQUEST));
            if (this.pending.size() >= MAX_PENDING) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
            final long id = this.allocateCorrelationId();
            final CompletableFuture<BackendResult> result = new CompletableFuture<>();
            final long generation = this.connectionGeneration;
            this.pending.put(id, new PendingControl(result, connection, generation));
            admission = new ConnectionAdmission(connection, generation, id, result);
            final ControlRequest.Builder control = ControlRequest.newBuilder().setKind(kind(request));
            if (request.world() != null) control.setWorld(identity(request.world()));
            if (request instanceof BackendController.RadiusRender radius) {
                control.setCenterX(radius.centerX()).setCenterZ(radius.centerZ()).setRadius(radius.radius());
            }
            try {
                connection.publishControl(id, control.build());
            } catch (final RuntimeException failure) {
                this.complete(id, BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
                return result;
            }
        }
        final ScheduledFuture<?> timeout = this.scheduler.schedule(
            () -> this.timeoutControl(admission.connection(), admission.generation(), admission.correlationId()),
            this.requestTimeout.toNanos(),
            TimeUnit.NANOSECONDS
        );
        synchronized (this) {
            final PendingControl current = this.pending.get(admission.correlationId());
            if (current != null && current.generation() == admission.generation() && current.connection() == admission.connection()) {
                this.timeouts.put(admission.correlationId(), timeout);
            } else {
                timeout.cancel(false);
            }
        }
        return admission.result();
    }

    private void timeoutControl(final BridgeConnection connection, final long generation, final long id) {
        final BridgePublisher.ControlDisposition disposition = connection.cancelControl(id);
        final CompletableFuture<BackendResult> future;
        synchronized (this) {
            final PendingControl current = this.pending.get(id);
            if (current == null || current.generation() != generation || current.connection() != connection) return;
            this.pending.remove(id, current);
            final ScheduledFuture<?> timeout = this.timeouts.remove(id);
            if (timeout != null) timeout.cancel(false);
            future = current.future();
        }
        if (disposition == BridgePublisher.ControlDisposition.DISPATCHED) {
            try {
                connection.close();
            } finally {
                future.complete(BackendResult.of(BackendResult.Code.BACKEND_TIMEOUT));
            }
        } else {
            future.complete(BackendResult.of(BackendResult.Code.BACKEND_TIMEOUT));
        }
    }

    CompletionStage<BackendResult> publishConfig(final xyz.jpenilla.squaremap.bridge.v1.ConfigReplace config) {
        synchronized (this) {
            final BridgeConnection connection = this.connection();
            if (connection == null || connection.isClosed()) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
            if (this.configPending != null && !this.configPending.isDone()) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
            final CompletableFuture<BackendResult> result = new CompletableFuture<>();
            this.configPending = result;
            this.configGeneration = this.connectionGeneration;
            final long generation = this.configGeneration;
            this.configPendingRevision = config.getRevision();
            this.configTimeout = this.scheduler.schedule(
                () -> this.completeConfig(generation, BackendResult.of(BackendResult.Code.BACKEND_TIMEOUT)),
                this.requestTimeout.toNanos(),
                TimeUnit.NANOSECONDS
            );
            try {
                connection.publish(new BridgeEvent.ReplaceState("config", Envelope.newBuilder().setConfigReplace(config).build()));
            } catch (final RuntimeException failure) {
                this.completeConfig(generation, BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
            }
            return result;
        }
    }

    private synchronized void completeConfig(final long generation, final BackendResult result) {
        if (this.configPending == null || this.configGeneration != generation) return;
        final CompletableFuture<BackendResult> future = this.configPending;
        this.configPending = null;
        this.configPendingRevision = 0L;
        final ScheduledFuture<?> timeout = this.configTimeout;
        this.configTimeout = null;
        if (timeout != null) timeout.cancel(false);
        future.complete(result);
    }

    private synchronized void completeConfig(final BackendResult result) {
        this.completeConfig(this.configGeneration, result);
    }

    public int pendingCount() {
        return this.pending.size();
    }

    public BridgePolicyReplace policy() {
        return this.policy.get();
    }

    public void dispatch(final Envelope envelope) {
        if (envelope == null) return;
        final BridgeConnection connection = this.listeningConnection;
        if (connection == null) return;
        if (!Arrays.equals(connection.sessionId(), envelope.getSessionId().toByteArray())) return;

        if (envelope.hasResumeWatermark()) {
            this.onResumeWatermark(envelope.getResumeWatermark());
            return;
        }
        if (envelope.hasDirtyReplayItem()) {
            this.onDirtyReplayItem(envelope.getDirtyReplayItem());
            return;
        }
        if (envelope.hasDirtyResyncComplete()) {
            this.onDirtyResyncComplete(envelope.getDirtyResyncComplete());
            return;
        }
        if (envelope.hasWorldResyncRequired()) {
            this.onWorldResyncRequired(envelope.getWorldResyncRequired());
            return;
        }
        if (envelope.hasProtocolError()) {
            final var error = envelope.getProtocolError();
            if (!error.getFatal() && error.getConfigRevision() != 0L && error.getConfigRevision() == this.configPendingRevision && this.configPending != null) {
                connection.rejectConfig(error.getConfigRevision());
                this.completeConfig(BackendResult.of(BackendResult.Code.INVALID_CONFIG));
            }
            return;
        }
        if (envelope.hasBridgePolicyReplace()) {
            final BridgePolicyReplace candidate = envelope.getBridgePolicyReplace();
            if (candidate.getRevision() != this.configPendingRevision || this.configPending == null) return;
            this.policy.set(candidate);
            connection.applyPolicy(candidate);
            this.completeConfig(BackendResult.of(BackendResult.Code.HEALTHY));
            return;
        }
        if (envelope.hasControlResult()) {
            this.complete(envelope.getCorrelationId(), fromProto(envelope.getControlResult()));
            return;
        }
        if (envelope.hasBackendHealth()) {
            this.complete(envelope.getCorrelationId(), BackendResult.of(BackendResult.Code.HEALTHY));
        }
    }

    public void clearPending(final BackendResult.Code code) {
        final BackendResult result = BackendResult.of(code);
        synchronized (this) {
            this.pending.forEach((id, entry) -> this.complete(id, entry.generation(), entry.connection(), result));
        }
    }

    private void clearPending(final long generation, final BackendResult.Code code) {
        final BackendResult result = BackendResult.of(code);
        this.pending.forEach((id, entry) -> {
            if (entry.generation() == generation) this.complete(id, generation, entry.connection(), result);
        });
    }

    private void complete(final long id, final BackendResult result) {
        final PendingControl entry = this.pending.get(id);
        if (entry != null) this.complete(id, entry.generation(), entry.connection(), result);
    }

    private void complete(final long id, final long generation, final BridgeConnection connection, final BackendResult result) {
        final PendingControl entry = this.pending.get(id);
        if (entry == null || entry.generation() != generation || entry.connection() != connection || !this.pending.remove(id, entry)) return;
        final ScheduledFuture<?> timeout = this.timeouts.remove(id);
        if (timeout != null) timeout.cancel(false);
        entry.future().complete(result);
    }

    private void failed(final BridgeConnection connection, final Throwable ignored) {
        if (this.listeningConnection != connection) return;
        final long generation = this.connectionGeneration;
        this.clearPending(generation, BackendResult.Code.BACKEND_UNAVAILABLE);
        this.completeConfig(generation, BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
        this.replayStates.clear();
        if (this.snapshotHandler != null) this.snapshotHandler.abortAll();
    }

    private BridgeConnection connection() {
        final BridgeConnection connection = this.supervisor == null ? this.listeningConnection : this.supervisor.currentConnection();
        if (connection != null && connection != this.listeningConnection) this.attach(connection);
        return connection;
    }

    private void ready(final Envelope envelope) {
        if (!envelope.hasReady()) return;
        this.readyHttpEnabled = envelope.getReady().getHttpEnabled();
        this.readyHttpPort = envelope.getReady().getHttpPort();
        Logging.logger().info("Rust backend ready (HTTP enabled={}, port={}, state revision={})", this.readyHttpEnabled, this.readyHttpPort, envelope.getReady().getStateRevision());
    }

    void attachForTest(final BridgeConnection connection) {
        this.attach(connection);
    }

    private synchronized void attach(final BridgeConnection connection) {
        final BridgeConnection next = Objects.requireNonNull(connection);
        final BridgeConnection previous = this.listeningConnection;
        if (previous == null) {
            this.connectionGeneration++;
            this.listeningConnection = next;
        } else if (previous != next) {
            final long oldGeneration = this.connectionGeneration;
            this.clearPending(oldGeneration, BackendResult.Code.BACKEND_UNAVAILABLE);
            if (this.configGeneration == oldGeneration) {
                this.completeConfig(oldGeneration, BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
            }
            this.replayStates.clear();
            this.connectionGeneration++;
            this.listeningConnection = next;
        } else {
            this.listeningConnection = next;
        }
        next.setReadyListener(this::ready);
        next.setResponseListener(this::dispatch);
        next.setReplayListener(this::dispatch);
        if (this.snapshotHandler != null) {
            next.setSnapshotRequestListener(envelope -> this.snapshotHandler.handle(envelope, next::publish));
            next.setAcknowledgementListener(this.snapshotHandler::acknowledge);
        }
        next.setFailureListener(failure -> this.failed(next, failure));
    }

    private void onResumeWatermark(final ResumeWatermark watermark) {
        final BridgeConnection connection = this.listeningConnection;
        if (connection == null || connection.isClosed()) return;
        if (this.worldManager == null) throw new IllegalStateException("world manager not available for replay");
        if (!MessageDigest.isEqual(connection.sessionId(), watermark.getSessionId().toByteArray())) return;

        final byte[] bridgeId = watermark.getBridgeId().toByteArray();
        final byte[] sessionId = watermark.getSessionId().toByteArray();
        final long configRevision = watermark.getConfigRevision();

        final WorldManager manager = this.worldManager.get();
        if (manager == null) throw new IllegalStateException("world manager not available for replay");
        final Collection<MapWorldInternal> worlds = manager.worlds();
        if (worlds == null || worlds.isEmpty()) return;

        for (final MapWorldInternal world : worlds) {
            final WorldIdentifier identifier = world.identifier();
            final long replayId = this.nextReplayId.incrementAndGet();
            final WorldIdentity protoWorld = this.identity(identifier);
            final DirtyReplayRequest request = DirtyReplayRequest.newBuilder()
                .setBridgeId(ByteString.copyFrom(bridgeId))
                .setSessionId(ByteString.copyFrom(sessionId))
                .setConfigRevision(configRevision)
                .setReplayId(replayId)
                .setWorld(protoWorld)
                .setMaxItems(MAX_REPLAY_ITEMS)
                .setStart(ReplayStart.getDefaultInstance())
                .build();
            this.replayStates.put(
                new ReplayKey(configRevision, replayId),
                new ReplayState(configRevision, replayId, bridgeId, sessionId, protoWorld)
            );
            final Envelope envelope = Envelope.newBuilder().setDirtyReplayRequest(request).build();
            connection.publish(new BridgeEvent.Transient(envelope));
        }
    }

    private void onDirtyReplayItem(final DirtyReplayItem item) {
        if (!item.hasWorld() || !item.hasCoordinate()) {
            throw new IllegalStateException("dirty replay item missing world or coordinate");
        }
        final ReplayState state = this.replayStates.get(new ReplayKey(item.getConfigRevision(), item.getReplayId()));
        if (state == null) throw new IllegalStateException("unexpected dirty replay item");
        if (state.configRevision != item.getConfigRevision()
            || !MessageDigest.isEqual(state.bridgeId, item.getBridgeId().toByteArray())
            || !MessageDigest.isEqual(state.sessionId, item.getSessionId().toByteArray())) {
            throw new IllegalStateException("dirty replay item identity mismatch");
        }
        final int expected = state.nextIndex.get();
        if (item.getItemIndex() != expected) {
            throw new IllegalStateException("dirty replay item index gap or duplicate: expected " + expected + " got " + item.getItemIndex());
        }
        if (!state.world.equals(item.getWorld())) {
            throw new IllegalStateException("dirty replay item world mismatch");
        }
        state.nextIndex.incrementAndGet();

        if (this.worldManager == null) throw new IllegalStateException("world manager not available for replay");
        final WorldManager manager = this.worldManager.get();
        if (manager == null) throw new IllegalStateException("world manager not available for replay");
        final WorldIdentifier itemWorld = WorldIdentifier.create(item.getWorld().getNamespace(), item.getWorld().getValue());
        final MapWorldInternal world = manager.getWorldIfEnabled(itemWorld)
            .orElseThrow(() -> new IllegalStateException("replay target world not enabled: " + itemWorld.asString()));
        final int x = item.getCoordinate().getX();
        final int z = item.getCoordinate().getZ();
        if (BackendLifecyclePolicy.javaDirtyOwner(this.mode)) {
            world.chunkModified(new ChunkCoordinate(x, z));
        }
    }

    private void onDirtyResyncComplete(final DirtyResyncComplete complete) {
        final ReplayState state = this.replayStates.remove(new ReplayKey(complete.getConfigRevision(), complete.getReplayId()));
        if (state == null) throw new IllegalStateException("unexpected dirty resync complete");
        if (complete.getStatus() != DirtyResyncStatus.DIRTY_RESYNC_STATUS_COMPLETE) {
            throw new IllegalStateException("dirty resync failed: " + complete.getFailureReason());
        }
        if (complete.getItemCount() != state.nextIndex.get()) {
            throw new IllegalStateException("dirty resync item count mismatch: expected " + state.nextIndex.get() + " got " + complete.getItemCount());
        }
        if (complete.getHasMore()) {
            if (!complete.hasLastCoordinate()) {
                throw new IllegalStateException("dirty resync has_more without last_coordinate");
            }
            final BridgeConnection connection = this.listeningConnection;
            if (connection == null || connection.isClosed()) return;
            final long nextReplayId = this.nextReplayId.incrementAndGet();
            final DirtyReplayRequest next = DirtyReplayRequest.newBuilder()
                .setBridgeId(ByteString.copyFrom(state.bridgeId))
                .setSessionId(ByteString.copyFrom(state.sessionId))
                .setConfigRevision(state.configRevision)
                .setReplayId(nextReplayId)
                .setWorld(state.world)
                .setMaxItems(MAX_REPLAY_ITEMS)
                .setContinuation(complete.getLastCoordinate())
                .build();
            final ReplayKey nextKey = new ReplayKey(state.configRevision, nextReplayId);
            this.replayStates.put(nextKey, new ReplayState(state.configRevision, nextReplayId, state.bridgeId, state.sessionId, state.world));
            final Envelope nextEnvelope = Envelope.newBuilder().setDirtyReplayRequest(next).build();
            connection.publish(new BridgeEvent.Transient(nextEnvelope));
        }
    }

    private void onWorldResyncRequired(final WorldResyncRequired required) {
        if (!required.hasWorld()) throw new IllegalStateException("world resync required without a world");
        final WorldIdentity proto = required.getWorld();
        final WorldIdentifier world = WorldIdentifier.create(proto.getNamespace(), proto.getValue());
        this.execute(new BackendController.FullRender(world)).whenComplete((result, error) -> {
            if (error != null) {
                Logging.logger().warn("Failed to dispatch resync full render for {}", world, error);
            } else if (result != null) {
                Logging.logger().info("Resync full render for {} completed with {}", world, result.code());
            }
        });
    }

    private record ReplayKey(long configRevision, long replayId) {}

    private static final class ReplayState {
        final long configRevision;
        final long replayId;
        final byte[] bridgeId;
        final byte[] sessionId;
        final WorldIdentity world;
        final java.util.concurrent.atomic.AtomicInteger nextIndex = new java.util.concurrent.atomic.AtomicInteger(0);

        ReplayState(
            final long configRevision,
            final long replayId,
            final byte[] bridgeId,
            final byte[] sessionId,
            final WorldIdentity world
        ) {
            this.configRevision = configRevision;
            this.replayId = replayId;
            this.bridgeId = bridgeId.clone();
            this.sessionId = sessionId.clone();
            this.world = world;
        }
    }


    private record PendingControl(CompletableFuture<BackendResult> future, BridgeConnection connection, long generation) {}

    private record ConnectionAdmission(BridgeConnection connection, long generation, long correlationId, CompletableFuture<BackendResult> result) {}

    private long allocateCorrelationId() {
        for (;;) {
            final long id = this.nextCorrelation.incrementAndGet();
            if (id <= 0 || id == MAX_CORRELATION_ID) throw new IllegalStateException("control correlation ID space exhausted");
            if (!this.pending.containsKey(id)) return id;
        }
    }

    private static ControlKind kind(final BackendController.BackendRequest request) {
        return switch (request) {
            case BackendController.FullRender ignored -> ControlKind.CONTROL_KIND_FULL_RENDER;
            case BackendController.RadiusRender ignored -> ControlKind.CONTROL_KIND_RADIUS_RENDER;
            case BackendController.CancelRender ignored -> ControlKind.CONTROL_KIND_CANCEL_RENDER;
            case BackendController.PauseRenders ignored -> ControlKind.CONTROL_KIND_PAUSE_RENDERS;
            case BackendController.ResetMap ignored -> ControlKind.CONTROL_KIND_RESET_MAP;
            case BackendController.RestartProgressLogging ignored -> ControlKind.CONTROL_KIND_UNSPECIFIED;
            case BackendController.ConfigSync ignored -> ControlKind.CONTROL_KIND_UNSPECIFIED;
            case BackendController.Reload ignored -> ControlKind.CONTROL_KIND_RELOAD;
            case BackendController.Health ignored -> ControlKind.CONTROL_KIND_HEALTH;
        };
    }

    private WorldIdentity identity(final WorldIdentifier world) {
        return WorldIdentity.newBuilder()
            .setNamespace(world.namespace())
            .setValue(world.value())
            .setEpoch(this.epochs.epoch(world))
            .build();
    }

    private static BackendResult fromProto(final ControlResult result) {
        final BackendResult.Code code = switch (result.getCode()) {
            case BACKEND_RESULT_CODE_FULL_RENDER_STARTED -> BackendResult.Code.FULL_RENDER_STARTED;
            case BACKEND_RESULT_CODE_RADIUS_RENDER_STARTED -> BackendResult.Code.RADIUS_RENDER_STARTED;
            case BACKEND_RESULT_CODE_RENDER_IN_PROGRESS -> BackendResult.Code.RENDER_IN_PROGRESS;
            case BACKEND_RESULT_CODE_RENDER_NOT_IN_PROGRESS -> BackendResult.Code.RENDER_NOT_IN_PROGRESS;
            case BACKEND_RESULT_CODE_RENDER_CANCELLED -> BackendResult.Code.RENDER_CANCELLED;
            case BACKEND_RESULT_CODE_RENDERS_PAUSED -> BackendResult.Code.RENDERS_PAUSED;
            case BACKEND_RESULT_CODE_RENDERS_RESUMED -> BackendResult.Code.RENDERS_RESUMED;
            case BACKEND_RESULT_CODE_MAP_RESET -> BackendResult.Code.MAP_RESET;
            case BACKEND_RESULT_CODE_RELOADED -> BackendResult.Code.RELOADED;
            case BACKEND_RESULT_CODE_HEALTHY -> BackendResult.Code.HEALTHY;
            case BACKEND_RESULT_CODE_UNKNOWN_WORLD -> BackendResult.Code.UNKNOWN_WORLD;
            case BACKEND_RESULT_CODE_INVALID_REQUEST -> BackendResult.Code.INVALID_REQUEST;
            case BACKEND_RESULT_CODE_INVALID_CONFIG -> BackendResult.Code.INVALID_CONFIG;
            case BACKEND_RESULT_CODE_BACKEND_UNAVAILABLE -> BackendResult.Code.BACKEND_UNAVAILABLE;
            case BACKEND_RESULT_CODE_BACKEND_TIMEOUT -> BackendResult.Code.BACKEND_TIMEOUT;
            case BACKEND_RESULT_CODE_FAILED, BACKEND_RESULT_CODE_UNSPECIFIED, UNRECOGNIZED -> BackendResult.Code.FAILED;
        };
        final List<BackendResult.Substitution> substitutions = new ArrayList<>();
        for (final BackendSubstitution substitution : result.getSubstitutionsList()) {
            final BackendResult.Value value = switch (substitution.getValueCase()) {
                case TEXT -> new BackendResult.Text(substitution.getText());
                case INTEGER -> new BackendResult.Integer(substitution.getInteger());
                case BOOLEAN -> new BackendResult.Boolean(substitution.getBoolean());
                case WORLD_IDENTITY -> new BackendResult.World(WorldIdentifier.create(
                    substitution.getWorldIdentity().getNamespace(),
                    substitution.getWorldIdentity().getValue()
                ));
                case VALUE_NOT_SET -> new BackendResult.Text("");
            };
            substitutions.add(new BackendResult.Substitution(substitution.getKey(), value));
        }
        return new BackendResult(code, substitutions);
    }

    public void abortForRestart() {
        this.clearPending(BackendResult.Code.BACKEND_UNAVAILABLE);
        this.completeConfig(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
        this.replayStates.clear();
        if (this.snapshotHandler != null) this.snapshotHandler.abortAll();
    }

    @Override
    public void close() {
        this.abortForRestart();
        if (this.snapshotHandler != null) this.snapshotHandler.close();
        this.scheduler.shutdownNow();
    }
}
