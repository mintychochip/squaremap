package xyz.jpenilla.squaremap.common.backend;

import com.google.inject.Inject;
import com.google.inject.Provider;
import com.google.inject.Singleton;
import java.time.Duration;
import java.util.ArrayList;
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
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.bridge.v1.BackendResultCode;
import xyz.jpenilla.squaremap.bridge.v1.BackendSubstitution;
import xyz.jpenilla.squaremap.bridge.v1.BridgePolicyReplace;
import xyz.jpenilla.squaremap.bridge.v1.ControlKind;
import xyz.jpenilla.squaremap.bridge.v1.ControlRequest;
import xyz.jpenilla.squaremap.bridge.v1.ControlResult;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;
import xyz.jpenilla.squaremap.common.ServerAccess;
import xyz.jpenilla.squaremap.common.SquaremapCommon;
import xyz.jpenilla.squaremap.common.bridge.process.BackendMode;
import xyz.jpenilla.squaremap.common.bridge.process.BridgeBootstrapConfig;
import xyz.jpenilla.squaremap.common.bridge.process.BridgeConnection;
import xyz.jpenilla.squaremap.common.bridge.process.SidecarSupervisor;
import xyz.jpenilla.squaremap.common.bridge.state.WorldEpochRegistry;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher;
import xyz.jpenilla.squaremap.common.config.ConfigBridgeExporter;

/** Rust control implementation with bounded correlation and timeout handling. */
@Singleton
public final class BridgeBackendController implements AutoCloseable {
    public static final Duration REQUEST_TIMEOUT = Duration.ofSeconds(10);
    private static final long MAX_CORRELATION_ID = Long.MAX_VALUE;
    private static final int MAX_PENDING = 1_024;
    private final SidecarSupervisor supervisor;
    private final ScheduledExecutorService scheduler;
    private final Duration requestTimeout;
    private final BackendMode mode;
    private final ConfigBridgeExporter configExporter;
    private final Provider<SquaremapCommon> common;
    private final EpochResolver epochs;
    private final ConcurrentMap<Long, CompletableFuture<BackendResult>> pending = new ConcurrentHashMap<>();
    private final ConcurrentMap<Long, ScheduledFuture<?>> timeouts = new ConcurrentHashMap<>();
    private final AtomicLong nextCorrelation = new AtomicLong(0L);
    private final AtomicReference<BridgePolicyReplace> policy = new AtomicReference<>();
    private volatile CompletableFuture<BackendResult> configPending;
    private volatile ScheduledFuture<?> configTimeout;
    private volatile long configPendingRevision;
    private volatile BridgeConnection listeningConnection;

    @Inject
    public BridgeBackendController(final BridgeBootstrapConfig config, final SidecarSupervisor supervisor,
                                   final ConfigBridgeExporter configExporter, final Provider<SquaremapCommon> common,
                                   final ServerAccess serverAccess, final WorldEpochRegistry epochs) {
        this(supervisor, defaultScheduler(), REQUEST_TIMEOUT, config.backendMode(), configExporter, common,
            world -> {
                final net.minecraft.server.level.ServerLevel level = serverAccess.level(world);
                return level == null ? 0L : epochs.epoch(world, level);
            });
    }

    private static ScheduledExecutorService defaultScheduler() {
        return Executors.newSingleThreadScheduledExecutor(runnable -> {
            final Thread thread = new Thread(runnable, "squaremap-backend-control");
            thread.setDaemon(true);
            return thread;
        });
    }

    /** Embedded constructor retaining the pre-existing test seam. */
    public BridgeBackendController(final SidecarSupervisor supervisor, final ScheduledExecutorService scheduler) {
        this(supervisor, scheduler, REQUEST_TIMEOUT, BackendMode.RUST, null, null, world -> 0L);
    }

    public BridgeBackendController(final BridgeConnection connection, final ScheduledExecutorService scheduler) {
        this(connection, scheduler, REQUEST_TIMEOUT, world -> 0L);
    }

