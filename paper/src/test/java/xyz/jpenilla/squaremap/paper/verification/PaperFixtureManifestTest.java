package xyz.jpenilla.squaremap.paper.verification;

import static org.junit.jupiter.api.Assertions.*;
import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.common.bridge.verification.ProofFixture;

final class PaperFixtureManifestTest {
    @Test void fixtureLoaderFailsClosedBeforeLaunchWhenArtifactsArePlaceholders() throws Exception {
        Path manifest = Path.of("../testdata/bridge/v1/paper-fixture.json");
        assertThrows(RuntimeException.class, () -> ProofFixture.load(manifest));
    }

    @Test void requiredScenarioIsExplicitAndOrdered() throws Exception {
        String json = Files.readString(Path.of("../testdata/bridge/v1/paper-fixture.json"));
        for (String step : new String[]{"readiness", "mutation", "reload", "cancel", "resume", "kill", "restart", "replay", "shutdown"}) {
            assertTrue(json.contains("\"" + step + "\""));
        }
    }
}
