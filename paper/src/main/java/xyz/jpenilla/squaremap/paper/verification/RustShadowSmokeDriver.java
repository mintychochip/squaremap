package xyz.jpenilla.squaremap.paper.verification;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.Objects;
import java.util.concurrent.TimeUnit;
import java.util.function.Consumer;
import xyz.jpenilla.squaremap.common.bridge.verification.ProofFixture;

/** Black-box local Paper proof driver; controls only the configured process channel. */
public final class RustShadowSmokeDriver {
    private static final List<Duration> RESTART_DELAYS = List.of(Duration.ofSeconds(1), Duration.ofSeconds(2), Duration.ofSeconds(4), Duration.ofSeconds(8), Duration.ofSeconds(16));
    private static final Duration RESTART_WINDOW = Duration.ofMinutes(10);
    private static final int MAX_RESTART_STARTS = 5;
    private final ProofFixture fixture;
    private final List<String> transcript = new ArrayList<>();

    public RustShadowSmokeDriver(final ProofFixture fixture) { this.fixture = Objects.requireNonNull(fixture, "fixture"); }
    public static List<Duration> restartDelays() { return RESTART_DELAYS; }
    public static Duration restartWindow() { return RESTART_WINDOW; }
    public static int maxRestartStarts() { return MAX_RESTART_STARTS; }
    public List<String> transcript() { return List.copyOf(transcript); }

    public void run(final List<String> command, final Duration timeout, final Consumer<String> control) throws IOException, InterruptedException {
        Objects.requireNonNull(command, "command");
        if (command.isEmpty()) throw new IllegalArgumentException("Paper command must not be empty");
        Process process = new ProcessBuilder(command).directory(fixture.roots().data().toFile()).redirectErrorStream(true).start();
        try (BufferedReader output = new BufferedReader(new InputStreamReader(process.getInputStream(), StandardCharsets.UTF_8))) {
            long deadline = System.nanoTime() + timeout.toNanos();
            String line;
            while (System.nanoTime() < deadline && (line = output.readLine()) != null) {
                transcript.add(line);
                if (control != null) control.accept(line);
            }
            if (process.isAlive()) {
                process.destroyForcibly();
                throw new IllegalStateException("Paper command timed out");
            }
            if (process.exitValue() != 0) throw new IllegalStateException("Paper command failed with exit " + process.exitValue());
        } finally {
            if (process.isAlive()) process.destroyForcibly();
            try { process.waitFor(1, TimeUnit.SECONDS); } catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); throw interrupted; }
        }
    }

    public record RecoveryRecord(int generation, boolean killObserved, boolean handshake, boolean baselineReplay, boolean converged) {
        public boolean complete() { return generation > 1 && killObserved && handshake && baselineReplay && converged; }
    }

    public static void main(final String[] args) throws Exception {
        if (args.length == 0) throw new IllegalArgumentException("usage: RustShadowSmokeDriver <fixture-manifest> [paper-command...]");
        ProofFixture fixture;
        try { fixture = ProofFixture.load(Path.of(args[0])); }
        catch (ProofFixture.BlockedFixtureArtifactsException blocked) { System.err.println(blocked.getMessage()); return; }
        if (args.length == 1) throw new IllegalArgumentException("Paper command is required");
        new RustShadowSmokeDriver(fixture).run(List.of(args).subList(1, args.length), Duration.ofMillis(fixture.timeoutsMs().get("lifecycle")), System.out::println);
    }
}
