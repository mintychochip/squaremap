package xyz.jpenilla.squaremap.common.bridge.verification;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParseException;
import com.google.gson.JsonParser;
import java.io.IOException;
import java.io.Reader;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.HexFormat;

/** Immutable, fail-closed metadata for the local Paper production proof. */
public record ProofFixture(
    int version,
    Artifact paper,
    Artifact plugin,
    Artifact sidecar,
    String minecraftVersion,
    World world,
    Roots roots,
    Ports ports,
    Map<String, Long> timeoutsMs,
    long quietIntervalMs,
    long rssIntervalMs,
    Map<String, Long> thresholds,
    List<String> scenario,
    List<String> normalization
) {
    private static final List<String> REQUIRED = List.of("readiness", "baseline", "mutation", "churn", "reload", "cancel", "resume", "quiescence", "kill", "restart", "replay", "second_mutation", "shutdown");

    public static ProofFixture load(final Path manifest) throws IOException {
        Objects.requireNonNull(manifest, "manifest");
        final JsonObject root;
        try (Reader reader = Files.newBufferedReader(manifest)) {
            final JsonElement element = JsonParser.parseReader(reader);
            if (!element.isJsonObject()) throw invalid("manifest must be an object");
            root = element.getAsJsonObject();
        } catch (final JsonParseException e) {
            throw invalid("manifest is not valid JSON", e);
        }
        final int version = integer(root, "version");
        if (version != 1) throw invalid("unsupported fixture version: " + version);
        final JsonObject data = object(root, "paper_fixture");
        final Path base = manifest.toAbsolutePath().normalize().getParent();
        final Artifact paper = artifact(data, "paper", base);
        final Artifact plugin = artifact(data, "plugin", base);
        final Artifact sidecar = artifact(data, "sidecar", base);
        final JsonObject worldJson = object(data, "world");
        final World world = new World(longValue(worldJson, "seed"), string(worldJson, "name"), positiveInt(worldJson, "view_distance"), positiveInt(worldJson, "simulation_distance"), string(worldJson, "timezone"), string(worldJson, "locale"));
        final JsonObject rootsJson = object(data, "roots");
        final Roots roots = new Roots(path(rootsJson, "data"), path(rootsJson, "java_output"), path(rootsJson, "rust_output"), path(rootsJson, "diagnostics"));
        validateRoots(roots);
        final JsonObject portsJson = object(data, "ports");
        final Ports ports = new Ports(port(portsJson, "paper"), port(portsJson, "rcon"), port(portsJson, "rust_http"));
        if (Set.of(ports.paper(), ports.rcon(), ports.rustHttp()).size() != 3) throw invalid("ports must not collide");
        final Map<String, Long> timeouts = positiveMap(object(data, "timeouts_ms"), "timeout");
        final long quiet = positive(data, "quiet_interval_ms");
        final long rss = positive(data, "rss_interval_ms");
        final Map<String, Long> thresholds = nonnegativeMap(object(data, "thresholds"));
        final List<String> scenario = strings(data, "scenario");
        if (!new LinkedHashSet<>(scenario).containsAll(REQUIRED)) throw invalid("scenario is missing a required lifecycle step");
        final List<String> normalization = strings(data, "normalization");
        for (String rule : normalization) if (!Set.of("volatile_headers", "volatile_timestamps").contains(rule)) throw invalid("undeclared normalization: " + rule);
        return new ProofFixture(version, paper, plugin, sidecar, string(data, "minecraft_version"), world, roots, ports, timeouts, quiet, rss, thresholds, List.copyOf(scenario), List.copyOf(normalization));
    }

    private static Artifact artifact(JsonObject data, String key, Path base) throws IOException {
        final JsonObject named = object(data, key);
        final String url = string(named, "url");
        final String version = string(named, "version");
        final String hash = string(named, "sha256").toLowerCase(Locale.ROOT);
        final JsonObject artifacts = object(data, "artifacts");
        final JsonObject file = object(artifacts, key);
        final Path path = base.resolve(string(file, "path")).normalize();
        if (!path.startsWith(base) || Files.isSymbolicLink(path) || !Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) throw invalid("missing or unsafe " + key + " artifact: " + path);
        final String expected = string(file, "sha256").toLowerCase(Locale.ROOT);
        if (!hash.equals(expected) || !hash.matches("[0-9a-f]{64}")) throw invalid("invalid " + key + " artifact hash");
        if (!sha256(path).equals(expected)) throw invalid("SHA-256 mismatch for " + key + " artifact");
        return new Artifact(version, url, expected, path);
    }

    private static void validateRoots(Roots roots) {
        final List<Path> paths = List.of(roots.data(), roots.javaOutput(), roots.rustOutput(), roots.diagnostics());
        for (int i = 0; i < paths.size(); i++) for (int j = i + 1; j < paths.size(); j++) if (paths.get(i).startsWith(paths.get(j)) || paths.get(j).startsWith(paths.get(i))) throw invalid("fixture roots overlap");
    }
    private static String sha256(Path path) throws IOException { try { return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(Files.readAllBytes(path))); } catch (NoSuchAlgorithmException e) { throw new AssertionError(e); } }
    private static JsonObject object(JsonObject o, String key) { JsonElement e = o.get(key); if (e == null || !e.isJsonObject()) throw invalid("missing object '" + key + "'"); return e.getAsJsonObject(); }
    private static String string(JsonObject o, String key) { JsonElement e = o.get(key); if (e == null || !e.isJsonPrimitive() || !e.getAsJsonPrimitive().isString() || e.getAsString().isBlank()) throw invalid("missing string '" + key + "'"); return e.getAsString(); }
    private static int integer(JsonObject o, String key) { try { return o.get(key).getAsInt(); } catch (RuntimeException e) { throw invalid("missing integer '" + key + "'"); } }
    private static int positiveInt(JsonObject o, String key) { int n = integer(o, key); if (n <= 0) throw invalid(key + " must be positive"); return n; }
    private static long longValue(JsonObject o, String key) { try { return o.get(key).getAsLong(); } catch (RuntimeException e) { throw invalid("missing number '" + key + "'"); } }
    private static long positive(JsonObject o, String key) { long n = longValue(o, key); if (n <= 0) throw invalid(key + " must be positive"); return n; }
    private static Path path(JsonObject o, String key) { String s = string(o, key); if (Path.of(s).isAbsolute() || s.contains("..")) throw invalid("root must be relative: " + key); return Path.of(s).normalize(); }
    private static int port(JsonObject o, String key) { int n = positiveInt(o, key); if (n > 65535) throw invalid("invalid port"); return n; }
    private static List<String> strings(JsonObject o, String key) { JsonElement e = o.get(key); if (e == null || !e.isJsonArray()) throw invalid("missing array '" + key + "'"); List<String> result = new ArrayList<>(); e.getAsJsonArray().forEach(x -> { if (!x.isJsonPrimitive() || !x.getAsJsonPrimitive().isString() || x.getAsString().isBlank()) throw invalid("invalid entry in " + key); result.add(x.getAsString()); }); return result; }
    private static Map<String, Long> positiveMap(JsonObject o, String ignored) { return numberMap(o, true); }
    private static Map<String, Long> nonnegativeMap(JsonObject o) { return numberMap(o, false); }
    private static Map<String, Long> numberMap(JsonObject o, boolean positive) { java.util.LinkedHashMap<String, Long> result = new java.util.LinkedHashMap<>(); o.entrySet().forEach(e -> { long n; try { n = e.getValue().getAsLong(); } catch (RuntimeException x) { throw invalid("invalid numeric value"); } if (positive ? n <= 0 : n < 0) throw invalid("invalid timing/threshold value"); result.put(e.getKey(), n); }); return Map.copyOf(result); }
    private static IllegalArgumentException invalid(String message) { return new IllegalArgumentException(message); }
    private static IllegalArgumentException invalid(String message, Throwable cause) { return new IllegalArgumentException(message, cause); }

    public record Artifact(String version, String url, String sha256, Path path) {}
    public record World(long seed, String name, int viewDistance, int simulationDistance, String timezone, String locale) {}
    public record Roots(Path data, Path javaOutput, Path rustOutput, Path diagnostics) {}
    public record Ports(int paper, int rcon, int rustHttp) {}
}
