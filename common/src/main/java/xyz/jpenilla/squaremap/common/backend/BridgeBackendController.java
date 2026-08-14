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
import xyz.jpenilla.squaremap.common.Logging;
import xyz.jpenilla.squaremap.common.ServerAccess;
import xyz.jpenilla.squaremap.common.SquaremapCommon;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher;
import xyz.jpenilla.squaremap.common.bridge.process.BackendMode;
import xyz.jpenilla.squaremap.common.bridge.process.BridgeBootstrapConfig;
import xyz.jpenilla.squaremap.common.bridge.process.BridgeConnection;
import xyz.jpenilla.squaremap.common.bridge.process.SidecarSupervisor;
import xyz.jpenilla.squaremap.common.bridge.snapshot.SnapshotRequestHandler;
import xyz.jpenilla.squaremap.common.bridge.state.WorldEpochRegistry;
import xyz.jpenilla.squaremap.common.config.ConfigBridgeExporter;

/** Rust control implementation with bounded correlation and timeout handling. */
@Singleton
public final class BridgeBackendController implements AutoCloseable {
    public static final Duration REQUEST_TIMEOUT = Duration.ofSeconds(10);
    private static final long MAX_CORRELATION_ID = Long.MAX_VALUE;
    private static final int MAX_PENDING = 1_024;
    private final SidecarSupervisor supervisor; private final ScheduledExecutorService scheduler; private final Duration requestTimeout; private final BackendMode mode; private final ConfigBridgeExporter configExporter; private final Provider<SquaremapCommon> common; private final EpochResolver epochs; private final SnapshotRequestHandler snapshotHandler;
    private final ConcurrentMap<Long, PendingControl> pending = new ConcurrentHashMap<>(); private final ConcurrentMap<Long, ScheduledFuture<?>> timeouts = new ConcurrentHashMap<>(); private final AtomicLong nextCorrelation = new AtomicLong(0L); private final AtomicReference<BridgePolicyReplace> policy = new AtomicReference<>();
    private volatile int readyHttpPort; private volatile boolean readyHttpEnabled; private volatile CompletableFuture<BackendResult> configPending; private volatile ScheduledFuture<?> configTimeout; private volatile long configPendingRevision; private volatile long configGeneration; private volatile BridgeConnection listeningConnection; private long connectionGeneration;
    @Inject public BridgeBackendController(final BridgeBootstrapConfig config, final SidecarSupervisor supervisor, final ConfigBridgeExporter configExporter, final Provider<SquaremapCommon> common, final ServerAccess serverAccess, final WorldEpochRegistry epochs, final SnapshotRequestHandler snapshotHandler) { this(supervisor, defaultScheduler(), REQUEST_TIMEOUT, config.backendMode(), configExporter, common, world -> { final net.minecraft.server.level.ServerLevel level = serverAccess.level(world); return level == null ? 0L : epochs.epoch(world, level); }, snapshotHandler); }
    private static ScheduledExecutorService defaultScheduler() { return Executors.newSingleThreadScheduledExecutor(runnable -> { final Thread thread = new Thread(runnable, "squaremap-backend-control"); thread.setDaemon(true); return thread; }); }
    public BridgeBackendController(final SidecarSupervisor supervisor, final ScheduledExecutorService scheduler) { this(supervisor, scheduler, REQUEST_TIMEOUT, BackendMode.RUST, null, null, world -> 0L, null); }
    public BridgeBackendController(final BridgeConnection connection, final ScheduledExecutorService scheduler) { this(connection, scheduler, REQUEST_TIMEOUT, world -> 0L); }
    public BridgeBackendController(final BridgeConnection connection, final ScheduledExecutorService scheduler, final Duration requestTimeout, final EpochResolver epochs) { this(null, scheduler, requestTimeout, BackendMode.RUST, null, null, epochs, null); this.attach(connection); }
    private BridgeBackendController(final SidecarSupervisor supervisor, final ScheduledExecutorService scheduler, final Duration requestTimeout, final BackendMode mode, final ConfigBridgeExporter configExporter, final Provider<SquaremapCommon> common, final EpochResolver epochs, final SnapshotRequestHandler snapshotHandler) { this.supervisor=supervisor; this.snapshotHandler=snapshotHandler; this.scheduler=Objects.requireNonNull(scheduler); this.requestTimeout=Objects.requireNonNull(requestTimeout); this.mode=Objects.requireNonNull(mode); this.configExporter=configExporter; this.common=common; if (supervisor != null) supervisor.setReconnectListener(this::attach); this.epochs=Objects.requireNonNull(epochs); }
    @FunctionalInterface interface EpochResolver { long epoch(WorldIdentifier world); }
    public BridgePublisher.PublishResult publishDirty(final xyz.jpenilla.squaremap.common.data.MapWorldInternal world, final xyz.jpenilla.squaremap.common.data.ChunkCoordinate coordinate, final long revision) { final BridgeConnection connection=this.connection(); if(connection==null||connection.isClosed())return BridgePublisher.PublishResult.COALESCED; final WorldIdentifier identifier=world.identifier(); return connection.publish(new xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent.DirtyChunk(new xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent.WorldKey(identifier.namespace(),identifier.value()),this.epochs.epoch(identifier),coordinate.x(),coordinate.z(),revision)); }
    CompletionStage<BackendResult> execute(final BackendController.BackendRequest request) {
        if (request instanceof BackendController.Reload) {
            if (this.common == null) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.INVALID_REQUEST));
            this.common.get().reload();
            return CompletableFuture.completedFuture(new BackendResult(
                BackendResult.Code.RELOADED,
                java.util.List.of(new BackendResult.Substitution("version", new BackendResult.Text(this.common.get().version())))
            ));
        }
        if (request instanceof BackendController.ConfigSync sync) return this.publishConfig(sync.config());
        if (request instanceof BackendController.RestartProgressLogging) return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.INVALID_REQUEST));
        final ConnectionAdmission admission;
        synchronized (this) {
            final BridgeConnection connection=this.connection(); if(connection==null||connection.isClosed())return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
            if(request instanceof BackendController.RadiusRender radius&&radius.radius()<1)return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.INVALID_REQUEST));
            if(pending.size()>=MAX_PENDING)return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));
            final long id=allocateCorrelationId(); final CompletableFuture<BackendResult> result=new CompletableFuture<>(); final long generation=this.connectionGeneration; pending.put(id,new PendingControl(result,connection,generation)); admission=new ConnectionAdmission(connection,generation,id,result);
            final ControlRequest.Builder control=ControlRequest.newBuilder().setKind(kind(request)); if(request.world()!=null)control.setWorld(identity(request.world())); if(request instanceof BackendController.RadiusRender radius)control.setCenterX(radius.centerX()).setCenterZ(radius.centerZ()).setRadius(radius.radius());
            try { connection.publishControl(id,control.build()); } catch(RuntimeException failure) { complete(id,BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE)); return result; }
        }
        final ScheduledFuture<?> timeout=this.scheduler.schedule(()->timeoutControl(admission.connection(),admission.generation(),admission.correlationId()),requestTimeout.toNanos(),TimeUnit.NANOSECONDS); synchronized(this){ final PendingControl current=pending.get(admission.correlationId()); if(current!=null&&current.generation()==admission.generation()&&current.connection()==admission.connection())timeouts.put(admission.correlationId(),timeout); else timeout.cancel(false); } return admission.result();
    }
    private void timeoutControl(final BridgeConnection connection, final long generation, final long id) { final BridgePublisher.ControlDisposition disposition=connection.cancelControl(id); final CompletableFuture<BackendResult> future; synchronized(this){ final PendingControl current=pending.get(id); if(current==null||current.generation()!=generation||current.connection()!=connection)return; pending.remove(id,current); final ScheduledFuture<?> timeout=timeouts.remove(id); if(timeout!=null)timeout.cancel(false); future=current.future(); } if(disposition==BridgePublisher.ControlDisposition.DISPATCHED){try{connection.close();}finally{future.complete(BackendResult.of(BackendResult.Code.BACKEND_TIMEOUT));}}else future.complete(BackendResult.of(BackendResult.Code.BACKEND_TIMEOUT)); }
    CompletionStage<BackendResult> publishConfig(final xyz.jpenilla.squaremap.bridge.v1.ConfigReplace config) { synchronized(this){ final BridgeConnection connection=connection(); if(connection==null||connection.isClosed())return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE)); if(configPending!=null&&!configPending.isDone())return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE)); final CompletableFuture<BackendResult> result=new CompletableFuture<>(); configPending=result; configGeneration=connectionGeneration; final long generation=configGeneration; configPendingRevision=config.getRevision(); configTimeout=scheduler.schedule(()->completeConfig(generation,BackendResult.of(BackendResult.Code.BACKEND_TIMEOUT)),requestTimeout.toNanos(),TimeUnit.NANOSECONDS); try{connection.publish(new xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent.ReplaceState("config",Envelope.newBuilder().setConfigReplace(config).build()));}catch(RuntimeException failure){completeConfig(generation,BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));} return result; } }
    private synchronized void completeConfig(final long generation, final BackendResult result){if(configPending==null||configGeneration!=generation)return; final CompletableFuture<BackendResult> future=configPending; configPending=null; configPendingRevision=0L; final ScheduledFuture<?> timeout=configTimeout; configTimeout=null; if(timeout!=null)timeout.cancel(false); future.complete(result);}
    private synchronized void completeConfig(final BackendResult result){completeConfig(configGeneration,result);}
    public int pendingCount(){return pending.size();} public BridgePolicyReplace policy(){return policy.get();}
    public void dispatch(final Envelope envelope){if(envelope==null||(!envelope.hasControlResult()&&!envelope.hasBackendHealth()&&!envelope.hasBridgePolicyReplace()&&!envelope.hasProtocolError()))return; final BridgeConnection connection=listeningConnection; if(connection==null||!java.util.Arrays.equals(connection.sessionId(),envelope.getSessionId().toByteArray()))return; if(envelope.hasProtocolError()){final var error=envelope.getProtocolError(); if(!error.getFatal()&&error.getConfigRevision()!=0L&&error.getConfigRevision()==configPendingRevision&&configPending!=null)completeConfig(BackendResult.of(BackendResult.Code.INVALID_CONFIG)); return;} if(envelope.hasBridgePolicyReplace()){final BridgePolicyReplace candidate=envelope.getBridgePolicyReplace(); if(candidate.getRevision()!=configPendingRevision||configPending==null)return; policy.set(candidate); connection.applyPolicy(candidate); completeConfig(BackendResult.of(BackendResult.Code.HEALTHY)); return;} complete(envelope.getCorrelationId(),envelope.hasControlResult()?fromProto(envelope.getControlResult()):BackendResult.of(BackendResult.Code.HEALTHY));}
    public void clearPending(final BackendResult.Code code){final BackendResult result=BackendResult.of(code); synchronized(this){pending.forEach((id,entry)->complete(id,entry.generation(),entry.connection(),result));}}
    private void clearPending(final long generation, final BackendResult.Code code){final BackendResult result=BackendResult.of(code); pending.forEach((id,entry)->{if(entry.generation()==generation)complete(id,generation,entry.connection(),result);});}
    private void complete(final long id, final BackendResult result){final PendingControl entry=pending.get(id); if(entry!=null)complete(id,entry.generation(),entry.connection(),result);}
    private void complete(final long id, final long generation, final BridgeConnection connection, final BackendResult result){final PendingControl entry=pending.get(id); if(entry==null||entry.generation()!=generation||entry.connection()!=connection||!pending.remove(id,entry))return; final ScheduledFuture<?> timeout=timeouts.remove(id); if(timeout!=null)timeout.cancel(false); entry.future().complete(result);}
    private void failed(final BridgeConnection connection, final Throwable ignored){if(listeningConnection!=connection)return; final long generation=connectionGeneration; clearPending(generation,BackendResult.Code.BACKEND_UNAVAILABLE); completeConfig(generation,BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE)); if(snapshotHandler!=null)snapshotHandler.abortAll();}
    private BridgeConnection connection(){final BridgeConnection connection=supervisor==null?listeningConnection:supervisor.currentConnection(); if(connection!=null&&connection!=listeningConnection)attach(connection); return connection;}
    private void ready(final Envelope envelope){if(!envelope.hasReady())return; readyHttpEnabled=envelope.getReady().getHttpEnabled(); readyHttpPort=envelope.getReady().getHttpPort(); Logging.logger().info("Rust backend ready (HTTP enabled={}, port={}, state revision={})",readyHttpEnabled,readyHttpPort,envelope.getReady().getStateRevision());}
    void attachForTest(final BridgeConnection connection){attach(connection);}
    private synchronized void attach(final BridgeConnection connection){final BridgeConnection next=Objects.requireNonNull(connection); final BridgeConnection previous=listeningConnection; if(previous==null){connectionGeneration++; listeningConnection=next;}else if(previous!=next){final long oldGeneration=connectionGeneration; clearPending(oldGeneration,BackendResult.Code.BACKEND_UNAVAILABLE); if(configGeneration==oldGeneration)completeConfig(oldGeneration,BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE)); connectionGeneration++; listeningConnection=next;}else listeningConnection=next; next.setReadyListener(this::ready); next.setResponseListener(this::dispatch); if(snapshotHandler!=null){next.setSnapshotRequestListener(envelope->snapshotHandler.handle(envelope,next::publish));next.setAcknowledgementListener(snapshotHandler::acknowledge);} next.setFailureListener(failure->failed(next,failure));}
    private record PendingControl(CompletableFuture<BackendResult> future,BridgeConnection connection,long generation){} private record ConnectionAdmission(BridgeConnection connection,long generation,long correlationId,CompletableFuture<BackendResult> result){}
    private long allocateCorrelationId(){for(;;){final long id=nextCorrelation.incrementAndGet();if(id<=0||id==MAX_CORRELATION_ID)throw new IllegalStateException("control correlation ID space exhausted");if(!pending.containsKey(id))return id;}}
    private static ControlKind kind(final BackendController.BackendRequest request){return switch(request){case BackendController.FullRender ignored->ControlKind.CONTROL_KIND_FULL_RENDER;case BackendController.RadiusRender ignored->ControlKind.CONTROL_KIND_RADIUS_RENDER;case BackendController.CancelRender ignored->ControlKind.CONTROL_KIND_CANCEL_RENDER;case BackendController.PauseRenders ignored->ControlKind.CONTROL_KIND_PAUSE_RENDERS;case BackendController.ResetMap ignored->ControlKind.CONTROL_KIND_RESET_MAP;case BackendController.RestartProgressLogging ignored->ControlKind.CONTROL_KIND_UNSPECIFIED;case BackendController.ConfigSync ignored->ControlKind.CONTROL_KIND_UNSPECIFIED;case BackendController.Reload ignored->ControlKind.CONTROL_KIND_RELOAD;case BackendController.Health ignored->ControlKind.CONTROL_KIND_HEALTH;};}
    private WorldIdentity identity(final WorldIdentifier world){return WorldIdentity.newBuilder().setNamespace(world.namespace()).setValue(world.value()).setEpoch(epochs.epoch(world)).build();}
    private static BackendResult fromProto(final ControlResult result){final BackendResult.Code code=switch(result.getCode()){case BACKEND_RESULT_CODE_FULL_RENDER_STARTED->BackendResult.Code.FULL_RENDER_STARTED;case BACKEND_RESULT_CODE_RADIUS_RENDER_STARTED->BackendResult.Code.RADIUS_RENDER_STARTED;case BACKEND_RESULT_CODE_RENDER_IN_PROGRESS->BackendResult.Code.RENDER_IN_PROGRESS;case BACKEND_RESULT_CODE_RENDER_NOT_IN_PROGRESS->BackendResult.Code.RENDER_NOT_IN_PROGRESS;case BACKEND_RESULT_CODE_RENDER_CANCELLED->BackendResult.Code.RENDER_CANCELLED;case BACKEND_RESULT_CODE_RENDERS_PAUSED->BackendResult.Code.RENDERS_PAUSED;case BACKEND_RESULT_CODE_RENDERS_RESUMED->BackendResult.Code.RENDERS_RESUMED;case BACKEND_RESULT_CODE_MAP_RESET->BackendResult.Code.MAP_RESET;case BACKEND_RESULT_CODE_RELOADED->BackendResult.Code.RELOADED;case BACKEND_RESULT_CODE_HEALTHY->BackendResult.Code.HEALTHY;case BACKEND_RESULT_CODE_UNKNOWN_WORLD->BackendResult.Code.UNKNOWN_WORLD;case BACKEND_RESULT_CODE_INVALID_REQUEST->BackendResult.Code.INVALID_REQUEST;case BACKEND_RESULT_CODE_INVALID_CONFIG->BackendResult.Code.INVALID_CONFIG;case BACKEND_RESULT_CODE_BACKEND_UNAVAILABLE->BackendResult.Code.BACKEND_UNAVAILABLE;case BACKEND_RESULT_CODE_BACKEND_TIMEOUT->BackendResult.Code.BACKEND_TIMEOUT;case BACKEND_RESULT_CODE_FAILED,BACKEND_RESULT_CODE_UNSPECIFIED,UNRECOGNIZED->BackendResult.Code.FAILED;}; final List<BackendResult.Substitution> substitutions=new ArrayList<>(); for(BackendSubstitution substitution:result.getSubstitutionsList()){final BackendResult.Value value=switch(substitution.getValueCase()){case TEXT->new BackendResult.Text(substitution.getText());case INTEGER->new BackendResult.Integer(substitution.getInteger());case BOOLEAN->new BackendResult.Boolean(substitution.getBoolean());case WORLD_IDENTITY->new BackendResult.World(WorldIdentifier.create(substitution.getWorldIdentity().getNamespace(),substitution.getWorldIdentity().getValue()));case VALUE_NOT_SET->new BackendResult.Text("");}; substitutions.add(new BackendResult.Substitution(substitution.getKey(),value));} return new BackendResult(code,substitutions);}
    public void abortForRestart(){clearPending(BackendResult.Code.BACKEND_UNAVAILABLE);completeConfig(BackendResult.of(BackendResult.Code.BACKEND_UNAVAILABLE));if(snapshotHandler!=null)snapshotHandler.abortAll();}
    @Override public void close(){abortForRestart();if(snapshotHandler!=null)snapshotHandler.close();scheduler.shutdownNow();}
}
