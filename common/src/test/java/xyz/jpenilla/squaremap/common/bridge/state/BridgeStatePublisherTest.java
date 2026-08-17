package xyz.jpenilla.squaremap.common.bridge.state;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertSame;

import java.util.concurrent.atomic.AtomicInteger;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.bridge.v1.PlayersReplace;

final class BridgeStatePublisherTest {
    @Test
    void shadowPublishesOneCollectedPlayersValueToBothSinks() {
        final AtomicInteger collections = new AtomicInteger();
        final PlayersReplace value = PlayersReplace.newBuilder().setRevision(1).setMaxPlayers(20).build();
        final java.util.List<PlayersReplace> bridge = new java.util.ArrayList<>();
        final BridgeStatePublisher publisher = BridgeStatePublisher.forTesting(
            () -> { collections.incrementAndGet(); return value; },
            bridge::add
        );

        publisher.publishPlayers();
        assertEquals(1, collections.get());
        assertSame(value, bridge.get(0));

        publisher.publishPlayers();
        assertEquals(2, collections.get());
        assertEquals(1, bridge.size());
    }
}
