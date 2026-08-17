package xyz.jpenilla.squaremap.common.bridge.process;

/** The only supported map backend. */
public enum BackendMode {
    RUST;

    public static BackendMode parse(final String raw) {
        if (raw == null || raw.isBlank()) {
            throw new IllegalArgumentException("squaremap only supports the Rust map backend");
        }
        final String normalized = raw.trim().toUpperCase(java.util.Locale.ROOT);
        if (!"RUST".equals(normalized)) {
            throw new IllegalArgumentException("squaremap only supports the Rust map backend; got " + raw);
        }
        return RUST;
    }
}
