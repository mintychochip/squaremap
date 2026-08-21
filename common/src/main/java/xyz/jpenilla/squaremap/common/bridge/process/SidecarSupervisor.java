package xyz.jpenilla.squaremap.common.bridge.process;

import com.google.inject.Singleton;

import com.google.protobuf.ByteString;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.nio.channels.ServerSocketChannel;
import java.nio.file.Path;
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
import java.util.function.Consumer;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.HelloAck;
import xyz.jpenilla.squaremap.bridge.v1.Shutdown;
import xyz.jpenilla.squaremap.bridge.v1.ShutdownReason;
import xyz.jpenilla.squaremap.bridge.v1.BridgePolicyReplace;
import xyz.jpenilla.squaremap.common.bridge.protocol.FrameLimits;
import xyz.jpenilla.squaremap.common.bridge.protocol.FrameCodec;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgePublisher;

/** Launches and authenticates one managed sidecar process. */
@Singleton
public final class SidecarSupervisor implements AutoCloseable {
    public static final int PROTOCOL_MAJOR = 1;
    public static final int PROTOCOL_MINOR = 0;
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
    private ScheduledFuture<?> restartTask;
    private final java.util.concurrent.CopyOnWriteArrayList<Consumer<BridgeConnection>> reconnectListeners = new java.util.concurrent.CopyOnWriteArrayList<>();
    private byte[] durableBridgeId;
    private BridgeBootstrapConfig restartConfig;
    private int restartStarts;
    private long healthySinceNanos;
    private long restartWindowStartNanos;

