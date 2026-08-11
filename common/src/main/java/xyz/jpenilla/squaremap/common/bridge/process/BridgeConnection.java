package xyz.jpenilla.squaremap.common.bridge.process;

/** Authenticated, accepted connection to a managed sidecar. */
public interface BridgeConnection extends AutoCloseable {
    /** Returns the immutable 16-byte protocol session identifier. */
    byte[] sessionId();

    /** Returns whether this connection has completed shutdown. */
    boolean isClosed();

    @Override
    void close();
}
