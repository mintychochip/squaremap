package xyz.jpenilla.squaremap.common.backend;

import java.util.List;
import java.util.Objects;
import xyz.jpenilla.squaremap.api.WorldIdentifier;

/** Structured backend outcome; Java maps codes to localized messages. */
public record BackendResult(Code code, List<Substitution> substitutions) {
    public BackendResult {
        Objects.requireNonNull(code, "code");
        substitutions = List.copyOf(substitutions == null ? List.of() : substitutions);
    }

    public static BackendResult of(final Code code) {
        return new BackendResult(code, List.of());
    }

    public static BackendResult world(final Code code, final WorldIdentifier world) {
        return new BackendResult(code, List.of(new Substitution("world", new World(world))));
    }

    public enum Code {
        FULL_RENDER_STARTED,
        RADIUS_RENDER_STARTED,
        RENDER_IN_PROGRESS,
        RENDER_NOT_IN_PROGRESS,
        RENDER_CANCELLED,
        RENDERS_PAUSED,
        RENDERS_RESUMED,
        MAP_RESET,
        RELOADED,
        HEALTHY,
        UNKNOWN_WORLD,
        INVALID_REQUEST,
        INVALID_CONFIG,
        BACKEND_UNAVAILABLE,
        BACKEND_TIMEOUT,
        FAILED,
        PROGRESS_LOGGING_RESTARTED
    }

    public record Substitution(String key, Value value) {
        public Substitution {
            Objects.requireNonNull(key, "key");
            if (key.isBlank()) throw new IllegalArgumentException("substitution key must not be blank");
            Objects.requireNonNull(value, "value");
        }
    }

    public sealed interface Value permits Text, Integer, Boolean, World {}
    public record Text(String value) implements Value { public Text { Objects.requireNonNull(value, "value"); } }
    public record Integer(long value) implements Value {}
    public record Boolean(boolean value) implements Value {}
    public record World(WorldIdentifier value) implements Value { public World { Objects.requireNonNull(value, "value"); } }
}
