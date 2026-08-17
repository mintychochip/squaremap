package xyz.jpenilla.squaremap.common.bridge.process;

import com.google.gson.JsonElement;
import com.google.gson.JsonIOException;
import com.google.gson.JsonObject;
import com.google.gson.JsonParseException;
import com.google.gson.JsonSyntaxException;
import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.Reader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Locale;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.regex.Pattern;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.checker.nullness.qual.Nullable;
import org.checkerframework.framework.qual.DefaultQualifier;
import xyz.jpenilla.squaremap.common.util.Util;
/**
 * Build-generated description of the native backend binaries released for this plugin version.
 *
 * <p>The manifest is produced at build time from the actual release binaries (never hand
 * edited) and packaged into every distribution jar at {@code /squaremap-backends.json}.
 * It maps the exact plugin version and the supported Rust target triples to the URL, byte
 * length, and lowercase SHA-256 of each stripped binary.
 */
@DefaultQualifier(NonNull.class)
public final class BackendManifest {
    /** Classpath resource path inside every distribution jar. */
    public static final String RESOURCE_PATH = "/squaremap-backends.json";

    /** The five Rust targets locked in the global constraints of the migration plan. */
    private static final Set<String> LOCKED_TRIPLES = Set.of(
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin"
    );

    private static final Pattern SHA256_PATTERN = Pattern.compile("^[0-9a-f]{64}$");
    private static final Pattern NON_HEX = Pattern.compile("[^0-9a-fA-F]");

    private final String pluginVersion;
    private final Map<String, BackendBinary> targets;

    private BackendManifest(final String pluginVersion, final Map<String, BackendBinary> targets) {
        this.pluginVersion = pluginVersion;
        this.targets = Map.copyOf(targets);
    }

    /** The exact plugin version these binaries were built for. */
    public String pluginVersion() {
        return this.pluginVersion;
    }

    /** Immutable map of Rust target triple to released binary. */
    public Map<String, BackendBinary> targets() {
        return this.targets;
    }

    /**
     * The released binary for {@code triple}, or {@code null} when this manifest does not
     * ship a binary for that platform.
     */
    public @Nullable BackendBinary forTarget(final String triple) {
        return this.targets.get(Objects.requireNonNull(triple, "triple"));
    }

    /**
     * Requires this manifest to belong to the running plugin version.
     *
     * @throws BackendManifestException when the versions differ
     */
    public BackendManifest requireVersion(final String pluginVersion) {
        if (!this.pluginVersion.equals(Objects.requireNonNull(pluginVersion, "pluginVersion"))) {
            throw new BackendManifestException(
                "Native backend manifest was built for plugin version '%s' but this plugin is '%s'; "
                    + "reinstall the plugin or install backend binaries for the matching version".formatted(
                    this.pluginVersion,
                    pluginVersion
                )
            );
        }
        return this;
    }

    /** Reads, parses, and validates the packaged manifest from the classpath. */
    public static BackendManifest load() throws IOException {
        final InputStream stream = BackendManifest.class.getResourceAsStream(RESOURCE_PATH);
        if (stream == null) {
            throw new BackendManifestException(
                "Packaged manifest " + RESOURCE_PATH + " is missing from this distribution; "
                    + "reinstall the plugin or run the development launcher with squaremap.backendBinary"
            );
        }
        try (InputStream closed = stream; Reader reader = new InputStreamReader(closed, StandardCharsets.UTF_8)) {
            return parse(reader);
        }
    }

    /** Reads, parses, and validates a manifest from a file (tests and tooling). */
    public static BackendManifest read(final Path path) throws IOException {
        try (Reader reader = Files.newBufferedReader(path, StandardCharsets.UTF_8)) {
            return parse(reader);
        }
    }

