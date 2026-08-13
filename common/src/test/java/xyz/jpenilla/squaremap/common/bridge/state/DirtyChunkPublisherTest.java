package xyz.jpenilla.squaremap.common.bridge.state;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicLong;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.common.bridge.process.BackendMode;
import xyz.jpenilla.squaremap.common.data.ChunkCoordinate;

final class DirtyChunkPublisherTest {
    @Test
    void routesOneAcceptedRevisionAccordingToBackendMode() {
        for (final BackendMode mode : BackendMode.values()) {
            final AtomicLong clock = new AtomicLong(40L);
            final List<DirtyChunkPublisher.DirtyUpdate> legacy = new ArrayList<>();
            final List<DirtyChunkPublisher.DirtyUpdate> bridge = new ArrayList<>();
            final DirtyChunkPublisher publisher = DirtyChunkPublisher.forTesting(
                mode,
                clock::incrementAndGet,
                legacy::add,
                bridge::add
            );

            publisher.publishAccepted(new ChunkCoordinate(4, 8));

            assertEquals(mode == BackendMode.RUST ? 0 : 1, legacy.size(), mode.name());
            assertEquals(mode == BackendMode.JAVA ? 0 : 1, bridge.size(), mode.name());
            assertEquals(41L, clock.get(), mode.name());
            if (mode == BackendMode.SHADOW) {
                assertEquals(legacy.get(0), bridge.get(0));
            }
        }
    }

    @Test
    void rejectedNotificationDoesNotAllocateRevision() {
        final AtomicLong clock = new AtomicLong(12L);
        final DirtyChunkPublisher publisher = DirtyChunkPublisher.forTesting(
            BackendMode.SHADOW,
            clock::incrementAndGet,
            ignored -> { },
            ignored -> { }
        );

        publisher.publishIf(false, new ChunkCoordinate(1, 2));

        assertEquals(12L, clock.get());
    }
}
