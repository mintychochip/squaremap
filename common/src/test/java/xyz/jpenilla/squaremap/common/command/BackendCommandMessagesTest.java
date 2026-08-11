package xyz.jpenilla.squaremap.common.command;

import java.util.ArrayList;
import java.util.List;
import net.kyori.adventure.text.Component;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.common.backend.BackendResult;

import static org.junit.jupiter.api.Assertions.assertEquals;

final class BackendCommandMessagesTest {
    @Test
    void sendsUnavailableTimeoutAndFailureToPlayerAndConsole() {
        final RecordingCommander player = new RecordingCommander("player");
        final RecordingCommander console = new RecordingCommander("console");
        for (final BackendResult.Code code : List.of(BackendResult.Code.BACKEND_UNAVAILABLE,
            BackendResult.Code.BACKEND_TIMEOUT, BackendResult.Code.FAILED)) {
            BackendCommandMessages.sendFailure(player, BackendResult.of(code), null);
            BackendCommandMessages.sendFailure(console, BackendResult.of(code), null);
        }
        assertEquals(3, player.messages.size());
        assertEquals(3, console.messages.size());
    }

    private static final class RecordingCommander implements Commander {
        private final Object id;
        private final List<Component> messages = new ArrayList<>();

        private RecordingCommander(final Object id) { this.id = id; }
        @Override public boolean hasPermission(final String permission) { return true; }
        @Override public Object commanderId() { return this.id; }
        @Override public void sendMessage(final Component message) { this.messages.add(message); }
    }
}
