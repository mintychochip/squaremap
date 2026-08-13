package xyz.jpenilla.squaremap.common.bridge.process;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.HexFormat;
import java.util.Objects;

/** Resolves only binaries whose bytes match the release manifest. */
public final class BinaryResolver {
    private BinaryResolver() {}

    public static Path verifyConfiguredPath(final Path configured, final BackendManifest.BackendBinary expected) throws IOException {
        Objects.requireNonNull(configured, "configured");
        Objects.requireNonNull(expected, "expected");
        final Path path = configured.toAbsolutePath().normalize();
        if (!Files.isRegularFile(path)) {
            throw new IllegalArgumentException("configured Rust backend is not a regular file: " + path);
        }
        final long length = Files.size(path);
        if (length != expected.length()) {
            throw new IllegalArgumentException("Rust backend length mismatch for " + path + ": expected " + expected.length() + ", got " + length);
        }
        final String digest = sha256(path);
        if (!digest.equals(expected.sha256())) {
            throw new IllegalArgumentException("Rust backend SHA-256 mismatch for " + path);
        }
        return path;
    }

    private static String sha256(final Path path) throws IOException {
        try {
            final MessageDigest digest = MessageDigest.getInstance("SHA-256");
            try (InputStream input = Files.newInputStream(path)) {
                final byte[] buffer = new byte[8192];
                int read;
                while ((read = input.read(buffer)) >= 0) {
                    if (read > 0) digest.update(buffer, 0, read);
                }
            }
            return HexFormat.of().formatHex(digest.digest());
        } catch (final NoSuchAlgorithmException impossible) {
            throw new AssertionError(impossible);
        }
    }
}
