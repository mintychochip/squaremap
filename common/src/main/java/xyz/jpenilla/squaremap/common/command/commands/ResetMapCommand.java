package xyz.jpenilla.squaremap.common.command.commands;

import com.google.inject.Inject;
import net.minecraft.server.level.ServerLevel;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.framework.qual.DefaultQualifier;
import org.incendo.cloud.context.CommandContext;
import org.incendo.cloud.processors.confirmation.ConfirmationManager;
import xyz.jpenilla.squaremap.common.backend.BackendController;
import xyz.jpenilla.squaremap.common.backend.BackendResult;
import xyz.jpenilla.squaremap.common.command.BackendCommandMessages;
import xyz.jpenilla.squaremap.common.command.Commander;
import xyz.jpenilla.squaremap.common.command.Commands;
import xyz.jpenilla.squaremap.common.command.SquaremapCommand;
import xyz.jpenilla.squaremap.common.config.Messages;
import xyz.jpenilla.squaremap.common.util.Components;
import xyz.jpenilla.squaremap.common.util.Util;

import static org.incendo.cloud.minecraft.extras.RichDescription.richDescription;
import static xyz.jpenilla.squaremap.common.command.argument.parser.LevelParser.levelParser;

@DefaultQualifier(NonNull.class)
public final class ResetMapCommand extends SquaremapCommand {
    private final BackendController backend;

    @Inject
    private ResetMapCommand(final Commands commands, final BackendController backend) {
        super(commands);
        this.backend = backend;
    }

    @Override
    public void register() {
        this.commands.registerSubcommand(builder -> builder.literal("resetmap")
            .required("world", levelParser())
            .commandDescription(richDescription(Messages.RESETMAP_COMMAND_DESCRIPTION))
            .meta(ConfirmationManager.META_CONFIRMATION_REQUIRED, true)
            .permission("squaremap.command.resetmap")
            .handler(this::executeResetMap));
    }

    private void executeResetMap(final CommandContext<Commander> context) {
        final Commander sender = context.sender();
        final ServerLevel world = context.get("world");
        this.backend.resetMap(Util.worldIdentifier(world)).whenComplete((result, failure) -> {
            final BackendResult outcome = failure == null ? result : BackendResult.of(BackendResult.Code.FAILED);
            if (outcome.code() == BackendResult.Code.MAP_RESET) {
                sender.sendMessage(Messages.SUCCESSFULLY_RESET_MAP.withPlaceholders(Components.worldPlaceholder(world)));
            } else {
                BackendCommandMessages.sendFailure(sender, outcome, Components.worldPlaceholder(world));
            }
        });
    }
}
