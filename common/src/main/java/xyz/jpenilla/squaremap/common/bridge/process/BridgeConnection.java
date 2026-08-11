package xyz.jpenilla.squaremap.common.bridge.process;

import xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher;

/** Authenticated, accepted connection to a managed sidecar. */
public interface BridgeConnection extends AutoCloseable {
    byte[] sessionId();
    boolean isClosed();
    BridgePublisher.PublishResult publish(BridgeEvent event);
    @Override
    void close();
}
