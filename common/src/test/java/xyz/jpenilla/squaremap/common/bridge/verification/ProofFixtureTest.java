package xyz.jpenilla.squaremap.common.bridge.verification;

import static org.junit.jupiter.api.Assertions.*;
import java.nio.charset.StandardCharsets;
import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.nio.file.attribute.PosixFilePermissions;
import java.nio.file.*;
import java.security.MessageDigest;
import java.util.HexFormat;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;
import org.junit.jupiter.api.Test;

final class ProofFixtureTest {
  private static final Path MANIFEST=Path.of("../testdata/bridge/v1/paper-fixture.json");
  private static final String PLACEHOLDER_SHA="8cdaa308c88a17fe9aa8c12c96a09f7d3200b33c878b5b948c595c46d65c1378";
  @Test void validatesRustToolchainAndCatalogCompatibleVersions() throws Exception {Path d=Files.createTempDirectory("proof-fixture");Path m=stageManifest(d);String x=unblock(Files.readString(m));x=x.replace("\"rust_toolchain\": \"1.88.0\"","\"rust_toolchain\": \"1.87.0\"");Files.writeString(m,x);var e=assertThrows(IllegalArgumentException.class,()->ProofFixture.load(m));assertFalse(e.getMessage().isBlank());}
  @Test void rejectsTopLevelArtifactHashDisagreement() throws Exception {Path d=Files.createTempDirectory("proof-fixture");Path m=stageManifest(d);String x=Files.readString(m).replaceFirst("(\"paper\": \\{\"version\": \"[^\"]+\", \"url\": \"[^\"]+\", \"sha256\": \")[0-9a-f]+", "$1"+"0".repeat(64));Files.writeString(m,x);assertThrows(RuntimeException.class,()->ProofFixture.load(m));}
  @Test void blocksPlaceholderArtifactsWithPrerequisites() throws Exception {Path d=Files.createTempDirectory("proof-fixture");Path m=stageManifest(d);String x=Files.readString(m).replace("https://example.test/","https://fill.invalid/");Files.writeString(m,x);var e=assertThrows(ProofFixture.BlockedFixtureArtifactsException.class,()->ProofFixture.load(m));assertTrue(e.getMessage().startsWith("blocked_fixture_artifacts"));assertEquals(4,e.prerequisites().size());assertTrue(e.prerequisites().get(3).contains("complete Paper world corpus"));}
  @Test void rejectsMissingArtifact() throws Exception {Path d=Files.createTempDirectory("proof-fixture");Path m=stageManifest(d);String x=Files.readString(m).replace("paper.jar","missing.jar");Files.writeString(m,unblock(x));var e=assertThrows(IllegalArgumentException.class,()->ProofFixture.load(m));assertFalse(e.getMessage().isBlank());}
  @Test void rejectsHashMismatch() throws Exception {Path d=Files.createTempDirectory("proof-fixture");Path m=stageManifest(d);String x=Files.readString(m).replaceFirst(PLACEHOLDER_SHA,"0000000000000000000000000000000000000000000000000000000000000000");Files.writeString(m,unblock(x));var e=assertThrows(IllegalArgumentException.class,()->ProofFixture.load(m));assertFalse(e.getMessage().isBlank());}
  @Test void rejectsUnsupportedVersion() throws Exception {Path d=Files.createTempDirectory("proof-fixture");Path m=stageManifest(d);Files.writeString(m,Files.readString(m).replace("\"version\": 1","\"version\": 2"));var e=assertThrows(IllegalArgumentException.class,()->ProofFixture.load(m));assertEquals("unsupported fixture version: 2",e.getMessage());}
  @Test void rejectsRootOverlapAndExistingTargetSymlinkAlias() throws Exception {Path d=Files.createTempDirectory("proof-fixture");Path m=stageManifest(d);String x=unblock(Files.readString(m));x=x.replace("proof-rust-output","proof-java-output/nested");x=replaceAllHashes(x,PLACEHOLDER_SHA);Files.writeString(m,x);var overlap=assertThrows(IllegalArgumentException.class,()->ProofFixture.load(m));assertEquals("fixture roots overlap",overlap.getMessage());Files.createDirectories(d.resolve("proof-java-output"));Path alias=d.resolve("alias");Files.createSymbolicLink(alias,d.resolve("proof-java-output"));x=unblock(Files.readString(m)).replace("proof-rust-output","alias");x=x.replace("proof-java-output","proof-java-output/nested");x=replaceAllHashes(x,PLACEHOLDER_SHA);Files.writeString(m,x);var symlink=assertThrows(IllegalArgumentException.class,()->ProofFixture.load(m));assertEquals("fixture roots overlap",symlink.getMessage());}
  @Test void loadsValidTemporaryArtifacts() throws Exception {Path d=Files.createTempDirectory("proof-fixture");Path m=stageManifest(d);byte[] paper=zip("paper/plugin.yml"),plugin=zip("plugin.yml"),sidecar=elf();Files.write(d.resolve("paper.jar"),paper);Files.write(d.resolve("plugin.jar"),plugin);Path sidecarPath=d.resolve("squaremap-server");Files.write(sidecarPath,sidecar);sidecarPath.toFile().setExecutable(true);byte[] level="level.dat: minimal".getBytes(StandardCharsets.UTF_8),region="MCA".getBytes(StandardCharsets.UTF_8);Files.write(d.resolve("level.dat"),level);Files.createDirectories(d.resolve("region"));Files.write(d.resolve("region/r.0.0.mca"),region);String inventory="{\"version\":1,\"files\":[{\"path\":\"level.dat\",\"sha256\":\""+sha256(level)+"\"},{\"path\":\"region/r.0.0.mca\",\"sha256\":\""+sha256(region)+"\"}]}";Files.writeString(d.resolve("world-corpus.inventory.json"),inventory);String x=unblock(Files.readString(m));x=replaceHash(x,"paper.jar",sha256(paper));x=replaceHash(x,"plugin.jar",sha256(plugin));x=replaceHash(x,"squaremap-server",sha256(sidecar));x=replaceTopHash(x,"paper",sha256(paper));x=replaceTopHash(x,"plugin",sha256(plugin));x=replaceTopHash(x,"sidecar",sha256(sidecar));x=x.replace("\"world_corpus\": {\"inventory\": \"world-corpus.inventory.json\", \"sha256\": \"0000000000000000000000000000000000000000000000000000000000000000\"}","\"world_corpus\": {\"inventory\": \"world-corpus.inventory.json\", \"sha256\": \""+sha256(inventory.getBytes(StandardCharsets.UTF_8))+"\"}");Files.writeString(m,x);assertDoesNotThrow(()->ProofFixture.load(m));}
  @Test void rejectsVolatileHeadersNormalization() throws Exception {Path d=Files.createTempDirectory("proof-fixture");Path m=stageManifest(d);String x=unblock(Files.readString(m));x=withArtifacts(d,x);x=withCorpus(d,x);x=x.replace("\"normalization\": []","\"normalization\": [\"volatile_headers\"]");Files.writeString(m,x);assertEquals("undeclared normalization: volatile_headers",assertThrows(IllegalArgumentException.class,()->ProofFixture.load(m)).getMessage());}
  @Test void rejectsCatalogVersionAndUrlInconsistency() throws Exception {Path d=Files.createTempDirectory("proof-fixture");Path m=stageManifest(d);String x=unblock(Files.readString(m));x=x.replace("\"server\": \"Paper 26.2\"","\"server\": \"Paper 26.1\"");Files.writeString(m,x);assertThrows(IllegalArgumentException.class,()->ProofFixture.load(m));x=unblock(Files.readString(m)).replace("https://fill.invalid/paper-26.2.jar","https://fill.invalid/paper-26.1.jar");Files.writeString(m,x);assertThrows(IllegalArgumentException.class,()->ProofFixture.load(m));}
  @Test void rejectsMutationOfLoadedCollectionsAndCanonicalizesRoots() throws Exception {Path d=Files.createTempDirectory("proof-fixture");Path m=stageManifest(d);String x=unblock(Files.readString(m));x=withArtifacts(d,x);x=withCorpus(d,x);Files.writeString(m,x);var f=assertDoesNotThrow(()->ProofFixture.load(m));assertTrue(f.roots().data().isAbsolute());assertThrows(UnsupportedOperationException.class,()->f.scenario().add("x"));assertThrows(UnsupportedOperationException.class,()->f.worldCorpus().files().add("x"));}
  private static byte[] zip(String name)throws Exception{ByteArrayOutputStream bytes=new ByteArrayOutputStream();try(ZipOutputStream zip=new ZipOutputStream(bytes)){zip.putNextEntry(new ZipEntry(name));zip.write("name: squaremap\n".getBytes(StandardCharsets.UTF_8));zip.closeEntry();}return bytes.toByteArray();}
  private static byte[] elf()throws Exception{ByteArrayOutputStream bytes=new ByteArrayOutputStream();try(DataOutputStream out=new DataOutputStream(bytes)){out.write(new byte[]{0x7f,'E','L','F',2,1,1,0});out.write(new byte[56]);}return bytes.toByteArray();}
  private static String sha256(byte[] bytes)throws Exception{return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));}
  private static String replaceHash(String x,String path,String hash){return switch(path){case "paper.jar"->x.replaceFirst("\"paper\": \\{\"path\": \"paper.jar\", \"sha256\": \"[0-9a-f]+\"\\}", "\"paper\": {\"path\": \"paper.jar\", \"sha256\": \""+hash+"\"}");case "plugin.jar"->x.replaceFirst("\"plugin\": \\{\"path\": \"plugin.jar\", \"sha256\": \"[0-9a-f]+\"\\}", "\"plugin\": {\"path\": \"plugin.jar\", \"sha256\": \""+hash+"\"}");default->x.replaceFirst("\"sidecar\": \\{\"path\": \"squaremap-server\", \"sha256\": \"[0-9a-f]+\"\\}", "\"sidecar\": {\"path\": \"squaremap-server\", \"sha256\": \""+hash+"\"}");};}
  private static String replaceTopHash(String x,String kind,String hash){return x.replaceFirst("\""+kind+"\": \\{\"version\": \"[^\"]+\", \"url\": \"[^\"]+\", \"sha256\": \"[0-9a-f]+\"\\}","\""+kind+"\": {\"version\": \""+(kind.equals("paper")?"paper-26.2":kind.equals("plugin")?"squaremap-local-26.2":"squaremap-rust-local-26.2")+"\", \"url\": \"https://example.test/"+kind+"-26.2\", \"sha256\": \""+hash+"\"}");}
  private static String withCorpus(Path d,String x)throws Exception{byte[] level="level.dat: minimal".getBytes(StandardCharsets.UTF_8),region="MCA".getBytes(StandardCharsets.UTF_8);Files.write(d.resolve("level.dat"),level);Files.createDirectories(d.resolve("region"));Files.write(d.resolve("region/r.0.0.mca"),region);String inventory="{\"version\":1,\"files\":[{\"path\":\"level.dat\",\"sha256\":\""+sha256(level)+"\"},{\"path\":\"region/r.0.0.mca\",\"sha256\":\""+sha256(region)+"\"}]}";Files.writeString(d.resolve("world-corpus.inventory.json"),inventory);return x.replace("\"world_corpus\": {\"inventory\": \"world-corpus.inventory.json\", \"sha256\": \"0000000000000000000000000000000000000000000000000000000000000000\"}","\"world_corpus\": {\"inventory\": \"world-corpus.inventory.json\", \"sha256\": \""+sha256(inventory.getBytes(StandardCharsets.UTF_8))+"\"}");}
  private static String withArtifacts(Path d,String x)throws Exception{byte[] paper=zip("paper/plugin.yml"),plugin=zip("plugin.yml"),sidecar=elf();Files.write(d.resolve("paper.jar"),paper);Files.write(d.resolve("plugin.jar"),plugin);Path sidecarPath=d.resolve("squaremap-server");Files.write(sidecarPath,sidecar);sidecarPath.toFile().setExecutable(true);x=replaceHash(x,"paper.jar",sha256(paper));x=replaceHash(x,"plugin.jar",sha256(plugin));x=replaceHash(x,"squaremap-server",sha256(sidecar));return replaceTopHash(replaceTopHash(replaceTopHash(x,"paper",sha256(paper)),"plugin",sha256(plugin)),"sidecar",sha256(sidecar));}
  private static String unblock(String x){return x.replace("https://fill.invalid/","https://example.test/").replace("\"artifact_prerequisites\": [\n      \"provide a real Paper server jar for paper.path\",\n      \"provide a real squaremap plugin jar for plugin.path\",\n      \"provide a real squaremap sidecar executable for sidecar.path\",\n      \"generate and provide a complete Paper world corpus for world_fixture.path and world_corpus.inventory\"\n    ],","\"artifact_prerequisites\": [],");}
  private static String replaceAllHashes(String x,String hash){return x.replaceAll("\"sha256\": \"[0-9a-f]+\"","\"sha256\": \""+hash+"\"");}
  private static Path stageManifest(Path d)throws Exception {Files.copy(MANIFEST,d.resolve("manifest.json"));for(String name:new String[]{"paper.jar","plugin.jar","squaremap-server"})Files.copy(Path.of("../testdata/bridge/v1/"+name),d.resolve(name));Files.copy(Path.of("../testdata/bridge/v1/world-fixture.json"),d.resolve("world-fixture.json"));return d.resolve("manifest.json");}
}

final class QuiescenceProbeTest {
  @Test
  void returnsOnlyAfterTwoEqualCompleteSamples() {
    final var sample = QuiescenceSnapshot.complete("hash", 7L);
    final var calls = new java.util.concurrent.atomic.AtomicInteger();
    final var observed = new java.util.concurrent.atomic.AtomicLong();
    final var probe = new QuiescenceProbe(() -> {
      if (calls.incrementAndGet() == 2) observed.set(System.nanoTime());
      return sample;
    });
    final long start = System.nanoTime();
    assertEquals(sample, probe.await(java.time.Duration.ofSeconds(1), java.time.Duration.ofMillis(10)));
    assertTrue(observed.get() - start >= java.time.Duration.ofMillis(10).toNanos());
    assertTrue(calls.get() >= 2);
  }

  @Test
  void rejectsMissingObservation() {
    final var probe = new QuiescenceProbe(() -> QuiescenceSnapshot.incomplete("metrics unavailable"));
    assertThrows(IllegalStateException.class,
        () -> probe.await(java.time.Duration.ofMillis(10), java.time.Duration.ZERO));
  }
}