    static final Duration[] RESTART_DELAYS = {
        Duration.ofSeconds(1), Duration.ofSeconds(2), Duration.ofSeconds(4),
        Duration.ofSeconds(8), Duration.ofSeconds(16)
    };
    static final int MAX_RESTARTS_PER_WINDOW = 5;
    static final Duration RESTART_WINDOW = Duration.ofMinutes(10);
    static final Duration HEALTHY_RESET = Duration.ofMinutes(10);
    public SidecarSupervisor() {
        this(new SecureRandom(), null);
    }
    SidecarSupervisor(final SecureRandom secureRandom) {
        this(secureRandom, null);
    }
    public SidecarSupervisor(final SecureRandom secureRandom, final java.nio.file.Path dataDirectory) {
        this.secureRandom = Objects.requireNonNull(secureRandom, "secureRandom");
        this.durableBridgeId = loadOrCreateBridgeId(this.secureRandom, dataDirectory);
        final ThreadFactory factory = runnable -> {
            final Thread thread = new Thread(runnable, "squaremap-sidecar");
            thread.setDaemon(true);
            return thread;
        };
        this.executor = Executors.newCachedThreadPool(factory);
        this.scheduler = Executors.newSingleThreadScheduledExecutor(factory);
    }
    public void setBridgeIdentityDirectory(final java.nio.file.Path dataDirectory) {
        synchronized (this.lock) {
            this.durableBridgeId = loadOrCreateBridgeId(this.secureRandom, dataDirectory);
        }
    }
    private static byte[] loadOrCreateBridgeId(final SecureRandom random, final java.nio.file.Path dataDirectory) {
        final byte[] identity = new byte[SESSION_ID_BYTES];
        if (dataDirectory != null) {
            final java.nio.file.Path path = dataDirectory.resolve("bridge-identity.bin");
            try {
                if (java.nio.file.Files.isRegularFile(path)) {
                    final byte[] stored = java.nio.file.Files.readAllBytes(path);
                    if (stored.length != SESSION_ID_BYTES || allZero(stored)) {
                        throw new IllegalStateException("persisted bridge identity is invalid");
                    }
                    return stored;
                }
                random.nextBytes(identity);
                if (allZero(identity)) identity[0] = 1;
                java.nio.file.Files.createDirectories(dataDirectory);
                final java.nio.file.Path temporary = path.resolveSibling(path.getFileName() + ".tmp");
                java.nio.file.Files.write(temporary, identity, java.nio.file.StandardOpenOption.CREATE, java.nio.file.StandardOpenOption.TRUNCATE_EXISTING, java.nio.file.StandardOpenOption.WRITE);
                try {
                    java.nio.file.Files.move(temporary, path, java.nio.file.StandardCopyOption.ATOMIC_MOVE, java.nio.file.StandardCopyOption.REPLACE_EXISTING);
                } catch (java.nio.file.AtomicMoveNotSupportedException unsupported) {
                    java.nio.file.Files.move(temporary, path, java.nio.file.StandardCopyOption.REPLACE_EXISTING);
                }
                return identity;
            } catch (java.io.IOException error) {
                throw new IllegalStateException("could not load or persist bridge identity", error);
            }
        }
        random.nextBytes(identity);
        if (allZero(identity)) identity[0] = 1;
        return identity;
    }
    private static boolean allZero(final byte[] bytes) {
        for (final byte value : bytes) {
            if (value != 0) return false;
        }
        return true;
    }
    BridgeConnection startDetachedForTests() {
        synchronized (this.lock) {
            final byte[] session = new byte[SESSION_ID_BYTES];
            this.connection = new ManagedConnection(null, null, session, Duration.ofMillis(100), true, 1L);
            this.lifecycleState = LifecycleState.READY;
            this.startFuture = CompletableFuture.completedFuture(this.connection);
            return this.connection;
        }
    }

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
            this.restartConfig = config;
            this.lifecycleState = LifecycleState.STARTING;
            this.executor.execute(() -> this.launch(config));
            return this.startFuture;
        }
    }
    public void setReconnectListener(final Consumer<BridgeConnection> listener) {
        this.reconnectListeners.add(Objects.requireNonNull(listener, "listener"));
    }
    public BridgePublisher.PublishResult publish(final BridgeEvent event) {
        synchronized (this.lock) {
            if (this.connection == null || this.closed || this.lifecycleState == LifecycleState.FAILED) return BridgePublisher.PublishResult.COALESCED;
            try {
                return this.connection.publish(event);
            } catch (final IllegalStateException failure) {
                return BridgePublisher.PublishResult.COALESCED;
            }
        }
    }

    public boolean isClosed() {
        synchronized (this.lock) {
            return this.lifecycleState == LifecycleState.FAILED || this.lifecycleState == LifecycleState.CLOSED;
        }
    }
    public BridgeConnection currentConnection() {
        synchronized (this.lock) {
            return this.connection;
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

    long currentProcessPidForTests() {
        synchronized (this.lock) {
            return this.process == null ? -1L : this.process.pid();
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
            final ScheduledFuture<?> restart = this.restartTask;
            if (restart != null) {
                restart.cancel(false);
                this.restartTask = null;
            }
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
            if (config.rustOutputRoot() == null || !config.rustOutputRoot().isAbsolute()) {
                throw new IllegalArgumentException("Rust output root must be configured as an absolute path");
            }
            final ProcessBuilder builder = new ProcessBuilder(command);
            builder.environment().put("SQUAREMAP_OUTPUT_ROOT", config.rustOutputRoot().toString());
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
            final long helloSequence = hello.getSequence();
            if (helloSequence <= 0L || helloSequence > Long.MAX_VALUE - 2L) {
                throw new SecurityException("invalid handshake sequence");
            }
            final byte[] sessionId = hello.getSessionId().toByteArray();
            final boolean validMajor = hello.getProtocolMajor() == PROTOCOL_MAJOR;
            final int negotiatedMinor = Math.min(hello.getProtocolMinor(), PROTOCOL_MINOR);
            final boolean validMinor = hello.getProtocolMinor() <= PROTOCOL_MINOR;
            final boolean validSession = sessionId.length == SESSION_ID_BYTES;
            final boolean validPayload = hello.getPayloadCase() == Envelope.PayloadCase.HELLO;
            final boolean validToken = validPayload && MessageDigest.isEqual(
                token,
                hello.getHello().getBootstrapToken().toByteArray()
            );
            final boolean acceptedHandshake = validMajor && validMinor && validSession && validToken;
            this.writeHelloAck(accepted, sessionId, helloSequence, acceptedHandshake, negotiatedMinor,
                acceptedHandshake ? "" : rejectionReason(validMajor, validMinor, validSession, validPayload, validToken));
            if (!acceptedHandshake) {
                throw new SecurityException("sidecar handshake rejected");
            }
            final ManagedConnection managed = new ManagedConnection(
                accepted,
                this.process,
                sessionId,
                config.shutdownGrace(),
                false,
                Math.addExact(helloSequence, 2L)
            );
            synchronized (this.lock) {
                if (this.lifecycleState != LifecycleState.STARTING || this.closed) {
                    closeQuietly(accepted);
                    throw new IOException("supervisor closed");
                }
                this.connection = managed;
                this.lifecycleState = LifecycleState.READY;
                this.healthySinceNanos = 0L;
                managed.publishBridgeIdentity(this.durableBridgeId);
                this.healthySinceNanos = System.nanoTime();
                final ScheduledFuture<?> timeout = this.timeoutTask;
                if (timeout != null) {
                    timeout.cancel(false);
                }
                this.reconnectListeners.forEach(listener -> listener.accept(managed));
                this.executor.execute(managed::readFrames);
                if (this.startFuture != null && !this.startFuture.isDone()) {
                    this.startFuture.complete(managed);
                }
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

    private static String currentTargetTriple() {
        final String os = System.getProperty("os.name", "").toLowerCase(java.util.Locale.ROOT);
        final String arch = System.getProperty("os.arch", "").toLowerCase(java.util.Locale.ROOT);
        final String normalizedArch = switch (arch) {
            case "amd64", "x86_64" -> "x86_64";
            case "aarch64", "arm64" -> "aarch64";
            default -> throw new IllegalArgumentException("unsupported Rust backend architecture: " + arch);
        };
        if (os.contains("win")) return normalizedArch + "-pc-windows-msvc";
        if (os.contains("mac") || os.contains("darwin")) return normalizedArch + "-apple-darwin";
        if (os.contains("linux")) return normalizedArch + "-unknown-linux-gnu";
        throw new IllegalArgumentException("unsupported Rust backend operating system: " + os);
    }
    private void writeHelloAck(
        final SocketChannel channel,
        final byte[] sessionId,
        final long sequence,
        final boolean accepted,
        final int negotiatedMinor,
        final String reason
    ) throws IOException {
        final Envelope ack = Envelope.newBuilder()
            .setProtocolMajor(PROTOCOL_MAJOR)
            .setProtocolMinor(negotiatedMinor)
            .setSessionId(ByteString.copyFrom(sessionId))
            .setSequence(sequence + 1)
            .setHelloAck(HelloAck.newBuilder()
                .setProtocolMajor(PROTOCOL_MAJOR)
                .setProtocolMinor(negotiatedMinor)
                .setAccepted(accepted)
                .setRejectionReason(reason))
            .build();
        FrameCodec.write(channel, ack);
    }

    private void failStart(final Throwable failure) {
        final ManagedConnection active;
        final boolean authenticatedFailure;
        final boolean terminal;
        synchronized (this.lock) {
            if (this.lifecycleState != LifecycleState.STARTING && this.lifecycleState != LifecycleState.READY) return;
            authenticatedFailure = this.connection != null && this.healthySinceNanos != 0L;
            active = this.connection;
            this.connection = null;
            terminal = !authenticatedFailure || this.closed || this.restartConfig == null || !this.scheduleRestartLocked();
            if (terminal) {
                this.lifecycleState = LifecycleState.FAILED;
                this.closed = true;
            } else {
                this.lifecycleState = LifecycleState.STARTING;
            }
            if (active != null) active.closed.set(true);
        }
        final ScheduledFuture<?> timeout = this.timeoutTask;
        if (timeout != null) timeout.cancel(false);
        if (active != null) active.publisher.close();
        this.cleanup(false, Duration.ZERO);
        if (terminal) {
            final CompletableFuture<BridgeConnection> future = this.startFuture;
            if (future != null && !future.isDone()) future.completeExceptionally(failure);
        }
    }

    private boolean scheduleRestartLocked() {
        final long now = System.nanoTime();
        if (this.restartWindowStartNanos == 0L || now - this.restartWindowStartNanos >= RESTART_WINDOW.toNanos()) {
            this.restartWindowStartNanos = now;
            this.restartStarts = 0;
        }
        if (this.restartStarts >= MAX_RESTARTS_PER_WINDOW) return false;
        final int attempt = this.restartStarts++;
        final Duration delay = RESTART_DELAYS[Math.min(attempt, RESTART_DELAYS.length - 1)];
        final BridgeBootstrapConfig config = this.restartConfig;
        this.restartTask = this.scheduler.schedule(() -> {
            synchronized (this.lock) {
                if (this.closed || this.lifecycleState != LifecycleState.STARTING) return;
                this.cleaned.set(false);
            }
            this.executor.execute(() -> this.launch(config));
        }, delay.toNanos(), TimeUnit.NANOSECONDS);
        return true;
    }
    private void cleanup(final boolean graceful, final Duration grace) {
        if (!this.cleaned.compareAndSet(false, true)) return;
        closeQuietly(this.listener);
        closeQuietly(this.socket);
        final Process child;
        synchronized (this.lock) {
            child = this.process;
            this.listener = null;
            this.socket = null;
            this.process = null;
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
        if (this.lifecycleState == LifecycleState.CLOSED || this.lifecycleState == LifecycleState.FAILED) {
            this.scheduler.shutdownNow();
            this.executor.shutdownNow();
        }
    }
    private void closeAccepted(final ManagedConnection accepted) {
        synchronized (this.lock) {
            if (!accepted.closed.compareAndSet(false, true)) return;
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
            if (accepted.socket != null && accepted.socket.isOpen()) FrameCodec.write(accepted.socket, shutdown);
        } catch (final IOException ignored) {
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
        final boolean validMinor,
        final boolean validSession,
        final boolean validPayload,
        final boolean validToken
    ) {
        if (!validMajor) {
            return "unsupported protocol major";
        }
        if (!validMinor) {
            return "unsupported protocol minor";
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
        private final Object failureLock = new Object();
        private Throwable capturedFailure;
        private Consumer<Throwable> failureListener = ignored -> {};
        private boolean failureDelivered;
        private final Duration shutdownGrace;
        private final boolean noProcess;
        private final BridgePublisher publisher;
        private final InboundSequenceTracker inboundSequences;
        private volatile FrameLimits frameLimits = FrameLimits.DEFAULT;
        private volatile Consumer<Envelope> replayListener = ignored -> {};
        private volatile Consumer<Envelope> readyListener = ignored -> {};
        private volatile Consumer<Envelope> responseListener = ignored -> {};
        private volatile Consumer<Envelope> snapshotRequestListener = ignored -> {};
        private ManagedConnection(final SocketChannel socket, final Process process, final byte[] sessionId,
                                  final Duration shutdownGrace, final boolean noProcess, final long inboundSequence) {
            this.socket = socket;
            this.process = process;
            this.sessionId = sessionId.clone();
            this.shutdownGrace = shutdownGrace;
            this.noProcess = noProcess;
            this.inboundSequences = new InboundSequenceTracker(inboundSequence);
            this.publisher = new BridgePublisher(this.sessionId, sent -> {
                if (this.socket != null) FrameCodec.write(this.socket, sent.envelope(), this.frameLimits);
            });
            this.publisher.setFailureListener(failure -> {
                this.signalFailure(failure);
                SidecarSupervisor.this.failStart(failure);
            });
        }

        @Override public byte[] sessionId() { return this.sessionId.clone(); }
        @Override public boolean isClosed() { return this.closed.get(); }
        @Override public BridgePublisher.PublishResult publish(final BridgeEvent event) { return this.publisher.publish(event); }
        @Override public BridgePublisher.ControlDisposition cancelControl(final long correlationId) { return this.publisher.cancelControl(correlationId); }
        @Override public void rejectConfig(final long revision) { this.publisher.rejectConfig(revision); }
        @Override public void applyPolicy(final BridgePolicyReplace policy) {
            this.frameLimits = FrameLimits.fromPeerPolicy(policy);
            this.publisher.applyPolicy(policy);
        }
        @Override public void setReadyListener(final Consumer<Envelope> listener) {
            this.readyListener = java.util.Objects.requireNonNull(listener, "listener");
        }
        @Override public void setResponseListener(final Consumer<Envelope> listener) {
            this.responseListener = java.util.Objects.requireNonNull(listener, "listener");
        }
        @Override public void setReplayListener(final Consumer<Envelope> listener) {
            this.replayListener = java.util.Objects.requireNonNull(listener, "listener");
        }
        @Override public void setSnapshotRequestListener(final Consumer<Envelope> listener) {
            this.snapshotRequestListener = java.util.Objects.requireNonNull(listener, "listener");
        }
        @Override public void setFailureListener(final Consumer<Throwable> listener) {
            Objects.requireNonNull(listener, "listener");
            final Throwable failure;
            synchronized (this.failureLock) {
                this.failureListener = listener;
                if (this.capturedFailure == null || this.failureDelivered) {
                    return;
                }
                this.failureDelivered = true;
                failure = this.capturedFailure;
            }
            listener.accept(failure);
        }
        @Override public void setAcknowledgementListener(final Consumer<BridgePublisher.Sent> listener) {
            this.publisher.setAcknowledgementListener(listener);
        }

        private void readFrames() {
            if (this.socket == null) return;
            try {
                while (!this.closed.get()) {
                    final Envelope envelope = FrameCodec.read(this.socket, this.frameLimits);
                    if (!java.security.MessageDigest.isEqual(this.sessionId, envelope.getSessionId().toByteArray())) {
                        throw new SecurityException("bridge envelope session mismatch");
                    }
                    if (!this.inboundSequences.accept(envelope.getSequence())) {
                        throw new SecurityException("bridge envelope sequence mismatch");
                    }
                    if (envelope.hasResumeWatermark() || envelope.hasDirtyReplayRequest()
                        || envelope.hasDirtyReplayItem() || envelope.hasDirtyResyncComplete()
                        || envelope.hasWorldResyncRequired()) this.replayListener.accept(envelope);
                    else if (envelope.hasReady()) this.readyListener.accept(envelope);
                    else if (envelope.hasAck()) this.publisher.acknowledge(envelope);
                    else if (envelope.hasProtocolError() && envelope.getProtocolError().getFatal()) throw new IOException("fatal bridge protocol error");
                    else if (envelope.hasChunkSnapshotRequest() || envelope.hasWorldEnumerationRequest()) this.snapshotRequestListener.accept(envelope);
                    else this.responseListener.accept(envelope);
                }
            } catch (final Exception failure) {
                if (!this.closed.get()) {
                    this.signalFailure(failure);
                    SidecarSupervisor.this.failStart(failure);
                }
            }
        }

        private void signalFailure(final Throwable failure) {
            final Consumer<Throwable> listener;
            synchronized (this.failureLock) {
                if (this.capturedFailure != null) {
                    return;
                }
                this.capturedFailure = Objects.requireNonNull(failure, "failure");
                if (this.failureDelivered) {
                    return;
                }
                this.failureDelivered = true;
                listener = this.failureListener;
            }
            listener.accept(failure);
        }

        @Override public void close() {
            this.signalFailure(new IOException("bridge connection closed"));
            this.publisher.close();
            if (this.noProcess) {
                this.closed.set(true);
                SidecarSupervisor.this.cleanup(false, Duration.ZERO);
                return;
            }
            SidecarSupervisor.this.closeAccepted(this);
        }
    }
}
