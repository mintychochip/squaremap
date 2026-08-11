package xyz.jpenilla.squaremap.common.command.commands;

import com.google.inject.Inject;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.framework.qual.DefaultQualifier;
import org.incendo.cloud.context.CommandContext;
import xyz.jpenilla.squaremap.common.backend.BackendController;
import xyz.jpenilla.squaremap.common.backend.BackendResult;
import xyz.jpenilla.squaremap.common.command.Commander;
import xyz.jpenilla.squaremap.common.command.Commands;
import xyz.jpenilla.squaremap.common.command.BackendCommandMessages;
import xyz.jpenilla.squaremap.common.command.SquaremapCommand;
import xyz.jpenilla.squaremap.common.config.Messages;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;
import xyz.jpenilla.squaremap.common.util.CommandUtil;
import xyz.jpenilla.squaremap.common.util.Components;

import static org.incendo.cloud.minecraft.extras.RichDescription.richDescription;
import static xyz.jpenilla.squaremap.common.command.argument.parser.MapWorldParser.mapWorldParser;

@DefaultQualifier(NonNull.class)
public final class CancelRenderCommand extends SquaremapCommand {
    private final BackendController backend;

    @Inject
    private CancelRenderCommand(final Commands commands, final BackendController backend) {
        super(commands);
        this.backend = backend;
    }

    @Override
    public void register() {
        this.commands.registerSubcommand(builder -> builder.literal("cancelrender")
            .optional("world", mapWorldParser(), richDescription(Messages.OPTIONAL_WORLD_ARGUMENT_DESCRIPTION))
            .commandDescription(richDescription(Messages.CANCEL_RENDER_COMMAND_DESCRIPTION))
            .permission("squaremap.command.cancelrender")
            .handler(this::executeCancelRender));
    }

    private void executeCancelRender(final CommandContext<Commander> context) {
        final Commander sender = context.sender();
        final MapWorldInternal world = CommandUtil.resolveWorld(context);
        this.backend.cancelRender(world.identifier()).whenComplete((result, failure) -> {
            final BackendResult outcome = failure == null ? result : BackendResult.of(BackendResult.Code.FAILED);
            switch (outcome.code()) {
                case RENDER_CANCELLED -> sender.sendMessage(Messages.CANCELLED_RENDER.withPlaceholders(Components.worldPlaceholder(world)));
                case RENDER_NOT_IN_PROGRESS -> sender.sendMessage(Messages.RENDER_NOT_IN_PROGRESS.withPlaceholders(Components.worldPlaceholder(world)));
                default -> BackendCommandMessages.sendFailure(sender, outcome, Components.worldPlaceholder(world));
            }
        });
    }
}
