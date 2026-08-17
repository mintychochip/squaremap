package xyz.jpenilla.squaremap.common.bridge.process;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URI;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.StandardCopyOption;
import java.nio.file.StandardOpenOption;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.HexFormat;
import java.util.Objects;

/** Resolves only binaries whose bytes match the release manifest. */
public final class BinaryResolver {
    private BinaryResolver() {}

    public static Path cacheCandidate(
        final Path cacheRoot,
        final String pluginVersion,
        final String targetTriple,
        final BackendManifest.BackendBinary expected
    ) {
        Objects.requireNonNull(cacheRoot, "cacheRoot");
        Objects.requireNonNull(pluginVersion, "pluginVersion");
        Objects.requireNonNull(targetTriple, "targetTriple");
        Objects.requireNonNull(expected, "expected");
        if (pluginVersion.isBlank() || targetTriple.isBlank()) {
            throw new IllegalArgumentException("cache key components must not be blank");
        }
        if (pluginVersion.contains("/") || pluginVersion.contains("\\")
            || targetTriple.contains("/") || targetTriple.contains("\\")) {
            throw new IllegalArgumentException("cache key components must not contain separators");
        }
        final Path root = cacheRoot.toAbsolutePath().normalize();
        final Path candidate = root.resolve("squaremap").resolve(pluginVersion).resolve(targetTriple).resolve("backend").normalize();
        if (!candidate.startsWith(root)) {
            throw new IllegalArgumentException("cache candidate escapes cache root");
        }
        return candidate;
    }

    public static Path verifyCachedPath(
        final Path cacheRoot,
        final String pluginVersion,
        final String targetTriple,
        final BackendManifest.BackendBinary expected
    ) throws IOException {
        final Path candidate = cacheCandidate(cacheRoot, pluginVersion, targetTriple, expected);
        final Path verified = verifyConfiguredPath(candidate, expected);
        final Path root = cacheRoot.toAbsolutePath().normalize();
        final Path existingParent = verified.getParent().toRealPath();
        if (!existingParent.startsWith(root.toRealPath())) {
            throw new IllegalArgumentException("cached backend escapes cache root");
        }
        return verified;
    }
    public static Path resolve(
        final Path cacheRoot,
        final String pluginVersion,
        final String targetTriple,
        final BackendManifest.BackendBinary expected,
        final Path configured
    ) throws IOException {
        Objects.requireNonNull(expected, "expected");
        if (configured != null) {
            try {
                return verifyConfiguredPath(configured, expected);
            } catch (final RuntimeException ignored) {
                // Fall through to the verified cache/download path.
            }
        }
        final Path candidate = cacheCandidate(cacheRoot, pluginVersion, targetTriple, expected);
        try {
            return verifyCachedPath(cacheRoot, pluginVersion, targetTriple, expected);
        } catch (final IOException | RuntimeException missing) {
            downloadVerified(expected.url(), candidate, expected);
            return verifyCachedPath(cacheRoot, pluginVersion, targetTriple, expected);
        }
    }

    private static void downloadVerified(
        final URI source,
        final Path destination,
        final BackendManifest.BackendBinary expected
    ) throws IOException {
        final Path parent = destination.getParent();
        Files.createDirectories(parent);
        final Path realParent = parent.toRealPath();
        if (Files.isSymbolicLink(parent)) throw new IOException("backend cache parent must not be a symbolic link");
        final Path temporary = Files.createTempFile(realParent, ".backend-", ".part");
        try {
            final HttpURLConnection connection = (HttpURLConnection) source.toURL().openConnection();
            connection.setConnectTimeout(10_000);
            connection.setReadTimeout(30_000);
            connection.setInstanceFollowRedirects(false);
            if (connection.getResponseCode() / 100 != 2) throw new IOException("backend download failed: HTTP " + connection.getResponseCode());
            try (InputStream input = connection.getInputStream();
                 OutputStream output = Files.newOutputStream(temporary, StandardOpenOption.TRUNCATE_EXISTING)) {
                input.transferTo(output);
            }
            verifyConfiguredPath(temporary, expected);
            Files.move(temporary, destination, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
        } finally {
            Files.deleteIfExists(temporary);
        }
    }
    public static Path verifyConfiguredPath(final Path configured, final BackendManifest.BackendBinary expected) throws IOException {
        Objects.requireNonNull(configured, "configured");
        Objects.requireNonNull(expected, "expected");
        final Path path = configured.toAbsolutePath().normalize();
        if (!Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) {
            throw new IllegalArgumentException("configured Rust backend is not a regular file: " + path);
        }
        if (Files.isSymbolicLink(path) || Files.isSymbolicLink(path.getParent())) {
            throw new IllegalArgumentException("configured Rust backend must not be a symbolic link: " + path);
        }
        final Path stable = path.toRealPath(LinkOption.NOFOLLOW_LINKS);
        if (!stable.equals(path)) throw new IllegalArgumentException("configured Rust backend path changed during verification");
        if (!Files.isReadable(stable)) {
            throw new IllegalArgumentException("configured Rust backend is not readable: " + path);
        }
        final long length = Files.size(stable);
        if (length != expected.length()) {
            throw new IllegalArgumentException("Rust backend length mismatch for " + path + ": expected " + expected.length() + ", got " + length);
        }
        final String digest = sha256(stable);
        if (!digest.equals(expected.sha256())) {
            throw new IllegalArgumentException("Rust backend SHA-256 mismatch for " + path);
        }
        if (Files.size(stable) != length || !Files.isRegularFile(stable, LinkOption.NOFOLLOW_LINKS)) {
            throw new IllegalArgumentException("configured Rust backend changed during verification");
        }
        return stable;
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
