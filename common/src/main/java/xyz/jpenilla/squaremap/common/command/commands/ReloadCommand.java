package xyz.jpenilla.squaremap.common.command.commands;

import com.google.inject.Inject;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.framework.qual.DefaultQualifier;
import org.incendo.cloud.context.CommandContext;
import xyz.jpenilla.squaremap.common.backend.BackendController;
import xyz.jpenilla.squaremap.common.backend.BackendResult;
import xyz.jpenilla.squaremap.common.command.Commander;
import xyz.jpenilla.squaremap.common.command.BackendCommandMessages;
import xyz.jpenilla.squaremap.common.command.Commands;
import xyz.jpenilla.squaremap.common.command.SquaremapCommand;
import xyz.jpenilla.squaremap.common.config.Messages;
import xyz.jpenilla.squaremap.common.util.Components;

import static org.incendo.cloud.minecraft.extras.RichDescription.richDescription;

@DefaultQualifier(NonNull.class)
public final class ReloadCommand extends SquaremapCommand {
    private final BackendController backend;

    @Inject
    private ReloadCommand(final Commands commands, final BackendController backend) {
        super(commands);
        this.backend = backend;
    }

    @Override
    public void register() {
        this.commands.registerSubcommand(builder -> builder.literal("reload")
            .commandDescription(richDescription(Messages.RELOAD_COMMAND_DESCRIPTION))
            .permission("squaremap.command.reload")
            .handler(this::execute));
    }

    public void execute(final CommandContext<Commander> context) {
        this.backend.reload().whenComplete((result, failure) -> {
            final BackendResult outcome = failure == null ? result : BackendResult.of(BackendResult.Code.FAILED);
            if (outcome.code() != BackendResult.Code.RELOADED) {
                BackendCommandMessages.sendFailure(context.sender(), outcome, null);
                return;
            }
            String version = "unknown";
            for (final BackendResult.Substitution substitution : outcome.substitutions()) {
                if (substitution.key().equals("version") && substitution.value() instanceof BackendResult.Text text) version = text.value();
            }
            context.sender().sendMessage(Messages.PLUGIN_RELOADED.withPlaceholders(
                Components.placeholder("name", "squaremap"), Components.placeholder("version", version)));
        });
    }
}
