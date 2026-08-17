package xyz.jpenilla.squaremap.common.bridge.process;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

final class BackendModeLifecycleTest {
    @Test
    void rustIsTheConfiguredOwner() {
        assertEquals(BackendMode.RUST, BackendLifecyclePolicy.configuredMode());
        assertTrue(BackendLifecyclePolicy.rustHttpOwner(BackendMode.RUST));
    }
}
