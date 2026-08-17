package xyz.jpenilla.squaremap.common.bridge.state;

import com.google.inject.Inject;
import com.google.inject.Singleton;
import java.util.Objects;
import java.util.function.Consumer;
import java.util.function.LongSupplier;
import xyz.jpenilla.squaremap.common.backend.BridgeBackendController;
import xyz.jpenilla.squaremap.common.data.ChunkCoordinate;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;

/** Emits one eligible loader dirty notification to the Rust backend. */
@Singleton
public final class DirtyChunkPublisher {
    private final LongSupplier revisions;
    private final Consumer<DirtyUpdate> bridge;

    @Inject
    public DirtyChunkPublisher(
        final BridgeRevisionClock revisions,
        final BridgeBackendController bridge
    ) {
        this(revisions::next, update -> bridge.publishDirty(update.world(), update.coordinate(), update.revision()));
    }

    private DirtyChunkPublisher(final LongSupplier revisions, final Consumer<DirtyUpdate> bridge) {
        this.revisions = Objects.requireNonNull(revisions, "revisions");
        this.bridge = Objects.requireNonNull(bridge, "bridge");
    }

    static DirtyChunkPublisher forTesting(final LongSupplier revisions, final Consumer<DirtyUpdate> bridge) {
        return new DirtyChunkPublisher(revisions, bridge);
    }

    public void publish(final MapWorldInternal world, final ChunkCoordinate coordinate) {
        this.publishIf(world.shouldRenderDirtyChunk(coordinate), world, coordinate);
    }

    void publishIf(final boolean eligible, final ChunkCoordinate coordinate) {
        this.publishIf(eligible, null, coordinate);
    }

    private void publishIf(
        final boolean eligible,
        final MapWorldInternal world,
        final ChunkCoordinate coordinate
    ) {
        if (!eligible) {
            return;
        }
        this.publishAccepted(world, coordinate);
    }

    void publishAccepted(final ChunkCoordinate coordinate) {
        this.publishAccepted(null, coordinate);
    }

    private void publishAccepted(final MapWorldInternal world, final ChunkCoordinate coordinate) {
        this.bridge.accept(new DirtyUpdate(world, coordinate, this.revisions.getAsLong()));
    }

    record DirtyUpdate(MapWorldInternal world, ChunkCoordinate coordinate, long revision) {
        DirtyUpdate {
            Objects.requireNonNull(coordinate, "coordinate");
            if (revision < 0L) {
                throw new IllegalArgumentException("revision must be non-negative");
            }
        }
    }
}
