package xyz.jpenilla.squaremap.common.bridge.state;

import com.google.inject.Inject;
import com.google.inject.Singleton;
import java.util.Objects;
import java.util.function.BooleanSupplier;
import java.util.function.Consumer;
import java.util.function.LongSupplier;
import xyz.jpenilla.squaremap.common.backend.BridgeBackendController;
import xyz.jpenilla.squaremap.common.bridge.process.BackendMode;
import xyz.jpenilla.squaremap.common.bridge.process.BridgeBootstrapConfig;
import xyz.jpenilla.squaremap.common.data.ChunkCoordinate;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;

/** Routes one eligible loader dirty notification to the selected map backend(s). */
@Singleton
public final class DirtyChunkPublisher {
    private final BackendMode mode;
    private final LongSupplier revisions;
    private final DirtySink legacy;
    private final DirtySink bridge;

    @Inject
    public DirtyChunkPublisher(
        final BridgeBootstrapConfig config,
        final BridgeRevisionClock revisions,
        final BridgeBackendController bridge
    ) {
        this(
            config.backendMode(),
            revisions::next,
            update -> update.world().chunkModified(update.coordinate()),
            update -> bridge.publishDirty(update.world(), update.coordinate(), update.revision())
        );
    }

    private DirtyChunkPublisher(
        final BackendMode mode,
        final LongSupplier revisions,
        final DirtySink legacy,
        final DirtySink bridge
    ) {
        this.mode = Objects.requireNonNull(mode, "mode");
        this.revisions = Objects.requireNonNull(revisions, "revisions");
        this.legacy = Objects.requireNonNull(legacy, "legacy");
        this.bridge = Objects.requireNonNull(bridge, "bridge");
    }

    static DirtyChunkPublisher forTesting(
        final BackendMode mode,
        final LongSupplier revisions,
        final Consumer<DirtyUpdate> legacy,
        final Consumer<DirtyUpdate> bridge
    ) {
        return new DirtyChunkPublisher(mode, revisions, legacy::accept, bridge::accept);
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
        final DirtyUpdate update = new DirtyUpdate(world, coordinate, this.revisions.getAsLong());
        if (this.mode != BackendMode.RUST) {
            this.legacy.accept(update);
        }
        if (this.mode != BackendMode.JAVA) {
            this.bridge.accept(update);
        }
    }

    record DirtyUpdate(MapWorldInternal world, ChunkCoordinate coordinate, long revision) {
        DirtyUpdate {
            Objects.requireNonNull(coordinate, "coordinate");
            if (revision < 0L) {
                throw new IllegalArgumentException("revision must be non-negative");
            }
        }
    }

    @FunctionalInterface
    private interface DirtySink {
        void accept(DirtyUpdate update);
    }
}
