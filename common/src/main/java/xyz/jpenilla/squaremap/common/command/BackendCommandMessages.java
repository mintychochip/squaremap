package xyz.jpenilla.squaremap.common.command;

import net.kyori.adventure.text.minimessage.tag.resolver.TagResolver;
import org.checkerframework.checker.nullness.qual.Nullable;
import xyz.jpenilla.squaremap.common.backend.BackendResult;
import xyz.jpenilla.squaremap.common.config.Messages;

/** Sends localized messages for every non-success backend outcome. */
public final class BackendCommandMessages {
    private BackendCommandMessages() {}

    public static void sendFailure(final Commander sender, final BackendResult result, final @Nullable TagResolver world) {
        switch (result.code()) {
            case UNKNOWN_WORLD -> sender.sendMessage(world == null ? Messages.COMMAND_EXCEPTION_COMMAND_EXECUTION : Messages.NO_SUCH_WORLD.withPlaceholders(world));
            case INVALID_REQUEST -> sender.sendMessage(Messages.BACKEND_INVALID_REQUEST_MESSAGE);
            case INVALID_CONFIG -> sender.sendMessage(Messages.BACKEND_INVALID_CONFIG_MESSAGE);
            case BACKEND_UNAVAILABLE -> sender.sendMessage(Messages.BACKEND_UNAVAILABLE_MESSAGE);
            case BACKEND_TIMEOUT -> sender.sendMessage(Messages.BACKEND_TIMEOUT_MESSAGE);
            case FAILED -> sender.sendMessage(Messages.COMMAND_EXCEPTION_COMMAND_EXECUTION);
            case FULL_RENDER_STARTED, RADIUS_RENDER_STARTED, RENDER_IN_PROGRESS, RENDER_NOT_IN_PROGRESS,
                RENDER_CANCELLED, RENDERS_PAUSED, RENDERS_RESUMED, MAP_RESET, RELOADED, HEALTHY,
                PROGRESS_LOGGING_RESTARTED -> { }
        }
    }
}
