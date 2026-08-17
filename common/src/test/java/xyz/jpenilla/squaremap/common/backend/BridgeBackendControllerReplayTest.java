package xyz.jpenilla.squaremap.common.backend;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.google.protobuf.ByteString;
import java.time.Duration;
import java.util.Collection;
import java.util.Collections;
import java.util.List;
import java.util.Optional;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import net.minecraft.server.level.ServerLevel;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.bridge.v1.ControlKind;
import xyz.jpenilla.squaremap.bridge.v1.DirtyReplayItem;
import xyz.jpenilla.squaremap.bridge.v1.DirtyResyncComplete;
import xyz.jpenilla.squaremap.bridge.v1.DirtyResyncStatus;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.ResumeWatermark;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;
import xyz.jpenilla.squaremap.bridge.v1.WorldResyncRequired;
import xyz.jpenilla.squaremap.common.WorldManager;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher;
import xyz.jpenilla.squaremap.common.bridge.process.BridgeConnection;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;

final class BridgeBackendControllerReplayTest {
    private final ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();

    @AfterEach
    void closeScheduler() {
        this.scheduler.shutdownNow();
    }

    @Test
    void resumeWatermarkWithNoWorldsPublishesNoReplayRequest() {
        final RecordingConnection connection = new RecordingConnection();
        final BridgeBackendController controller = this.controller(connection);

        controller.dispatch(Envelope.newBuilder()
            .setSessionId(ByteString.copyFrom(connection.sessionId))
            .setResumeWatermark(ResumeWatermark.newBuilder()
                .setBridgeId(ByteString.copyFrom(connection.sessionId))
                .setSessionId(ByteString.copyFrom(connection.sessionId))
                .setConfigRevision(7L)
                .setLastDurableSequence(19L))
            .build());

        assertTrue(connection.transientEvents.isEmpty(), "no replay requests should be published for an empty world list");
        assertTrue(connection.replayListenerSet, "replay listener must be attached");
    }

    @Test
    void worldResyncRequiredWithoutWorldIsRejected() {
        final RecordingConnection connection = new RecordingConnection();
        final BridgeBackendController controller = this.controller(connection);

        assertThrows(IllegalStateException.class, () -> controller.dispatch(Envelope.newBuilder()
            .setSessionId(ByteString.copyFrom(connection.sessionId))
            .setWorldResyncRequired(WorldResyncRequired.newBuilder())
            .build()));
        assertTrue(connection.controlEvents.isEmpty(), "no control should be dispatched for a malformed resync");
    }

    @Test
    void unexpectedDirtyReplayItemIsRejected() {
        final RecordingConnection connection = new RecordingConnection();
        final BridgeBackendController controller = this.controller(connection);

        assertThrows(IllegalStateException.class, () -> controller.dispatch(Envelope.newBuilder()
            .setSessionId(ByteString.copyFrom(connection.sessionId))
            .setDirtyReplayItem(DirtyReplayItem.newBuilder()
                .setBridgeId(ByteString.copyFrom(connection.sessionId))
                .setSessionId(ByteString.copyFrom(connection.sessionId))
                .setConfigRevision(7L)
                .setReplayId(3L)
                .setItemIndex(0)
                .setWorld(WorldIdentity.newBuilder().setNamespace("minecraft").setValue("overworld").setEpoch(1L))
                .setCoordinate(xyz.jpenilla.squaremap.bridge.v1.ChunkCoordinate.newBuilder().setX(1).setZ(2))
                .setRevision(9L))
            .build()));
    }

    @Test
    void unexpectedResyncCompleteIsRejected() {
        final RecordingConnection connection = new RecordingConnection();
        final BridgeBackendController controller = this.controller(connection);

        assertThrows(IllegalStateException.class, () -> controller.dispatch(Envelope.newBuilder()
            .setSessionId(ByteString.copyFrom(connection.sessionId))
            .setDirtyResyncComplete(DirtyResyncComplete.newBuilder()
                .setBridgeId(ByteString.copyFrom(connection.sessionId))
                .setSessionId(ByteString.copyFrom(connection.sessionId))
                .setConfigRevision(7L)
                .setReplayId(3L)
                .setItemCount(0)
                .setStatus(DirtyResyncStatus.DIRTY_RESYNC_STATUS_COMPLETE))
            .build()));
    }

    private BridgeBackendController controller(final RecordingConnection connection) {
        final BridgeBackendController controller = new BridgeBackendController(
            connection,
            this.scheduler,
            Duration.ofSeconds(1),
            world -> 7L,
            () -> new EmptyWorldManager()
        );
        controller.attachForTest(connection);
        return controller;
    }

    private static final class EmptyWorldManager implements WorldManager {
        @Override
        public Collection<MapWorldInternal> worlds() {
            return Collections.emptyList();
        }

        @Override
        public Optional<MapWorldInternal> getWorldIfEnabled(final WorldIdentifier worldIdentifier) {
            return Optional.empty();
        }

        @Override
        public Optional<MapWorldInternal> getWorldIfEnabled(final ServerLevel level) {
            return Optional.empty();
        }
    }

    private static final class RecordingConnection implements BridgeConnection {
        private final byte[] sessionId = new byte[16];
        final List<BridgeEvent> transientEvents = new java.util.ArrayList<>();
        final List<BridgeEvent> controlEvents = new java.util.ArrayList<>();
        boolean replayListenerSet;

        @Override
        public byte[] sessionId() {
            return this.sessionId;
        }

        @Override
        public boolean isClosed() {
            return false;
        }

        @Override
        public BridgePublisher.PublishResult publish(final BridgeEvent event) {
            if (event instanceof BridgeEvent.Transient) {
                this.transientEvents.add(event);
            } else if (event instanceof BridgeEvent.Control) {
                this.controlEvents.add(event);
            }
            return BridgePublisher.PublishResult.ACCEPTED;
        }

        @Override
        public void rejectConfig(final long revision) {}

        @Override
        public void setReplayListener(final java.util.function.Consumer<Envelope> listener) {
            this.replayListenerSet = true;
        }

        @Override
        public void setReadyListener(final java.util.function.Consumer<Envelope> listener) {
        }

        @Override
        public void setResponseListener(final java.util.function.Consumer<Envelope> listener) {
        }

        @Override
        public void setFailureListener(final java.util.function.Consumer<Throwable> listener) {
        }

        @Override
        public void setAcknowledgementListener(final java.util.function.Consumer<BridgePublisher.Sent> listener) {
        }

        @Override
        public void close() {
        }
    }
}
