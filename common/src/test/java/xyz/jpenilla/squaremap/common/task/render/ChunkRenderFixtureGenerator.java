package xyz.jpenilla.squaremap.common.task.render;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.condition.EnabledIfSystemProperty;

/** Explicit opt-in generator; ordinary oracle comparison never writes. */
final class ChunkRenderFixtureGenerator {
    private ChunkRenderFixtureGenerator() {}

    @Test
    @EnabledIfSystemProperty(named = "squaremap.regenerate", matches = "--overwrite")
    void overwriteFixture() throws Exception { write(); }

    private static void write() throws Exception {
        if (!"--overwrite".equals(System.getProperty("squaremap.regenerate"))) throw new IllegalArgumentException("pass -Dsquaremap.regenerate=--overwrite");
        final Path manifest = ChunkRenderFixtureOracleTest.FIXTURE;
        final Path corpus = manifest.getParent();
        final ChunkRenderFixtureDocument.Projection projection = ChunkRenderFixtureDocument.build(ChunkRenderFixtureCatalog.create());
        final Path temporaryRoot = corpus.resolve("render.tmp");
        Files.createDirectories(temporaryRoot);
        try {
            for (final var entry : projection.files().entrySet()) {
                final Path target = temporaryRoot.resolve(entry.getKey());
                Files.createDirectories(target.getParent());
                Files.write(target, entry.getValue());
            }
            Files.writeString(temporaryRoot.resolve("manifest.json"), projection.document().toString());
            for (final var entry : projection.files().entrySet()) {
                final Path source = temporaryRoot.resolve(entry.getKey());
                final Path target = corpus.resolve(entry.getKey());
                Files.createDirectories(target.getParent());
                Files.move(source, target, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
            }
            Files.move(temporaryRoot.resolve("manifest.json"), manifest, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
            final java.util.Set<Path> expected = new java.util.HashSet<>(projection.files().keySet());
            expected.add(Path.of("manifest.json"));
            try (var paths = Files.walk(corpus)) {
                paths.filter(path -> !path.equals(corpus) && !path.startsWith(temporaryRoot) && !expected.contains(corpus.relativize(path)))
                    .sorted(java.util.Comparator.reverseOrder()).forEach(path -> {
                        try {
                            if (Files.isDirectory(path)) try (var children = Files.list(path)) { if (children.findAny().isPresent()) return; }
                            Files.deleteIfExists(path);
                        } catch (Exception exception) { throw new RuntimeException(exception); }
                    });
            }
        } finally {
            if (Files.exists(temporaryRoot)) try (var paths = Files.walk(temporaryRoot)) { paths.sorted(java.util.Comparator.reverseOrder()).forEach(path -> { try { Files.deleteIfExists(path); } catch (Exception ignored) {} }); }
        }
    }
}
