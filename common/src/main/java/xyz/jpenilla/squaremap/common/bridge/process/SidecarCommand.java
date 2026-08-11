package xyz.jpenilla.squaremap.common.bridge.process;

import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Objects;

/** Immutable executable and fixed arguments for a managed sidecar. */
public final class SidecarCommand {
    private final List<String> command;

    public SidecarCommand(final List<String> command) {
        Objects.requireNonNull(command, "command");
        if (command.isEmpty()) {
            throw new IllegalArgumentException("sidecar command must not be empty");
        }
        final List<String> copy = new ArrayList<>(command.size());
        for (final String argument : command) {
            if (argument == null || argument.isBlank()) {
                throw new IllegalArgumentException("sidecar command arguments must not be blank");
            }
            copy.add(argument);
        }
        this.command = List.copyOf(copy);
    }

    public SidecarCommand(final Path executable, final List<String> arguments) {
        this(withExecutable(executable, arguments));
    }

    public SidecarCommand(final Path executable) {
        this(List.of(Objects.requireNonNull(executable, "executable").toString()));
    }

    public List<String> command() {
        return this.command;
    }

    public Path executable() {
        return Path.of(this.command.get(0));
    }

    public List<String> arguments() {
        return this.command.subList(1, this.command.size());
    }

    private static List<String> withExecutable(final Path executable, final List<String> arguments) {
        Objects.requireNonNull(executable, "executable");
        Objects.requireNonNull(arguments, "arguments");
        final List<String> command = new ArrayList<>(arguments.size() + 1);
        command.add(executable.toString());
        command.addAll(arguments);
        return command;
    }
}
