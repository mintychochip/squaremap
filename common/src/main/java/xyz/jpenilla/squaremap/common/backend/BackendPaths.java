package xyz.jpenilla.squaremap.common.backend;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Objects;
import xyz.jpenilla.squaremap.common.bridge.process.BackendMode;

/**
 * Resolves the isolated output roots used by the Java and Rust backends and
 * rejects any configuration in which two backends could write the same tree.
 *
 * <p>Canonicalization follows the longest existing ancestor through symlinks
 * (the path's {@code toRealPath()} operation) and appends the non-existing suffix verbatim, so
 * two paths that resolve through different symlinks or relative spellings compare as the same canonical root.
 * <p>This class never promotes one backend or redirects a backend at another
 * backend's root: {@link #resolve} only assigns each backend its own root and
 * {@link #validateDistinctRoots} refuses any overlap.
 */
public final class BackendPaths {
    private BackendPaths() {
    }

    /**
     * Canonicalizes a configured output root.
     *
     * <p>If the path exists, its fully resolved real path is returned. If part
     * of the path does not exist yet, the longest existing ancestor is
     * canonicalized and the non-existing suffix is appended back, preserving
     * the caller's spelling for the portion the backend will create.
     *
     * @param root configured output root; must be absolute
     * @return canonicalized root
     * @throws IllegalArgumentException when the root has no existing ancestor
     *         or cannot be resolved
     */
    public static Path canonicalize(final Path root) {
        final Path absolute = Objects.requireNonNull(root, "root").toAbsolutePath().normalize();
        Path existing = absolute;
        final List<Path> suffix = new ArrayList<>();
        while (!Files.exists(existing)) {
            final Path name = existing.getFileName();
            if (name == null) {
                throw new IllegalArgumentException("output root has no existing ancestor: " + absolute);
            }
            suffix.add(0, name);
            existing = existing.getParent();
            if (existing == null) {
                throw new IllegalArgumentException("output root has no existing ancestor: " + absolute);
            }
        }
        try {
            Path canonical = existing.toRealPath();
            for (final Path part : suffix) {
                canonical = canonical.resolve(part);
            }
            return canonical.normalize();
        } catch (final IOException error) {
            throw new IllegalArgumentException("could not canonicalize output root " + absolute, error);
        }
    }

    /**
     * Rejects equal roots, parent/child nesting, and symlink aliases between a
     * pair of backend output roots. Either root may be missing; non-existing
     * suffixes are resolved against the canonicalized existing ancestor.
     *
     * @throws IllegalArgumentException when the roots overlap or alias one tree
     */
    public static void validateDistinctRoots(final Path first, final Path second) {
        final Path canonicalFirst = canonicalize(first);
        final Path canonicalSecond = canonicalize(second);
        if (canonicalFirst.equals(canonicalSecond)) {
            throw new IllegalArgumentException(
                "backend output roots must be distinct; both resolve to " + canonicalFirst
            );
        }
        if (canonicalFirst.startsWith(canonicalSecond) || canonicalSecond.startsWith(canonicalFirst)) {
            throw new IllegalArgumentException(
                "backend output roots must not nest; " + canonicalFirst + " overlaps " + canonicalSecond
            );
        }
    }

    /**
     * Resolves the root for one backend mode and verifies it is isolated from
     * the other active backend's root. The Java backend owns the legacy web
     * output tree; shadow and rust backends must use a separate tree.
     *
     * @param mode backend mode; {@link BackendMode#JAVA} returns the Java root
     * @param javaRoot root used by the Java backend (uncanonicalized)
     * @param configuredRustRoot configured root for the rust backend
     * @return canonicalized root for {@code mode}
     * @throws IllegalArgumentException when roots overlap or the configured
     *         rust root is missing for a non-Java mode
     */
    public static Path resolve(
        final BackendMode mode,
        final Path javaRoot,
        final Path configuredRustRoot
    ) {
        Objects.requireNonNull(mode, "mode");
        Objects.requireNonNull(javaRoot, "javaRoot");
        if (mode == BackendMode.JAVA) {
            return canonicalize(javaRoot);
        }
        if (configuredRustRoot == null || !configuredRustRoot.isAbsolute()) {
            throw new IllegalArgumentException("Rust backend requires an absolute output root distinct from Java web output");
        }
        final Path canonicalRust = canonicalize(configuredRustRoot);
        validateDistinctRoots(javaRoot, configuredRustRoot);
        return canonicalRust;
    }
}
