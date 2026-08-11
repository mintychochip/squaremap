package xyz.jpenilla.squaremap.common.bridge.state;

import com.google.inject.Singleton;
import java.util.concurrent.atomic.AtomicLong;

/** Issues monotonic replacement revisions shared by all bridge exporters. */
@Singleton
public final class BridgeRevisionClock {
    private final AtomicLong revision = new AtomicLong();

    public long next() {
        return this.revision.incrementAndGet();
    }
}
