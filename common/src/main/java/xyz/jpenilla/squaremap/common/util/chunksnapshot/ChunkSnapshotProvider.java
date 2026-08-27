package xyz.jpenilla.squaremap.common.util.chunksnapshot;

import java.util.concurrent.CompletableFuture;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.checker.nullness.qual.Nullable;
import org.checkerframework.framework.qual.DefaultQualifier;

@DefaultQualifier(NonNull.class)
public interface ChunkSnapshotProvider {
    CompletableFuture<@Nullable ChunkSnapshot> asyncSnapshot(int x, int z);

    /** Live ticks pass {@code loadedOnly} so distant dirties never {@code scheduleChunkLoad}. */
    default CompletableFuture<@Nullable ChunkSnapshot> asyncSnapshot(int x, int z, boolean loadedOnly) {
        return this.asyncSnapshot(x, z);
    }
}
