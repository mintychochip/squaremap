package xyz.jpenilla.squaremap.common.backend;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.List;
import java.util.concurrent.CompletableFuture;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.api.WorldIdentifier;

final class ExclusiveRustOwnershipTest {
    @Test
    void backendControllerOnlyEmitsToTheBridge() {
        final java.util.List<BackendController.BackendRequest> bridge = new java.util.ArrayList<>();
        final BackendController controller = new BackendController(
            request -> {
                bridge.add(request);
                return CompletableFuture.completedFuture(BackendResult.of(BackendResult.Code.HEALTHY));
            }
        );
        final WorldIdentifier world = WorldIdentifier.create("minecraft", "overworld");
        controller.fullRender(world);
        controller.health();
        assertEquals(2, bridge.size());
    }

    @Test
    void dualBackendOwnershipTypesAreNotOnTheClasspath() {
        for (final String type : List.of(
            "xyz.jpenilla.squaremap.common.httpd.IntegratedServer",
            "xyz.jpenilla.squaremap.common.httpd.JsonCache",
            "xyz.jpenilla.squaremap.common.httpd.ViteRunner",
            "xyz.jpenilla.squaremap.common.backend.LegacyBackendController",
            "xyz.jpenilla.squaremap.common.data.RenderManager"
        )) {
            final ClassNotFoundException missing = assertThrows(ClassNotFoundException.class, () -> Class.forName(type));
            assertTrue(missing.getMessage().contains(type));
        }
    }
}
