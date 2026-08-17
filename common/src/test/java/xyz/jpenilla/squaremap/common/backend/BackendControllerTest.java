package xyz.jpenilla.squaremap.common.backend;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertTimeoutPreemptively;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.common.bridge.process.BackendMode;

final class BackendControllerTest {
    private final WorldIdentifier world = WorldIdentifier.create("minecraft", "overworld");
    private final ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();

    @AfterEach
    void closeScheduler() { this.scheduler.shutdownNow(); }

    @Test
    void rustRoutesOnlyBridgeAndPreservesTypedCodes() {
        final List<BackendController.BackendRequest> bridge = new ArrayList<>();
        final BackendController controller = new BackendController(request -> {
            bridge.add(request);
            return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.HEALTHY));
        });
        assertEquals(BackendResult.Code.HEALTHY, controller.fullRender(world).toCompletableFuture().join().code());
        assertEquals(BackendResult.Code.HEALTHY, controller.health().toCompletableFuture().join().code());
        assertEquals(2, bridge.size());
    }

    @Test
    void bridgeDispatchesCorrelatedTypedResultAndDiscardsLateOrWrongSession() {
        final FakeConnection connection = new FakeConnection();
        final BridgeBackendController bridge = new BridgeBackendController(connection, this.scheduler);
        final CompletableFuture<BackendResult> pending = bridge.execute(new BackendController.Health()).toCompletableFuture();
        assertEquals(1, bridge.pendingCount());
        bridge.dispatch(xyz.jpenilla.squaremap.bridge.v1.Envelope.newBuilder().setSessionId(com.google.protobuf.ByteString.copyFrom(new byte[16])).setCorrelationId(999)
            .setControlResult(xyz.jpenilla.squaremap.bridge.v1.ControlResult.newBuilder().setCode(xyz.jpenilla.squaremap.bridge.v1.BackendResultCode.BACKEND_RESULT_CODE_HEALTHY)).build());
        assertTrue(!pending.isDone());
        connection.listener.accept(xyz.jpenilla.squaremap.bridge.v1.Envelope.newBuilder().setSessionId(com.google.protobuf.ByteString.copyFrom(connection.sessionId())).setCorrelationId(connection.correlation)
            .setControlResult(xyz.jpenilla.squaremap.bridge.v1.ControlResult.newBuilder().setCode(xyz.jpenilla.squaremap.bridge.v1.BackendResultCode.BACKEND_RESULT_CODE_HEALTHY)).build());
        assertEquals(BackendResult.Code.HEALTHY, pending.join().code()); assertEquals(0, bridge.pendingCount());
    }

    @Test
    void epochIsCarriedAndTimeoutCancelsQueuedControl() {
        final FakeConnection connection = new FakeConnection();
        final BridgeBackendController bridge = new BridgeBackendController(connection, this.scheduler, Duration.ofMillis(25), ignored -> 7L);
        final CompletionStage<BackendResult> pending = bridge.execute(new BackendController.FullRender(this.world));
        assertEquals(7L, connection.control.getWorld().getEpoch());
        final BackendResult result = assertTimeoutPreemptively(Duration.ofSeconds(2), pending.toCompletableFuture()::join);
        assertEquals(BackendResult.Code.BACKEND_TIMEOUT, result.code()); assertTrue(connection.cancelled);
    }

    @Test
    void dispatchedTimeoutTerminatesSessionBeforeTimeoutResult() {
        final FakeConnection connection = new FakeConnection(); connection.cancelDisposition = xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher.ControlDisposition.DISPATCHED;
        final BridgeBackendController bridge = new BridgeBackendController(connection, this.scheduler, Duration.ofMillis(25), ignored -> 7L);
        final CompletionStage<BackendResult> timedOut = bridge.execute(new BackendController.FullRender(this.world));
        final BackendResult result = assertTimeoutPreemptively(Duration.ofSeconds(2), timedOut.toCompletableFuture()::join);
        assertEquals(BackendResult.Code.BACKEND_TIMEOUT, result.code()); assertTrue(connection.closed);
        assertTrue(bridge.execute(new BackendController.Health()).toCompletableFuture().join().code() == BackendResult.Code.BACKEND_UNAVAILABLE);
    }

    @Test
    void lateConfigRejectionDoesNotCompleteNewRevision() {
        final FakeConnection connection = new FakeConnection(); final BridgeBackendController bridge = new BridgeBackendController(connection, this.scheduler, Duration.ofMillis(25), ignored -> 0L);
        final CompletionStage<BackendResult> first = bridge.publishConfig(xyz.jpenilla.squaremap.bridge.v1.ConfigReplace.newBuilder().setRevision(1).build());
        assertEquals(BackendResult.Code.BACKEND_TIMEOUT, first.toCompletableFuture().join().code());
        final CompletionStage<BackendResult> second = bridge.publishConfig(xyz.jpenilla.squaremap.bridge.v1.ConfigReplace.newBuilder().setRevision(2).build());
        bridge.dispatch(xyz.jpenilla.squaremap.bridge.v1.Envelope.newBuilder().setSessionId(com.google.protobuf.ByteString.copyFrom(connection.sessionId())).setProtocolError(xyz.jpenilla.squaremap.bridge.v1.ProtocolError.newBuilder().setConfigRevision(1)).build());
        assertFalse(second.toCompletableFuture().isDone());
        bridge.dispatch(xyz.jpenilla.squaremap.bridge.v1.Envelope.newBuilder().setSessionId(com.google.protobuf.ByteString.copyFrom(connection.sessionId())).setBridgePolicyReplace(xyz.jpenilla.squaremap.bridge.v1.BridgePolicyReplace.newBuilder().setRevision(2)).build());
        assertEquals(BackendResult.Code.HEALTHY, second.toCompletableFuture().join().code());
    }

    @Test
    void mismatchedConfigPolicyRevisionIsDiscarded() {
        final FakeConnection connection = new FakeConnection(); final BridgeBackendController bridge = new BridgeBackendController(connection, this.scheduler);
        final CompletionStage<BackendResult> pending = bridge.publishConfig(xyz.jpenilla.squaremap.bridge.v1.ConfigReplace.newBuilder().setRevision(9).build());
        bridge.dispatch(xyz.jpenilla.squaremap.bridge.v1.Envelope.newBuilder().setSessionId(com.google.protobuf.ByteString.copyFrom(connection.sessionId())).setBridgePolicyReplace(xyz.jpenilla.squaremap.bridge.v1.BridgePolicyReplace.newBuilder().setRevision(8)).build());
        assertTrue(!pending.toCompletableFuture().isDone());
        bridge.dispatch(xyz.jpenilla.squaremap.bridge.v1.Envelope.newBuilder().setSessionId(com.google.protobuf.ByteString.copyFrom(connection.sessionId())).setBridgePolicyReplace(xyz.jpenilla.squaremap.bridge.v1.BridgePolicyReplace.newBuilder().setRevision(9)).build());
        assertEquals(BackendResult.Code.HEALTHY, pending.toCompletableFuture().join().code());
    }

    @Test
    void invalidConfigIsConsumedAndNotRepublishedAfterReconnect() {
        final FakeConnection connection = new FakeConnection();
        final BridgeBackendController bridge = new BridgeBackendController(connection, this.scheduler);
        final CompletionStage<BackendResult> pending = bridge.publishConfig(xyz.jpenilla.squaremap.bridge.v1.ConfigReplace.newBuilder().setRevision(7).build());
        bridge.dispatch(xyz.jpenilla.squaremap.bridge.v1.Envelope.newBuilder()
            .setSessionId(com.google.protobuf.ByteString.copyFrom(connection.sessionId()))
            .setProtocolError(xyz.jpenilla.squaremap.bridge.v1.ProtocolError.newBuilder()
                .setFatal(false)
                .setCode(xyz.jpenilla.squaremap.bridge.v1.ProtocolErrorCode.PROTOCOL_ERROR_CODE_INVALID_MESSAGE)
                .setConfigRevision(7))
            .build());
        assertEquals(BackendResult.Code.INVALID_CONFIG, pending.toCompletableFuture().join().code());
        assertTrue(connection.rejectedConfigRevisions.contains(7L));
        bridge.dispatch(xyz.jpenilla.squaremap.bridge.v1.Envelope.newBuilder()
            .setSessionId(com.google.protobuf.ByteString.copyFrom(connection.sessionId()))
            .setBridgePolicyReplace(xyz.jpenilla.squaremap.bridge.v1.BridgePolicyReplace.newBuilder().setRevision(8))
            .build());
        assertTrue(!connection.rejectedConfigRevisions.contains(8L));
    }


    @Test
    void restartAbortLeavesBridgeControllerReusable() {
        final FakeConnection connection = new FakeConnection(); final BridgeBackendController bridge = new BridgeBackendController(connection, this.scheduler); bridge.abortForRestart();
        final CompletionStage<BackendResult> pending = bridge.execute(new BackendController.Health()); assertTrue(connection.correlation > 0);
        connection.listener.accept(xyz.jpenilla.squaremap.bridge.v1.Envelope.newBuilder().setSessionId(com.google.protobuf.ByteString.copyFrom(connection.sessionId())).setCorrelationId(connection.correlation).setControlResult(xyz.jpenilla.squaremap.bridge.v1.ControlResult.newBuilder().setCode(xyz.jpenilla.squaremap.bridge.v1.BackendResultCode.BACKEND_RESULT_CODE_HEALTHY)).build());
        assertEquals(BackendResult.Code.HEALTHY, pending.toCompletableFuture().join().code());
    }
    @Test
    void bridgeReloadUsesJavaReloadPath() {
        final FakeConnection connection = new FakeConnection();
        final BridgeBackendController bridge = new BridgeBackendController(connection, this.scheduler);
        assertEquals(BackendResult.Code.INVALID_REQUEST, bridge.execute(new BackendController.Reload()).toCompletableFuture().join().code());
        assertEquals(0, connection.correlation);
    }

    @Test
    void reconnectFailsOldRequestsAndDoesNotRetainConfig() {
        final FakeConnection first = new FakeConnection(); final BridgeBackendController bridge = new BridgeBackendController(first, this.scheduler);
        final CompletionStage<BackendResult> request = bridge.execute(new BackendController.Health());
        final CompletionStage<BackendResult> config = bridge.publishConfig(xyz.jpenilla.squaremap.bridge.v1.ConfigReplace.newBuilder().setRevision(1).build());
        final FakeConnection second = new FakeConnection(); bridge.attachForTest(second);
        assertEquals(BackendResult.Code.BACKEND_UNAVAILABLE, request.toCompletableFuture().join().code()); assertEquals(BackendResult.Code.BACKEND_UNAVAILABLE, config.toCompletableFuture().join().code());
        final CompletionStage<BackendResult> replacement = bridge.execute(new BackendController.Health()); first.failureListener.accept(new IllegalStateException("late failure")); assertTrue(!replacement.toCompletableFuture().isDone());
        second.listener.accept(xyz.jpenilla.squaremap.bridge.v1.Envelope.newBuilder().setSessionId(com.google.protobuf.ByteString.copyFrom(second.sessionId())).setCorrelationId(second.correlation).setControlResult(xyz.jpenilla.squaremap.bridge.v1.ControlResult.newBuilder().setCode(xyz.jpenilla.squaremap.bridge.v1.BackendResultCode.BACKEND_RESULT_CODE_HEALTHY)).build());
        assertEquals(BackendResult.Code.HEALTHY, replacement.toCompletableFuture().join().code());
    }

    @Test
    void reconnectAdmissionIsNotClearedByAttachCleanup() {
        final FakeConnection first = new FakeConnection(); final BridgeBackendController bridge = new BridgeBackendController(first, this.scheduler);
        final FakeConnection second = new FakeConnection(); second.onControlPublish = () -> bridge.attachForTest(second);
        final CompletionStage<BackendResult> replacement = bridge.execute(new BackendController.Health());
    }

    private static final class FakeConnection implements xyz.jpenilla.squaremap.common.bridge.process.BridgeConnection {
        private final byte[] sessionId = new byte[16]; private java.util.function.Consumer<xyz.jpenilla.squaremap.bridge.v1.Envelope> listener = ignored -> {}; private java.util.function.Consumer<Throwable> failureListener = ignored -> {}; private xyz.jpenilla.squaremap.bridge.v1.ControlRequest control; private long correlation; private boolean cancelled; private boolean closed; private Runnable onControlPublish; private FakeConnection lastControlConnection; private xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher.ControlDisposition cancelDisposition = xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher.ControlDisposition.RECALLED; private final java.util.Set<Long> rejectedConfigRevisions = new java.util.HashSet<>();
        @Override public byte[] sessionId() { return this.sessionId; } @Override public boolean isClosed() { return this.closed; }
        @Override public xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher.PublishResult publish(xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent event) { if (event instanceof xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent.Control controlEvent) { this.correlation = controlEvent.correlationId(); this.control = controlEvent.payload().getControlRequest(); if (this.onControlPublish != null) { this.lastControlConnection = this; this.onControlPublish.run(); } } return xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher.PublishResult.ACCEPTED; }
        public void rejectConfig(final long revision) { this.rejectedConfigRevisions.add(revision); }
        @Override public xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher.ControlDisposition cancelControl(final long correlationId) { this.cancelled = this.correlation == correlationId; return this.cancelDisposition; }
        @Override public void setResponseListener(java.util.function.Consumer<xyz.jpenilla.squaremap.bridge.v1.Envelope> listener) { this.listener = listener; } @Override public void setFailureListener(java.util.function.Consumer<Throwable> listener) { this.failureListener = listener; } @Override public void close() { this.closed = true; this.failureListener.accept(new IllegalStateException("fake session terminated")); }
    }
}
