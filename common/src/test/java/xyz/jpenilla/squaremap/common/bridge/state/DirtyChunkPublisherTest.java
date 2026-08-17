package xyz.jpenilla.squaremap.common.bridge.state;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicLong;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.common.data.ChunkCoordinate;

final class DirtyChunkPublisherTest {
    @Test
    void routesOneAcceptedRevisionToTheBridge() {
        final AtomicLong clock = new AtomicLong(40L);
        final List<DirtyChunkPublisher.DirtyUpdate> bridge = new ArrayList<>();
        final DirtyChunkPublisher publisher = DirtyChunkPublisher.forTesting(clock::incrementAndGet, bridge::add);

        publisher.publishAccepted(new ChunkCoordinate(4, 8));

        assertEquals(1, bridge.size());
        assertEquals(41L, clock.get());
        assertEquals(4, bridge.get(0).coordinate().x());
    }

    @Test
    void rejectedNotificationDoesNotAllocateRevision() {
        final AtomicLong clock = new AtomicLong(12L);
        final DirtyChunkPublisher publisher = DirtyChunkPublisher.forTesting(clock::incrementAndGet, ignored -> { });

        publisher.publishIf(false, new ChunkCoordinate(1, 2));

        assertEquals(12L, clock.get());
    }
}
