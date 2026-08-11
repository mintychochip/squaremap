package xyz.jpenilla.squaremap.common.bridge.process;

import java.util.Objects;
import java.util.function.Consumer;
import xyz.jpenilla.squaremap.bridge.v1.ControlRequest;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.BridgePolicyReplace;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher;
/** Authenticated, accepted connection to a managed sidecar. */
public interface BridgeConnection extends AutoCloseable {
    byte[] sessionId();
    boolean isClosed();
    BridgePublisher.PublishResult publish(BridgeEvent event);

    default BridgePublisher.PublishResult publishControl(final long correlationId, final ControlRequest request) {
        Objects.requireNonNull(request, "request");
        return this.publish(new BridgeEvent.Control(correlationId, Envelope.newBuilder().setControlRequest(request).build()));
    }
    default BridgePublisher.ControlDisposition cancelControl(final long correlationId) {
        return BridgePublisher.ControlDisposition.RECALLED;
    }
    default void applyPolicy(final BridgePolicyReplace policy) {}

    default void setResponseListener(final Consumer<Envelope> listener) {
        Objects.requireNonNull(listener, "listener");
    }
    default void setFailureListener(final Consumer<Throwable> listener) {
        Objects.requireNonNull(listener, "listener");
    }

    @Override
    void close();
}
