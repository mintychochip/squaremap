package xyz.jpenilla.squaremap.common.bridge.process;

import com.google.protobuf.ByteString;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.nio.channels.ServerSocketChannel;
import java.nio.channels.SocketChannel;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Base64;
import java.util.List;
import java.util.Objects;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.ThreadFactory;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.HelloAck;
import xyz.jpenilla.squaremap.bridge.v1.Shutdown;
import xyz.jpenilla.squaremap.bridge.v1.ShutdownReason;
import xyz.jpenilla.squaremap.common.bridge.protocol.FrameCodec;

/** Launches and authenticates one managed sidecar process. */
public final class SidecarSupervisor implements AutoCloseable {
    public static final int BOOTSTRAP_TOKEN_BYTES = 32;
    public static final int SESSION_ID_BYTES = 16;
    private enum LifecycleState {
        NEW,
        STARTING,
        READY,
        FAILED,
        CLOSED
    }

    public static final int MAX_STDERR_BYTES = 64 * 1024;

    private final Object lock = new Object();
    private final SecureRandom secureRandom;
    private final ExecutorService executor;
    private final ScheduledExecutorService scheduler;
    private final AtomicBoolean cleaned = new AtomicBoolean();
    private final byte[] stderr = new byte[MAX_STDERR_BYTES];
    private int stderrLength;
    private List<String> launchedCommand = List.of();
    private CompletableFuture<BridgeConnection> startFuture;
    private ServerSocketChannel listener;
    private SocketChannel socket;
    private Process process;
    private ManagedConnection connection;
    private LifecycleState lifecycleState = LifecycleState.NEW;
    private volatile boolean closed;
    private ScheduledFuture<?> timeoutTask;

    public SidecarSupervisor() {
        this(new SecureRandom());
    }

    SidecarSupervisor(final SecureRandom secureRandom) {
        this.secureRandom = Objects.requireNonNull(secureRandom, "secureRandom");
        final ThreadFactory factory = runnable -> {
            final Thread thread = new Thread(runnable, "squaremap-sidecar");
            thread.setDaemon(true);
            return thread;
        };
        this.executor = Executors.newCachedThreadPool(factory);
        this.scheduler = Executors.newSingleThreadScheduledExecutor(factory);
    }

    /** Starts exactly one sidecar; subsequent calls return the same stage. */
    public CompletionStage<BridgeConnection> start(final BridgeBootstrapConfig config) {
        Objects.requireNonNull(config, "config");
        synchronized (this.lock) {
            if (this.startFuture != null) {
                return this.startFuture;
            }
            this.startFuture = new CompletableFuture<>();
            if (this.lifecycleState == LifecycleState.CLOSED || this.lifecycleState == LifecycleState.FAILED) {
                this.startFuture.completeExceptionally(new IllegalStateException("supervisor is closed"));
                return this.startFuture;
            }
            if (config.backendMode() == BackendMode.JAVA) {
                final byte[] session = new byte[SESSION_ID_BYTES];
                this.connection = new ManagedConnection(null, null, session, config.shutdownGrace(), true);
                this.lifecycleState = LifecycleState.READY;
                this.startFuture.complete(this.connection);
                return this.startFuture;
            }
            this.lifecycleState = LifecycleState.STARTING;
            this.executor.execute(() -> this.launch(config));
            return this.startFuture;
        }
    }

    public boolean isClosed() {
        synchronized (this.lock) {
            return this.lifecycleState == LifecycleState.FAILED || this.lifecycleState == LifecycleState.CLOSED;
        }
    }

    /** Returns a bounded copy of child stderr captured so far. */
    public byte[] stderrSnapshot() {
        synchronized (this.lock) {
            final byte[] copy = new byte[this.stderrLength];
            System.arraycopy(this.stderr, 0, copy, 0, this.stderrLength);
            return copy;
        }
    }

    List<String> launchedCommandSnapshot() {
        synchronized (this.lock) {
            return this.launchedCommand;
        }
    }

    @Override
    public void close() {
        final ManagedConnection active;
        synchronized (this.lock) {
            if (this.lifecycleState == LifecycleState.CLOSED) {
                return;
            }
            this.lifecycleState = LifecycleState.CLOSED;
            this.closed = true;
            active = this.connection;
        }
        if (active != null) {
            active.close();
        } else {
            this.cleanup(false, BridgeBootstrapConfig.DEFAULT_SHUTDOWN_GRACE);
            final CompletableFuture<BridgeConnection> future = this.startFuture;
            if (future != null && !future.isDone()) {
                future.completeExceptionally(new IllegalStateException("supervisor closed"));
            }
        }
    }

