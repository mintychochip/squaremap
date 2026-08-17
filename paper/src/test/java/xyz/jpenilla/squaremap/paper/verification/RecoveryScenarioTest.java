package xyz.jpenilla.squaremap.paper.verification;

import static org.junit.jupiter.api.Assertions.*;
import java.time.Duration;
import java.util.List;
import org.junit.jupiter.api.Test;

final class RecoveryScenarioTest {
    @Test void restartPolicyUsesBoundedExponentialDelays() {
        assertEquals(List.of(Duration.ofSeconds(1), Duration.ofSeconds(2), Duration.ofSeconds(4), Duration.ofSeconds(8), Duration.ofSeconds(16)), RustShadowSmokeDriver.restartDelays());
        assertEquals(5, RustShadowSmokeDriver.maxRestartStarts());
        assertEquals(Duration.ofMinutes(10), RustShadowSmokeDriver.restartWindow());
    }

    @Test void recoveryTranscriptRequiresHandshakeAndReplay() {
        RustShadowSmokeDriver.RecoveryRecord record = new RustShadowSmokeDriver.RecoveryRecord(2, true, true, true, true);
        assertTrue(record.complete());
        assertFalse(new RustShadowSmokeDriver.RecoveryRecord(2, true, false, true, true).complete());
    }
}
