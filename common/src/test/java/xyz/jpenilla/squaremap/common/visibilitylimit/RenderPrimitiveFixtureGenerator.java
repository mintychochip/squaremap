package xyz.jpenilla.squaremap.common.visibilitylimit;

import com.google.gson.JsonObject;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.condition.EnabledIfSystemProperty;

/** Explicit opt-in generator; normal oracle tests never write fixtures. */
final class RenderPrimitiveFixtureGenerator {
    private RenderPrimitiveFixtureGenerator() {}

    @Test
    @EnabledIfSystemProperty(named = "squaremap.regenerate", matches = "--overwrite")
    void overwriteFixture() throws Exception {
        write();
    }

    public static void main(final String[] args) throws Exception {
        write();
    }

    private static void write() throws Exception {
        if (!"--overwrite".equals(System.getProperty("squaremap.regenerate"))) {
            throw new IllegalArgumentException("pass -Dsquaremap.regenerate=--overwrite");
        }
        final Path fixture = RenderPrimitiveOracleTest.FIXTURE;
        final JsonObject document = RenderPrimitiveOracleTest.document();
        Files.createDirectories(fixture.getParent());
        final Path temporary = fixture.resolveSibling(fixture.getFileName() + ".tmp");
        try {
            Files.writeString(temporary, document.toString());
            Files.move(temporary, fixture, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
        } finally {
            Files.deleteIfExists(temporary);
        }
    }
}
