package xyz.jpenilla.squaremap.common.bridge.state;

import java.util.HashMap;
import java.util.Map;
import java.util.Objects;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import com.google.inject.Singleton;

/** Allocates one monotonically increasing epoch for each observed world load. */
@Singleton
public final class WorldEpochRegistry {
    private final Map<WorldIdentifier, Observation> observations = new HashMap<>();
    private final Map<WorldIdentifier, Long> lastEpoch = new HashMap<>();

    public synchronized long epoch(final WorldIdentifier identifier, final Object world) {
        final Observation previous = this.observations.get(identifier);
        if (previous != null && previous.world == world) return previous.epoch;
        final long next = Math.max(1L, this.lastEpoch.getOrDefault(identifier, 0L) + 1L);
        this.observations.put(identifier, new Observation(world, next));
        this.lastEpoch.put(identifier, next);
        return next;
    }

    private record Observation(Object world, long epoch) {
        private Observation {
            Objects.requireNonNull(world, "world");
        }
    }
}
