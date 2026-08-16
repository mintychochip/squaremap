package xyz.jpenilla.squaremap.common.httpd;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.common.bridge.process.BackendLifecyclePolicy;
import xyz.jpenilla.squaremap.common.bridge.process.BackendMode;

final class HttpOwnershipContractTest {
    @Test
    void backendModesAssignExclusiveHttpOwnership() {
        assertTrue(BackendLifecyclePolicy.javaHttpOwner(BackendMode.JAVA));
        assertTrue(BackendLifecyclePolicy.javaHttpOwner(BackendMode.SHADOW));
        assertFalse(BackendLifecyclePolicy.javaHttpOwner(BackendMode.RUST));
        assertFalse(BackendLifecyclePolicy.rustHttpOwner(BackendMode.SHADOW));
        assertTrue(BackendLifecyclePolicy.rustHttpOwner(BackendMode.RUST));
        assertFalse(BackendLifecyclePolicy.javaDirtyOwner(BackendMode.RUST));
        assertFalse(BackendLifecyclePolicy.javaRenderOwner(BackendMode.RUST));
        assertTrue(BackendLifecyclePolicy.javaDirtyOwner(BackendMode.JAVA));
        assertTrue(BackendLifecyclePolicy.javaRenderOwner(BackendMode.SHADOW));
    }
}