    private void launch(final BridgeBootstrapConfig config) {
        final byte[] token = new byte[BOOTSTRAP_TOKEN_BYTES];
        try {
            final ServerSocketChannel openedListener = ServerSocketChannel.open();
            openedListener.bind(new InetSocketAddress(InetAddress.getLoopbackAddress(), 0));
            synchronized (this.lock) {
                if (this.lifecycleState != LifecycleState.STARTING || this.closed) {
                    closeQuietly(openedListener);
                    throw new IOException("supervisor closed");
                }
                this.listener = openedListener;
            }
            final InetSocketAddress address = (InetSocketAddress) openedListener.getLocalAddress();

            final List<String> command = new ArrayList<>(config.sidecarCommand().command());
            command.add("bridge");
            command.add("--connect");
            command.add(connectAddress(address));
            command.add("--plugin-version");
            command.add(config.pluginVersion());
            synchronized (this.lock) {
                this.launchedCommand = List.copyOf(command);
            }
            final ProcessBuilder builder = new ProcessBuilder(command);
            synchronized (this.lock) {
                if (this.lifecycleState != LifecycleState.STARTING || this.closed) {
                    throw new IOException("supervisor closed");
                }
                this.process = builder.start();
            }
            this.drain(this.process.getErrorStream(), true);
            this.drain(this.process.getInputStream(), false);
            this.secureRandom.nextBytes(token);
            this.writeToken(token);
            this.timeoutTask = this.scheduler.schedule(
                () -> this.failStart(new IOException("sidecar readiness timed out")),
                config.readinessTimeout().toNanos(),
                TimeUnit.NANOSECONDS
            );
            final SocketChannel accepted = this.listener.accept();
            synchronized (this.lock) {
                if (this.lifecycleState != LifecycleState.STARTING || this.closed) {
                    closeQuietly(accepted);
                    throw new IOException("supervisor closed");
                }
                this.socket = accepted;
            }
            this.listener.close();
            this.listener = null;
            final Envelope hello = FrameCodec.read(accepted);
            final byte[] sessionId = hello.getSessionId().toByteArray();
            final boolean validMajor = hello.getProtocolMajor() == 1;
            final boolean validSession = sessionId.length == SESSION_ID_BYTES;
            final boolean validPayload = hello.getPayloadCase() == Envelope.PayloadCase.HELLO;
            final boolean validToken = validPayload && MessageDigest.isEqual(
                token,
                hello.getHello().getBootstrapToken().toByteArray()
            );
            final boolean acceptedHandshake = validMajor && validSession && validToken;
            this.writeHelloAck(accepted, sessionId, hello.getSequence(), acceptedHandshake,
                acceptedHandshake ? "" : rejectionReason(validMajor, validSession, validPayload, validToken));
            if (!acceptedHandshake) {
                throw new SecurityException("sidecar handshake rejected");
            }
            final ManagedConnection managed = new ManagedConnection(
                accepted,
                this.process,
                sessionId,
                config.shutdownGrace(),
                false
            );
            synchronized (this.lock) {
                if (this.lifecycleState != LifecycleState.STARTING || this.closed) {
                    closeQuietly(accepted);
                    throw new IOException("supervisor closed");
                }
                this.connection = managed;
                this.lifecycleState = LifecycleState.READY;
                this.startFuture.complete(managed);
            }
            final ScheduledFuture<?> timeout = this.timeoutTask;
            if (timeout != null) {
                timeout.cancel(false);
            }
        } catch (final Throwable failure) {
            this.failStart(failure);
        } finally {
            java.util.Arrays.fill(token, (byte) 0);
        }
    }

    private void writeToken(final byte[] token) throws IOException {
        try (OutputStream output = this.process.getOutputStream()) {
            output.write(Base64.getEncoder().encode(token));
            output.write('\n');
            output.flush();
        }
    }

    private void writeHelloAck(
        final SocketChannel channel,
        final byte[] sessionId,
        final long sequence,
        final boolean accepted,
        final String reason
    ) throws IOException {
        final Envelope ack = Envelope.newBuilder()
            .setProtocolMajor(1)
            .setProtocolMinor(0)
            .setSessionId(ByteString.copyFrom(sessionId))
            .setSequence(sequence + 1)
            .setHelloAck(HelloAck.newBuilder()
                .setProtocolMajor(1)
                .setProtocolMinor(0)
                .setAccepted(accepted)
                .setRejectionReason(reason))
            .build();
        FrameCodec.write(channel, ack);
    }

    private void failStart(final Throwable failure) {
        synchronized (this.lock) {
            if (this.lifecycleState != LifecycleState.STARTING) {
                return;
            }
            this.lifecycleState = LifecycleState.FAILED;
            this.closed = true;
        }
        final ScheduledFuture<?> timeout = this.timeoutTask;
        if (timeout != null) {
            timeout.cancel(false);
        }
        this.cleanup(false, Duration.ZERO);
        final CompletableFuture<BridgeConnection> future = this.startFuture;
        if (future != null && !future.isDone()) {
            future.completeExceptionally(failure);
        }
    }

