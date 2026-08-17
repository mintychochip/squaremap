package xyz.jpenilla.squaremap.common.bridge.snapshot;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicInteger;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.common.data.ChunkCoordinate;
import xyz.jpenilla.squaremap.common.data.RegionCoordinate;

final class SnapshotRequestHandlerTest {
    @Test
    void pagesAreSortedAndBounded() throws Exception {
        final Path directory = Files.createTempDirectory("squaremap-enumeration");
        try {
            Files.write(directory.resolve("r.1.0.mca"), new byte[] {1});
            Files.write(directory.resolve("r.0.0.mca"), new byte[] {1});
            final SnapshotRequestHandler.EnumerationPage page0 = SnapshotRequestHandler.enumerate(
                directory, ignored -> true, ignored -> true, 2, 0);
            final SnapshotRequestHandler.EnumerationPage page1 = SnapshotRequestHandler.enumerate(
                directory, ignored -> true, ignored -> true, 2, 2);
            assertEquals(List.of(new ChunkCoordinate(0, 0), new ChunkCoordinate(0, 1)), page0.items());
            assertEquals(List.of(new ChunkCoordinate(0, 2), new ChunkCoordinate(0, 3)), page1.items());
            assertTrue(page0.hasMore());
            assertTrue(page1.hasMore());
        } finally {
            Files.walk(directory).sorted(java.util.Comparator.reverseOrder()).forEach(path -> path.toFile().delete());
        }
    }

    @Test
    void visitationStopsAfterPageCapacity() throws Exception {
        final Path directory = Files.createTempDirectory("squaremap-enumeration");
        try {
            Files.write(directory.resolve("r.0.0.mca"), new byte[] {1});
            final AtomicInteger visits = new AtomicInteger();
            final SnapshotRequestHandler.EnumerationPage page = SnapshotRequestHandler.enumerate(
                directory, ignored -> true, ignored -> { visits.incrementAndGet(); return true; }, 2, 0);
            assertEquals(2, page.items().size());
            assertTrue(page.hasMore());
            assertEquals(3, visits.get());
        } finally {
            Files.walk(directory).sorted(java.util.Comparator.reverseOrder()).forEach(path -> path.toFile().delete());
        }
    }

    @Test
    void pageConstantsDefineContract() {
        assertEquals(16, SnapshotRequestHandler.MAX_ENUMERATION_PAGES);
        assertEquals(1024, SnapshotRequestHandler.MAX_ENUMERATION_ITEMS);
    }
}
