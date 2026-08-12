package xyz.jpenilla.squaremap.common.bridge.snapshot;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.concurrent.CompletableFuture;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshot;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshotRequest;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;

final class SnapshotRequestServiceContractTest {
    private static ChunkSnapshotRequest request(final long id) {
        return ChunkSnapshotRequest.newBuilder().setRequestId(id).setRevision(1)
            .setWorld(WorldIdentity.newBuilder().setNamespace("minecraft").setValue("overworld").setEpoch(1)).build();
    }

    @Test
    void cancellationCompletesResultAndReleasesCancellableStages() {
        final SnapshotRequestService service = new SnapshotRequestService(1);
        final CompletableFuture<ChunkSnapshot> upstream = new CompletableFuture<>();
        final CompletableFuture<ChunkSnapshot> encoded = new CompletableFuture<>();
        final var result = service.requestWork(request(1), ignored -> new SnapshotRequestService.Work(encoded, upstream));
        assertTrue(service.cancel(1));
        assertTrue(result.isCompletedExceptionally());
        assertTrue(upstream.isCancelled());
        assertTrue(encoded.isCancelled());
        service.awaitIdle();
        assertEquals(0, service.inFlightCount());
    }
}
