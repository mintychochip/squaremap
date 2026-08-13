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
}
