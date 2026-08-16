package xyz.jpenilla.squaremap.common.bridge.process;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

final class BackendModeLifecycleTest {
    @Test
    void javaAndShadowOwnJavaHttpLifecycle() {
        assertTrue(BackendLifecyclePolicy.javaHttpOwner(BackendMode.JAVA));
        assertTrue(BackendLifecyclePolicy.javaHttpOwner(BackendMode.SHADOW));
    }

    @Test
    void rustOwnsHttpLifecycleAndDoesNotTouchJavaCache() {
        assertFalse(BackendLifecyclePolicy.javaHttpOwner(BackendMode.RUST));
        assertFalse(BackendLifecyclePolicy.javaCacheOwner(BackendMode.RUST));
    }

    @Test
    void javaAndShadowOwnCacheLifecycle() {
        assertTrue(BackendLifecyclePolicy.javaCacheOwner(BackendMode.JAVA));
        assertTrue(BackendLifecyclePolicy.javaCacheOwner(BackendMode.SHADOW));
    }

    @Test
    void shadowDisablesRustHttpOwnership() {
        assertFalse(BackendLifecyclePolicy.rustHttpOwner(BackendMode.SHADOW));
        assertTrue(BackendLifecyclePolicy.rustHttpOwner(BackendMode.RUST));
    }

    @Test
    void shadowOwnershipPolicyDisablesRustHttpInExportContract() {
        assertFalse(BackendLifecyclePolicy.rustHttpOwner(BackendMode.SHADOW));
        assertTrue(BackendLifecyclePolicy.rustHttpOwner(BackendMode.RUST));
    }

    @Test
    void rustDoesNotOwnJavaDirtyOrRenderSchedulers() {
        assertFalse(BackendLifecyclePolicy.javaDirtyOwner(BackendMode.RUST));
        assertFalse(BackendLifecyclePolicy.javaRenderOwner(BackendMode.RUST));
        assertTrue(BackendLifecyclePolicy.javaDirtyOwner(BackendMode.JAVA));
        assertTrue(BackendLifecyclePolicy.javaRenderOwner(BackendMode.SHADOW));
    }
}
