package xyz.jpenilla.squaremap.common.bridge.recording;

import java.io.BufferedOutputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.Objects;
import java.util.concurrent.atomic.AtomicLong;
import java.util.regex.Pattern;

/**
 * Opt-in, bounded, fail-closed recorder for already coalesced bridge frames.
 * The recorder never owns transport ordering or acknowledgements; callers invoke
 * {@link #record(Direction, byte[])} immediately at the transport boundary.
 */
public final class BridgeRecorder implements AutoCloseable {
    public enum Direction { OUTBOUND, INBOUND }
    public static final int MAGIC = 0x534D5243; // SMRC
    public static final int VERSION = 1;
    private static final Pattern SECRET = Pattern.compile("(?i)(token|secret|password|authorization)(\\s*[=:]\\s*)([^\\s,;]+)");

    private final DataOutputStream output;
    private final Path diagnosticsRoot;
    private final Path recording;
    private final long maxBytes;
    private final AtomicLong bytes = new AtomicLong();
    private long lastNanos;
    private volatile Throwable failure;
    private volatile boolean closed;

    public BridgeRecorder(final Path diagnosticsRoot, final Path recording, final long maxBytes) throws IOException {
        this.diagnosticsRoot = confinedRoot(diagnosticsRoot);
        this.recording = confined(this.diagnosticsRoot, recording);
        if (maxBytes < 32) throw new IllegalArgumentException("maxBytes must be at least 32");
        this.maxBytes = maxBytes;
        Files.createDirectories(this.recording.getParent());
        this.output = new DataOutputStream(new BufferedOutputStream(Files.newOutputStream(this.recording,
            StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE)));
        this.output.writeInt(MAGIC);
        this.output.writeInt(VERSION);
        this.output.writeLong(0L);
        this.bytes.set(16L);
    }

    public static BridgeRecorder disabled() {
        return new BridgeRecorder();
    }

    private BridgeRecorder() { this.output = null; this.diagnosticsRoot = null; this.recording = null; this.maxBytes = 0; this.closed = true; }

    public synchronized void record(final Direction direction, final byte[] frame) {
        Objects.requireNonNull(direction, "direction");
        Objects.requireNonNull(frame, "frame");
        if (output == null) return;
        ensureOpen();
        try {
            final byte[] safe = redact(frame);
            final long now = System.nanoTime();
            final long timestamp = Math.max(now, lastNanos);
            final long required = 1L + 8L + 4L + safe.length;
            if (required > maxBytes - bytes.get()) throw fail(new IOException("bridge recording bound exceeded"));
            output.writeByte(direction.ordinal());
            output.writeLong(timestamp);
            output.writeInt(safe.length);
            output.write(safe);
            bytes.addAndGet(required);
            lastNanos = timestamp;
        } catch (IOException e) {
            throw fail(e);
        }
    }

    public synchronized void flush() {
        if (output == null) return;
        ensureOpen();
        try { output.flush(); } catch (IOException e) { throw fail(e); }
    }

    public synchronized Throwable failure() { return failure; }
    public synchronized long bytesWritten() { return bytes.get(); }
    public Path path() { return recording; }
    public boolean enabled() { return output != null; }

    @Override public synchronized void close() {
        if (output == null || closed) return;
        try {
            output.flush();
            output.close();
            closed = true;
        } catch (IOException e) { throw fail(e); }
    }

    private void ensureOpen() { if (closed) throw new IllegalStateException("bridge recorder is closed", failure); }
    private RuntimeException fail(final Throwable cause) { failure = cause; closed = true; try { output.close(); } catch (Exception ignored) {} return new IllegalStateException("bridge recorder failed", cause); }

    private static byte[] redact(final byte[] frame) {
        final String text = new String(frame, StandardCharsets.UTF_8);
        if (!SECRET.matcher(text).find()) return frame.clone();
        return SECRET.matcher(text).replaceAll("$1$2<redacted>").getBytes(StandardCharsets.UTF_8);
    }
    private static Path confinedRoot(final Path root) throws IOException {
        Objects.requireNonNull(root, "diagnosticsRoot");
        Files.createDirectories(root);
        return root.toRealPath();
    }
    private static Path confined(final Path root, final Path path) throws IOException {
        Objects.requireNonNull(path, "recording");
        final Path absolute = path.isAbsolute() ? path.normalize() : root.resolve(path).normalize();
        if (!absolute.startsWith(root)) throw new IllegalArgumentException("recording path escapes diagnostics root");
        Path cursor = absolute;
        while (cursor != null && !Files.exists(cursor, LinkOption.NOFOLLOW_LINKS)) cursor = cursor.getParent();
        if (cursor != null && !cursor.toRealPath().startsWith(root)) throw new IllegalArgumentException("recording path escapes diagnostics root");
        return absolute;
    }
}