    public BridgeBackendController(final BridgeConnection connection, final ScheduledExecutorService scheduler,
                                   final Duration requestTimeout, final EpochResolver epochs) {
        this(null, scheduler, requestTimeout, BackendMode.RUST, null, null, epochs);
        this.attach(connection);
    }

    private BridgeBackendController(final SidecarSupervisor supervisor, final ScheduledExecutorService scheduler,
                                    final Duration requestTimeout, final BackendMode mode,
                                    final ConfigBridgeExporter configExporter, final Provider<SquaremapCommon> common,
                                    final EpochResolver epochs) {
        this.supervisor = supervisor;
        this.scheduler = Objects.requireNonNull(scheduler, "scheduler");
        this.requestTimeout = Objects.requireNonNull(requestTimeout, "requestTimeout");
        this.mode = Objects.requireNonNull(mode, "mode");
        this.configExporter = configExporter;
        this.common = common;
        this.epochs = Objects.requireNonNull(epochs, "epochs");
    }

    @FunctionalInterface
    interface EpochResolver { long epoch(WorldIdentifier world); }

    CompletionStage<BackendResult> execute(final BackendController.BackendRequest request) {
        if (request instanceof BackendController.ConfigSync sync) return this.publishConfig(sync.config());
        if (request instanceof BackendController.RestartProgressLogging) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.INVALID_REQUEST));
        if (request instanceof BackendController.Reload) {
            if (this.mode == BackendMode.RUST && this.common != null) this.common.get().reloadForBridge();
            if (this.configExporter == null) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
            return this.publishConfig(this.configExporter.export()).thenApply(result -> {
                if (result.code() != BackendResult.Code.HEALTHY) return result;
                if (this.common == null) return BackendResult.of(BackendResult.Code.RELOADED);
                return new BackendResult(BackendResult.Code.RELOADED,
                    List.of(new BackendResult.Substitution("version", new BackendResult.Text(this.common.get().version()))));
            });
        }
        final BridgeConnection connection = this.connection();
        if (connection == null || connection.isClosed()) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
        if (request instanceof BackendController.RadiusRender radius && radius.radius() < 1) {
            return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.INVALID_REQUEST));
        }
        if (this.pending.size() >= MAX_PENDING) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
        final long correlationId = this.allocateCorrelationId();
        final CompletableFuture<BackendResult> result = new CompletableFuture<>();
        this.pending.put(correlationId, result);
        final ControlRequest.Builder control = ControlRequest.newBuilder().setKind(kind(request));
        if (request.world() != null) control.setWorld(identity(request.world()));
        if (request instanceof BackendController.RadiusRender radius) {
            control.setCenterX(radius.centerX()).setCenterZ(radius.centerZ()).setRadius(radius.radius());
        }
        try {
            connection.publishControl(correlationId, control.build());
        } catch (final RuntimeException failure) {
            this.complete(correlationId, BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
            return result;
        }
        final ScheduledFuture<?> timeout = this.scheduler.schedule(
            () -> this.timeoutControl(connection, correlationId), this.requestTimeout.toNanos(), TimeUnit.NANOSECONDS);
        this.timeouts.put(correlationId, timeout);
        return result;
    }

    private void timeoutControl(final BridgeConnection connection, final long correlationId) {
        final BridgePublisher.ControlDisposition disposition = connection.cancelControl(correlationId);
        final CompletableFuture<BackendResult> future = this.pending.remove(correlationId);
        final ScheduledFuture<?> timeout = this.timeouts.remove(correlationId);
        if (timeout != null) timeout.cancel(false);
        if (future == null) return;
        if (disposition == BridgePublisher.ControlDisposition.DISPATCHED) {
            try {
                connection.close();
            } finally {
                future.complete(BackendResult.of(BackendResult.Code.BACKEND_TIMEOUT));
            }
            return;
        }
        future.complete(BackendResult.of(BackendResult.Code.BACKEND_TIMEOUT));
    }

    CompletionStage<BackendResult> publishConfig(final xyz.jpenilla.squaremap.bridge.v1.ConfigReplace config) {
        final BridgeConnection connection = this.connection();
        if (connection == null || connection.isClosed()) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
        synchronized (this) {
            if (this.configPending != null && !this.configPending.isDone()) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
            final CompletableFuture<BackendResult> result = new CompletableFuture<>();
            this.configPending = result;
            this.configPendingRevision = config.getRevision();
            this.configTimeout = this.scheduler.schedule(
                () -> this.completeConfig(BackendResult.of(BackendResult.Code.BACKEND_TIMEOUT)),
                this.requestTimeout.toNanos(), TimeUnit.NANOSECONDS);
            try {
                connection.publish(new xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent.ReplaceState(
                    "config", Envelope.newBuilder().setConfigReplace(config).build()));
            } catch (RuntimeException failure) {
                this.completeConfig(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
            }
            return result;
        }
    }

    private synchronized void completeConfig(final BackendResult result) {
        final CompletableFuture<BackendResult> resultFuture = this.configPending;
        this.configPending = null;
        this.configPendingRevision = 0L;
        final ScheduledFuture<?> timeout = this.configTimeout;
        this.configTimeout = null;
        if (timeout != null) timeout.cancel(false);
        if (resultFuture != null) resultFuture.complete(result);
    }

    public int pendingCount() { return this.pending.size(); }
    public BridgePolicyReplace policy() { return this.policy.get(); }

    public void dispatch(final Envelope envelope) {
        if (envelope == null || (!envelope.hasControlResult() && !envelope.hasBackendHealth()
            && !envelope.hasBridgePolicyReplace() && !envelope.hasProtocolError())) return;
        final BridgeConnection connection = this.listeningConnection;
        if (connection == null || !java.util.Arrays.equals(connection.sessionId(), envelope.getSessionId().toByteArray())) return;
        if (envelope.hasProtocolError()) {
            final xyz.jpenilla.squaremap.bridge.v1.ProtocolError error = envelope.getProtocolError();
            if (!error.getFatal() && error.getConfigRevision() != 0L
                && error.getConfigRevision() == this.configPendingRevision && this.configPending != null) {
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
        final BackendResult result = envelope.hasControlResult() ? fromProto(envelope.getControlResult()) : BackendResult.of(BackendResult.Code.HEALTHY);
        this.complete(envelope.getCorrelationId(), result);
    }

    public void clearPending(final BackendResult.Code code) {
        final BackendResult result = BackendResult.of(code);
        this.pending.forEach((id, future) -> this.complete(id, result));
    }

    private void complete(final long id, final BackendResult result) {
        final CompletableFuture<BackendResult> future = this.pending.remove(id);
        final ScheduledFuture<?> timeout = this.timeouts.remove(id);
        if (timeout != null) timeout.cancel(false);
        if (future != null) future.complete(result);
    }

    private void failed(final Throwable ignored) {
        this.clearPending(BackendResult.Code.BACKEND_UNAVAILABLE);
        this.completeConfig(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
    }

    private BridgeConnection connection() {
        final BridgeConnection connection = this.supervisor == null ? this.listeningConnection : this.supervisor.currentConnection();
        if (connection != null && connection != this.listeningConnection) this.attach(connection);
        return connection;
    }

    private void attach(final BridgeConnection connection) {
        this.listeningConnection = Objects.requireNonNull(connection, "connection");
        connection.setResponseListener(this::dispatch);
        connection.setFailureListener(this::failed);
    }

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
        return WorldIdentity.newBuilder().setNamespace(world.namespace()).setValue(world.value()).setEpoch(this.epochs.epoch(world)).build();
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
                case WORLD_IDENTITY -> new BackendResult.World(WorldIdentifier.create(substitution.getWorldIdentity().getNamespace(), substitution.getWorldIdentity().getValue()));
                case VALUE_NOT_SET -> new BackendResult.Text("");
            };
            substitutions.add(new BackendResult.Substitution(substitution.getKey(), value));
        }
        return new BackendResult(code, substitutions);
    }

    @Override public void close() {
        this.clearPending(BackendResult.Code.BACKEND_UNAVAILABLE);
        this.completeConfig(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
        this.scheduler.shutdownNow();
    }
}
