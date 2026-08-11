package xyz.jpenilla.squaremap.common.command.commands;

import com.google.inject.Inject;
import net.minecraft.core.BlockPos;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.checker.nullness.qual.Nullable;
import org.checkerframework.framework.qual.DefaultQualifier;
import org.incendo.cloud.context.CommandContext;
import xyz.jpenilla.squaremap.common.backend.BackendController;
import xyz.jpenilla.squaremap.common.backend.BackendResult;
import xyz.jpenilla.squaremap.common.command.BackendCommandMessages;
import xyz.jpenilla.squaremap.common.command.Commander;
import xyz.jpenilla.squaremap.common.command.Commands;
import xyz.jpenilla.squaremap.common.command.PlatformCommands;
import xyz.jpenilla.squaremap.common.command.SquaremapCommand;
import xyz.jpenilla.squaremap.common.config.Messages;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;
import xyz.jpenilla.squaremap.common.util.Components;

import static org.incendo.cloud.minecraft.extras.RichDescription.richDescription;
import static org.incendo.cloud.parser.standard.IntegerParser.integerParser;
import static xyz.jpenilla.squaremap.common.command.argument.parser.MapWorldParser.mapWorldParser;

@DefaultQualifier(NonNull.class)
public final class RadiusRenderCommand extends SquaremapCommand {
    private final PlatformCommands platformCommands;
    private final BackendController backend;

    @Inject
    private RadiusRenderCommand(final Commands commands, final PlatformCommands platformCommands, final BackendController backend) {
        super(commands);
        this.platformCommands = platformCommands;
        this.backend = backend;
    }

    @Override
    public void register() {
        this.commands.registerSubcommand(builder -> builder.literal("radiusrender")
            .required("world", mapWorldParser())
            .required("radius", integerParser(1))
            .optional("center", this.platformCommands.columnPosParser(), richDescription(Messages.OPTIONAL_CENTER_ARGUMENT_DESCRIPTION))
            .commandDescription(richDescription(Messages.RADIUSRENDER_COMMAND_DESCRIPTION))
            .permission("squaremap.command.radiusrender")
            .handler(this::executeRadiusRender));
    }

    private void executeRadiusRender(final CommandContext<Commander> context) {
        final Commander sender = context.sender();
        final MapWorldInternal world = context.get("world");
        final int radius = context.get("radius");
        @Nullable BlockPos center = context.<BlockPos>optional("center").orElse(null);
        if (center == null) center = new BlockPos(0, 0, 0);
        final BlockPos requestedCenter = center;
        this.backend.radiusRender(world.identifier(), requestedCenter.getX(), requestedCenter.getZ(), radius).whenComplete((result, failure) -> {
            final BackendResult outcome = failure == null ? result : BackendResult.of(BackendResult.Code.FAILED);
            switch (outcome.code()) {
                case RADIUS_RENDER_STARTED -> sender.sendMessage(Components.miniMessage(Messages.LOG_STARTED_RADIUSRENDER, Components.worldPlaceholder(world)));
                case RENDER_IN_PROGRESS -> sender.sendMessage(Messages.RENDER_IN_PROGRESS.withPlaceholders(Components.worldPlaceholder(world)));
                default -> BackendCommandMessages.sendFailure(sender, outcome, Components.worldPlaceholder(world));
            }
        });
    }
}
