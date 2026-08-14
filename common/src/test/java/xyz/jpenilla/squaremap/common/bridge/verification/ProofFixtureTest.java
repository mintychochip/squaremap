package xyz.jpenilla.squaremap.common.bridge.verification;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;

final class ProofFixtureTest {
    private static final Path MANIFEST = Path.of("../testdata/bridge/v1/manifest.json");

    @Test
    void loadsPinnedPaperFixture() throws Exception {
        final ProofFixture fixture = ProofFixture.load(MANIFEST);
        assertEquals(1, fixture.version());
        assertEquals("paper-1.21.8", fixture.paper().version());
        assertEquals("1.21.8", fixture.minecraftVersion());
        assertEquals(13, fixture.scenario().size());
    }

    @Test
    void rejectsMissingArtifact() throws Exception {
        final Path copy = Files.createTempDirectory("proof-fixture");
        final Path manifest = copy.resolve("manifest.json");
        Files.writeString(manifest, Files.readString(MANIFEST).replace("\"hello.bin\"", "\"missing.bin\""));
        assertThrows(IllegalArgumentException.class, () -> ProofFixture.load(manifest));
    }
}
