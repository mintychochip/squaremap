package xyz.jpenilla.squaremap.common.bridge.process;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

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
}
