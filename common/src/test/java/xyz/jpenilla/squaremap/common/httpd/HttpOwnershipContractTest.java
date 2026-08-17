package xyz.jpenilla.squaremap.common.httpd;

import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.common.bridge.process.BackendLifecyclePolicy;
import xyz.jpenilla.squaremap.common.bridge.process.BackendMode;

final class HttpOwnershipContractTest {
    @Test
    void rustOwnsTheMapHttpListener() {
        assertTrue(BackendLifecyclePolicy.rustHttpOwner(BackendMode.RUST));
        assertTrue(BackendLifecyclePolicy.rustHttpOwner(BackendLifecyclePolicy.configuredMode()));
    }
}
