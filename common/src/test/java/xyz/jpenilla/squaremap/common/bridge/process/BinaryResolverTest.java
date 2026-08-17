package xyz.jpenilla.squaremap.common.bridge.process;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.nio.file.FileSystemException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.HexFormat;
import org.junit.jupiter.api.Test;

final class BinaryResolverTest {
    @Test
    void acceptsExactBytes() throws Exception {
        final Path file = Files.createTempFile("squaremap-backend", ".bin");
        Files.writeString(file, "backend", StandardCharsets.UTF_8);
        final byte[] bytes = Files.readAllBytes(file);
        final String sha = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
        final BackendManifest.BackendBinary binary = new BackendManifest.BackendBinary(
            java.net.URI.create("https://example.invalid/backend"), bytes.length, sha);
        assertEquals(file.toAbsolutePath().normalize(), BinaryResolver.verifyConfiguredPath(file, binary));
    }

    @Test
    void rejectsDigestMismatch() throws Exception {
        final Path file = Files.createTempFile("squaremap-backend", ".bin");
        Files.writeString(file, "backend", StandardCharsets.UTF_8);
        final BackendManifest.BackendBinary binary = new BackendManifest.BackendBinary(
            java.net.URI.create("https://example.invalid/backend"), 7, "0".repeat(64));
        assertThrows(IllegalArgumentException.class, () -> BinaryResolver.verifyConfiguredPath(file, binary));
    }

    @Test
    void rejectsSymbolicLink() throws Exception {
        final Path target = Files.createTempFile("squaremap-backend-target", ".bin");
        Files.writeString(target, "backend", StandardCharsets.UTF_8);
        final Path link = target.resolveSibling("squaremap-backend-link.bin");
        try {
            Files.createSymbolicLink(link, target);
        } catch (UnsupportedOperationException | java.nio.file.FileSystemException ignored) {
            return;
        }
        final byte[] bytes = Files.readAllBytes(target);
        final String sha = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
        final BackendManifest.BackendBinary binary = new BackendManifest.BackendBinary(
            java.net.URI.create("https://example.invalid/backend"), bytes.length, sha);
        assertThrows(IllegalArgumentException.class, () -> BinaryResolver.verifyConfiguredPath(link, binary));
    }

    @Test
    void derivesConfinedVersionedTargetCacheCandidate() {
        final Path root = Path.of("/tmp/squaremap-cache");
        final BackendManifest.BackendBinary binary = new BackendManifest.BackendBinary(
            java.net.URI.create("https://example.invalid/backend"), 7, "0".repeat(64));
        assertEquals(
            root.resolve("squaremap").resolve("fixture").resolve("x86_64-unknown-linux-gnu").resolve("backend"),
            BinaryResolver.cacheCandidate(root, "fixture", "x86_64-unknown-linux-gnu", binary)
        );
    }

    @Test
    void rejectsTraversalInCacheKeyComponents() {
        final BackendManifest.BackendBinary binary = new BackendManifest.BackendBinary(
            java.net.URI.create("https://example.invalid/backend"), 7, "0".repeat(64));
        assertThrows(IllegalArgumentException.class, () ->
            BinaryResolver.cacheCandidate(Path.of("/tmp/cache"), "../fixture", "target", binary));
        assertThrows(IllegalArgumentException.class, () ->
            BinaryResolver.cacheCandidate(Path.of("/tmp/cache"), "fixture", "target/escape", binary));
    }

    @Test
    void verifiesExistingCachedCandidateAgainstManifest() throws Exception {
        final Path root = Files.createTempDirectory("squaremap-cache");
        final Path candidate = root.resolve("squaremap/fixture/target/backend");
        Files.createDirectories(candidate.getParent());
        Files.writeString(candidate, "backend", StandardCharsets.UTF_8);
        final byte[] bytes = Files.readAllBytes(candidate);
        final BackendManifest.BackendBinary binary = new BackendManifest.BackendBinary(
            java.net.URI.create("https://example.invalid/backend"), bytes.length,
            HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes)));
        assertEquals(candidate, BinaryResolver.verifyCachedPath(root, "fixture", "target", binary));
    }

    @Test
    void rejectsMissingCachedCandidateOffline() throws Exception {
        final Path root = Files.createTempDirectory("squaremap-cache");
        final BackendManifest.BackendBinary binary = new BackendManifest.BackendBinary(
            java.net.URI.create("https://example.invalid/backend"), 7, "0".repeat(64));
        assertThrows(IllegalArgumentException.class, () ->
            BinaryResolver.verifyCachedPath(root, "fixture", "target", binary));
    }

    @Test
    void rejectsSymlinkedCacheParent() throws Exception {
        final Path root = Files.createTempDirectory("squaremap-cache");
        final Path outside = Files.createTempDirectory("squaremap-cache-outside");
        Files.createDirectories(outside.resolve("target"));
        Files.writeString(outside.resolve("target/backend"), "backend", StandardCharsets.UTF_8);
        Files.createDirectories(root.resolve("squaremap/fixture"));
        try {
            Files.createSymbolicLink(root.resolve("squaremap/fixture/target"), outside.resolve("target"));
        } catch (UnsupportedOperationException | FileSystemException ignored) {
            return;
        }
        final byte[] bytes = Files.readAllBytes(outside.resolve("target/backend"));
        final BackendManifest.BackendBinary binary = new BackendManifest.BackendBinary(
            java.net.URI.create("https://example.invalid/backend"), bytes.length,
            HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes)));
        assertThrows(IllegalArgumentException.class, () ->
            BinaryResolver.verifyCachedPath(root, "fixture", "target", binary));
    }
    @Test
    void rejectsConfiguredPathWhoseParentIsSymlink() throws Exception {
        final Path root = Files.createTempDirectory("squaremap-parent-race");
        final Path real = Files.createDirectory(root.resolve("real"));
        final Path binary = real.resolve("backend");
        Files.writeString(binary, "backend", StandardCharsets.UTF_8);
        final Path alias = root.resolve("alias");
        try {
            Files.createSymbolicLink(alias, real);
        } catch (final UnsupportedOperationException | FileSystemException ignored) {
            return;
        }
        final byte[] bytes = Files.readAllBytes(binary);
        final BackendManifest.BackendBinary expected = new BackendManifest.BackendBinary(
            java.net.URI.create("https://example.invalid/backend"), bytes.length,
            HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes)));
        assertThrows(IllegalArgumentException.class, () -> BinaryResolver.verifyConfiguredPath(alias.resolve("backend"), expected));
    }
}
