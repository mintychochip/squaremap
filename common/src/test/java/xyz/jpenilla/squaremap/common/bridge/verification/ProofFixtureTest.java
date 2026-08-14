package xyz.jpenilla.squaremap.common.bridge.verification;

import static org.junit.jupiter.api.Assertions.*;
import java.nio.file.*;
import org.junit.jupiter.api.Test;

final class ProofFixtureTest {
  private static final Path MANIFEST=Path.of("../testdata/bridge/v1/manifest.json");
  @Test void blocksPlaceholderArtifactsWithPrerequisites(){var e=assertThrows(ProofFixture.BlockedFixtureArtifactsException.class,()->ProofFixture.load(MANIFEST));assertTrue(e.getMessage().startsWith("blocked_fixture_artifacts"));assertEquals(3,e.prerequisites().size());}
  @Test void rejectsMissingArtifact() throws Exception {Path d=Files.createTempDirectory("proof-fixture");Path m=d.resolve("manifest.json");Files.writeString(m,Files.readString(MANIFEST).replace("hello.bin","missing.bin"));assertThrows(IllegalArgumentException.class,()->ProofFixture.load(m));}
  @Test void rejectsUnsupportedVersion() throws Exception {Path d=Files.createTempDirectory("proof-fixture");Path m=d.resolve("manifest.json");Files.writeString(m,Files.readString(MANIFEST).replace("\"version\": 1","\"version\": 2"));assertThrows(IllegalArgumentException.class,()->ProofFixture.load(m));}
  @Test void rejectsInvalidPortAndTimeout() throws Exception {String s=Files.readString(MANIFEST).replace("25575","25565");Path d=Files.createTempDirectory("proof-fixture");Path m=d.resolve("manifest.json");Files.writeString(m,s);assertThrows(IllegalArgumentException.class,()->ProofFixture.load(m));}
  @Test void rejectsLifecycleOrderDuplicateMissingAndNormalization() throws Exception {for(String replacement:new String[]{"[\"baseline\",\"readiness\"","[\"readiness\",\"readiness\"","[\"readiness\",\"baseline\",\"mutation\",\"churn\",\"reload\",\"cancel\",\"resume\",\"quiescence\",\"kill\",\"restart\",\"replay\",\"shutdown\"","\"normalization\": [\"not-declared\"]"}){Path d=Files.createTempDirectory("proof-fixture");Path m=d.resolve("manifest.json");String x=Files.readString(MANIFEST);if(replacement.startsWith("\"normalization"))x=x.replace("\"normalization\": []",replacement);else x=x.replaceFirst("\\[\\\"readiness\\\".*?\\]",replacement+"]");Files.writeString(m,x);assertThrows(IllegalArgumentException.class,()->ProofFixture.load(m));}}
  @Test void rejectsRootOverlapAndSymlinkAlias() throws Exception {Path d=Files.createTempDirectory("proof-fixture");Path m=d.resolve("manifest.json");String x=Files.readString(MANIFEST).replace("proof-rust-output","proof-java-output/nested");Files.writeString(m,x);assertThrows(IllegalArgumentException.class,()->ProofFixture.load(m));}
}
