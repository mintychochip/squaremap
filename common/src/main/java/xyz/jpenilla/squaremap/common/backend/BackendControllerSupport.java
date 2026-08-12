package xyz.jpenilla.squaremap.common.backend;

import com.google.inject.Inject;
import com.google.inject.Singleton;
import java.util.concurrent.CompletionStage;

/** Internal adapter for configuration-only commands; map controls use BackendController directly. */
@Singleton
public final class BackendControllerSupport {
    private final BackendController controller;

    @Inject
    public BackendControllerSupport(final BackendController controller) {
        this.controller = controller;
    }

    public CompletionStage<BackendResult> publishConfig() {
        return this.controller.publishConfig();
    }

    public CompletionStage<BackendResult> restartProgressLogging() {
        return this.controller.restartProgressLogging();
    }
    public void close() {
        if (this.controller instanceof AutoCloseable closeable) {
            try {
                closeable.close();
            } catch (final Exception failure) {
                throw new IllegalStateException("failed to close backend controller", failure);
            }
        }
    }
    public void abortForRestart() {
        this.controller.abortForRestart();
    }
}
