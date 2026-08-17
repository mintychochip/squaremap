package xyz.jpenilla.squaremap.common.bridge.verification;

import java.time.Duration;
import java.util.Objects;
import java.util.function.Supplier;

/** Requires two complete, identical observations separated by a quiet interval. */
public final class QuiescenceProbe {
    private final Supplier<QuiescenceSnapshot> observations;

    public QuiescenceProbe(final Supplier<QuiescenceSnapshot> observations) {
        this.observations = Objects.requireNonNull(observations, "observations");
    }

    public QuiescenceSnapshot await(final Duration timeout, final Duration quietInterval) {
        Objects.requireNonNull(timeout, "timeout");
        Objects.requireNonNull(quietInterval, "quietInterval");
        final long timeoutNanos = safeNanos(timeout);
        final long quietNanos = safeNanos(quietInterval);
        final long start = System.nanoTime();
        QuiescenceSnapshot previous = null;
        while (elapsedNanos(start) <= timeoutNanos) {
            if (Thread.currentThread().isInterrupted()) {
                throw new IllegalStateException("quiescence wait interrupted");
            }
            final QuiescenceSnapshot current = Objects.requireNonNull(this.observations.get(), "observation");
            if (!current.isComplete()) {
                throw new IllegalStateException(current.failure().isEmpty() ? "quiescence observation incomplete" : current.failure());
            }
            if (previous != null && previous.equals(current) && elapsedNanos(start) <= timeoutNanos) return current;
            previous = current;
            if (quietNanos > 0L) {
                try {
                    final long remaining = timeoutNanos - elapsedNanos(start);
                    if (remaining <= 0L) break;
                    final long sleepNanos = Math.min(quietNanos, remaining);
                    Thread.sleep(java.util.concurrent.TimeUnit.NANOSECONDS.toMillis(sleepNanos),
                        (int) (sleepNanos % 1_000_000L));
                } catch (final InterruptedException interrupted) {
                    Thread.currentThread().interrupt();
                    throw new IllegalStateException("quiescence wait interrupted", interrupted);
                }
            }
        }
        throw new IllegalStateException("quiescence timeout");
    }

    private static long safeNanos(final Duration duration) {
        if (duration.isNegative()) throw new IllegalArgumentException("durations must not be negative");
        try {
            return duration.toNanos();
        } catch (final ArithmeticException overflow) {
            return Long.MAX_VALUE;
        }
    }
    private static long elapsedNanos(final long start) {
        final long elapsed = System.nanoTime() - start;
        return elapsed < 0L ? Long.MAX_VALUE : elapsed;
    }
}