    private void cleanup(final boolean graceful, final Duration grace) {
        if (!this.cleaned.compareAndSet(false, true)) {
            return;
        }
        closeQuietly(this.listener);
        closeQuietly(this.socket);
        final Process child;
        synchronized (this.lock) {
            child = this.process;
        }
        if (child != null) {
            closeQuietly(child.getOutputStream());
            if (child.isAlive()) {
                if (graceful) {
                    child.destroy();
                    waitFor(child, grace);
                }
                if (child.isAlive()) {
                    child.destroyForcibly();
                    waitFor(child, Duration.ofSeconds(1));
                }
            }
            closeQuietly(child.getInputStream());
            closeQuietly(child.getErrorStream());
        }
    }

    private void closeAccepted(final ManagedConnection accepted) {
        synchronized (this.lock) {
            if (!accepted.closed.compareAndSet(false, true)) {
                return;
            }
            this.lifecycleState = LifecycleState.CLOSED;
            this.closed = true;
        }
        try {
            final Envelope shutdown = Envelope.newBuilder()
                .setProtocolMajor(1)
                .setProtocolMinor(0)
                .setSessionId(ByteString.copyFrom(accepted.sessionId))
                .setSequence(2)
                .setShutdown(Shutdown.newBuilder().setReason(ShutdownReason.SHUTDOWN_REASON_REQUESTED))
                .build();
            if (accepted.socket != null && accepted.socket.isOpen()) {
                FrameCodec.write(accepted.socket, shutdown);
            }
        } catch (final IOException ignored) {
            // Forced process termination below is the fallback when the bridge is unavailable.
        } finally {
            closeQuietly(accepted.socket);
            this.cleanup(true, accepted.shutdownGrace);
        }
    }

    private void drain(final InputStream stream, final boolean capture) {
        this.executor.execute(() -> {
            try (InputStream input = stream) {
                final byte[] buffer = new byte[4096];
                int count;
                while ((count = input.read(buffer)) >= 0) {
                    if (capture && count > 0) {
                        synchronized (this.lock) {
                            final int retained = Math.min(count, MAX_STDERR_BYTES);
                            final int offset = count - retained;
                            if (retained >= MAX_STDERR_BYTES) {
                                System.arraycopy(buffer, offset, this.stderr, 0, MAX_STDERR_BYTES);
                                this.stderrLength = MAX_STDERR_BYTES;
                            } else {
                                final int shift = Math.max(0, this.stderrLength + retained - MAX_STDERR_BYTES);
                                if (shift > 0) {
                                    System.arraycopy(this.stderr, shift, this.stderr, 0, this.stderrLength - shift);
                                    this.stderrLength -= shift;
                                }
                                System.arraycopy(buffer, offset, this.stderr, this.stderrLength, retained);
                                this.stderrLength += retained;
                            }
                        }
                    }
                }
            } catch (final IOException ignored) {
                // Stream closure is part of normal supervisor shutdown.
            }
        });
    }

    private static String connectAddress(final InetSocketAddress address) {
        final String host = address.getAddress().getHostAddress();
        return host.indexOf(':') >= 0 ? "[" + host + "]:" + address.getPort() : host + ":" + address.getPort();
    }

    private static String rejectionReason(
        final boolean validMajor,
        final boolean validSession,
        final boolean validPayload,
        final boolean validToken
    ) {
        if (!validMajor) {
            return "unsupported protocol major";
        }
        if (!validSession) {
            return "invalid session id";
        }
        if (!validPayload) {
            return "expected hello";
        }
        if (!validToken) {
            return "invalid bootstrap token";
        }
        return "handshake rejected";
    }

    private static void waitFor(final Process process, final Duration duration) {
        try {
            process.waitFor(Math.max(1L, duration.toMillis()), TimeUnit.MILLISECONDS);
        } catch (final InterruptedException interrupted) {
            Thread.currentThread().interrupt();
        }
    }

    private static void closeQuietly(final AutoCloseable closeable) {
        if (closeable == null) {
            return;
        }
        try {
            closeable.close();
        } catch (final Exception ignored) {
            // Best-effort cleanup.
        }
    }

    private final class ManagedConnection implements BridgeConnection {
        private final SocketChannel socket;
        private final Process process;
        private final byte[] sessionId;
        private final AtomicBoolean closed = new AtomicBoolean();
        private final Duration shutdownGrace;
        private final boolean noProcess;


        private ManagedConnection(
            final SocketChannel socket,
            final Process process,
            final byte[] sessionId,
            final Duration shutdownGrace,
            final boolean noProcess
        ) {
            this.socket = socket;
            this.process = process;
            this.sessionId = sessionId.clone();
            this.shutdownGrace = shutdownGrace;
            this.noProcess = noProcess;
        }

        @Override
        public byte[] sessionId() {
            return this.sessionId.clone();
        }
        @Override
        public boolean isClosed() {
            return this.closed.get();
        }


        @Override
        public void close() {
            if (this.noProcess) {
                this.closed.set(true);
                SidecarSupervisor.this.cleanup(false, Duration.ZERO);
                return;
            }
            SidecarSupervisor.this.closeAccepted(this);
        }
    }
}