    /**
     * Parses and strictly validates a manifest document.
     *
     * @throws BackendManifestException on structural or cryptographic validation failures;
     *     {@link JsonIOException}/{@link JsonSyntaxException} propagate from the reader
     */
    public static BackendManifest parse(final Reader reader) throws IOException {
        final JsonObject root;
        try {
            final JsonElement element = Util.gson().fromJson(reader, JsonElement.class);
            if (element == null || !element.isJsonObject()) {
                throw new BackendManifestException("native backend manifest must be a JSON object");
            }
            root = element.getAsJsonObject();
        } catch (final JsonParseException error) {
            throw new BackendManifestException("native backend manifest is not valid JSON", error);
        }

        final @Nullable JsonElement versionElement = root.get("pluginVersion");
        if (versionElement == null || !versionElement.isJsonPrimitive() || !versionElement.getAsJsonPrimitive().isString()) {
            throw new BackendManifestException("native backend manifest is missing a string 'pluginVersion'");
        }
        final String pluginVersion = versionElement.getAsString().trim();
        if (pluginVersion.isEmpty()) {
            throw new BackendManifestException("native backend manifest 'pluginVersion' must not be blank");
        }

        final @Nullable JsonElement targetsElement = root.get("targets");
        if (targetsElement == null || !targetsElement.isJsonObject()) {
            throw new BackendManifestException("native backend manifest is missing an object 'targets'");
        }
        final Map<String, BackendBinary> targets = new LinkedHashMap<>();
        for (final Map.Entry<String, JsonElement> entry : targetsElement.getAsJsonObject().entrySet()) {
            final String triple = entry.getKey();
            if (!LOCKED_TRIPLES.contains(triple)) {
                throw new BackendManifestException(
                    "native backend manifest contains unsupported target '%s'; supported targets are %s"
                        .formatted(triple, String.join(", ", LOCKED_TRIPLES))
                );
            }
            final JsonElement value = entry.getValue();
            if (!value.isJsonObject()) {
                throw new BackendManifestException("native backend manifest target '%s' must be an object".formatted(triple));
            }
            final JsonObject binary = value.getAsJsonObject();
            targets.put(triple, parseBinary(triple, binary));
        }
        if (targets.isEmpty()) {
            throw new BackendManifestException("native backend manifest 'targets' must not be empty");
        }

        return new BackendManifest(pluginVersion, Collections.unmodifiableMap(targets));
    }

    private static BackendBinary parseBinary(final String triple, final JsonObject binary) {
        final String url = requiredString(binary, triple, "url");
        final String lengthValue = requiredString(binary, triple, "length");
        final String sha256 = normalizeSha256(requiredString(binary, triple, "sha256"), triple);

        final long length;
        try {
            length = Long.parseLong(lengthValue);
        } catch (final NumberFormatException error) {
            throw new BackendManifestException(
                "native backend manifest target '%s' has invalid 'length' '%s'".formatted(triple, lengthValue),
                error
            );
        }
        if (length <= 0L) {
            throw new BackendManifestException(
                "native backend manifest target '%s' has non-positive 'length' '%s'".formatted(triple, lengthValue)
            );
        }

        final java.net.URI uri;
        try {
            uri = java.net.URI.create(url);
        } catch (final IllegalArgumentException error) {
            throw new BackendManifestException(
                "native backend manifest target '%s' has invalid 'url' '%s'".formatted(triple, url),
                error
            );
        }
        if (uri.getScheme() == null || uri.getHost() == null || uri.getPath() == null) {
            throw new BackendManifestException(
                "native backend manifest target '%s' has non-absolute 'url' '%s'".formatted(triple, url)
            );
        }

        return new BackendBinary(uri, length, sha256);
    }

    private static String requiredString(final JsonObject binary, final String triple, final String field) {
        final @Nullable JsonElement element = binary.get(field);
        if (element == null || !element.isJsonPrimitive() || !element.getAsJsonPrimitive().isString()) {
            throw new BackendManifestException(
                "native backend manifest target '%s' is missing string field '%s'".formatted(triple, field)
            );
        }
        final String value = element.getAsString();
        if (value.isEmpty()) {
            throw new BackendManifestException(
                "native backend manifest target '%s' field '%s' must not be blank".formatted(triple, field)
            );
        }
        return value;
    }

    private static String normalizeSha256(final String value, final String triple) {
        if (!SHA256_PATTERN.matcher(value).matches()) {
            throw new BackendManifestException(
                "native backend manifest target '%s' has invalid 'sha256' '%s'; expected 64 lowercase hex characters"
                    .formatted(triple, value)
            );
        }
        return value;
    }

    /** A single released backend binary: download URL, byte length, lowercase SHA-256. */
    public static final class BackendBinary {
        private final java.net.URI url;
        private final long length;
        private final String sha256;

        BackendBinary(final java.net.URI url, final long length, final String sha256) {
            this.url = url;
            this.length = length;
            this.sha256 = sha256;
        }

        /** Release download URL recorded in the manifest. */
        public java.net.URI url() {
            return this.url;
        }

        /** Exact byte length of the released binary. */
        public long length() {
            return this.length;
        }

        /** Lowercase hexadecimal SHA-256 of the released binary. */
        public String sha256() {
            return this.sha256;
        }
    }

    /** Actionable failure when the native backend manifest cannot be loaded or matched. */
    public static final class BackendManifestException extends RuntimeException {
        public BackendManifestException(final String message) {
            super(message);
        }

        public BackendManifestException(final String message, final Throwable cause) {
            super(message, cause);
        }
    }
}
