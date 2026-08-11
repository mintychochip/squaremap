package xyz.jpenilla.squaremap.common.bridge.state;

import com.google.protobuf.ByteString;
import com.google.inject.Inject;
import com.google.inject.Provider;
import java.util.ArrayList;
import java.util.List;
import net.kyori.adventure.text.flattener.ComponentFlattener;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.tags.ItemTags;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.ai.attributes.AttributeInstance;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.level.GameType;
import net.minecraft.world.phys.Vec3;
import org.checkerframework.checker.nullness.qual.Nullable;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.bridge.v1.Player;
import xyz.jpenilla.squaremap.bridge.v1.PlayersReplace;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;
import xyz.jpenilla.squaremap.common.AbstractPlayerManager;
import xyz.jpenilla.squaremap.common.ServerAccess;
import xyz.jpenilla.squaremap.common.config.ConfigManager;
import xyz.jpenilla.squaremap.common.config.WorldConfig;
import xyz.jpenilla.squaremap.common.util.Util;
import xyz.jpenilla.squaremap.api.HtmlComponentSerializer;

/** Collects public player state into an immutable protocol snapshot. */
public final class PlayerStateExporter {
    private final Provider<ComponentFlattener> flattener;
    private final AbstractPlayerManager playerManager;
    private final ServerAccess serverAccess;
    private final ConfigManager configManager;
    private final WorldEpochRegistry epochs;
    private PlayersReplace last;
    private final BridgeRevisionClock revisions;
    @Inject
    public PlayerStateExporter(
        final Provider<ComponentFlattener> flattener,
        final AbstractPlayerManager playerManager,
        final ServerAccess serverAccess,
        final ConfigManager configManager,
        final WorldEpochRegistry epochs,
        final BridgeRevisionClock revisions
    ) {
        this.flattener = flattener;
        this.playerManager = playerManager;
        this.serverAccess = serverAccess;
        this.configManager = configManager;
        this.epochs = epochs;
        this.revisions = revisions;
    }

    public PlayersReplace export() {
        final HtmlComponentSerializer serializer = HtmlComponentSerializer.withFlattener(this.flattener.get());
        final List<Input> inputs = new ArrayList<>();
        this.serverAccess.levels().forEach(world -> {
            final WorldConfig config = this.configManager.worldConfig(world);
            final WorldIdentifier identifier = WorldIdentifier.create(
                world.dimension().identifier().getNamespace(),
                world.dimension().identifier().getPath()
            );
            final long epoch = this.epochs.epoch(identifier, world);
            world.players().forEach(player -> {
                final Vec3 pos = player.position();
                inputs.add(new Input(
                    player.getGameProfile().name(),
                    player.getUUID().toString().replace("-", ""),
                    identifier.namespace(),
                    identifier.value(),
                    epoch,
                    Mth.floor(pos.x()),
                    Mth.floor(pos.y()),
                    Mth.floor(pos.z()),
                    Math.round(player.getYHeadRot()),
                    config.PLAYER_TRACKER_USE_DISPLAY_NAME ? serializer.serialize(this.playerManager.displayName(player)) : null,
                    config.PLAYER_TRACKER_NAMEPLATE_SHOW_ARMOR ? armorPoints(player) : null,
                    config.PLAYER_TRACKER_NAMEPLATE_SHOW_HEALTH ? (int) player.getHealth() : null,
                    config.PLAYER_TRACKER_ENABLED,
                    config.PLAYER_TRACKER_HIDE_SPECTATORS,
                    config.PLAYER_TRACKER_HIDE_INVISIBLE,
                    config.PLAYER_TRACKER_HIDE_MAP_INVISIBILITY_EQUIPMENT,
                    player.gameMode.getGameModeForPlayer() == GameType.SPECTATOR,
                    player.isInvisible(),
                    hasMapInvisibilityItemEquipped(player),
                    this.playerManager.hidden(player),
                    this.playerManager.otherwiseHidden(player),
                    config.PLAYER_TRACKER_USE_DISPLAY_NAME
                ));
            });
        });
        return snapshot(inputs, this.serverAccess.maxPlayers(), this.revisions);
    }

    static PlayersReplace snapshot(final List<Input> inputs, final int maxPlayers, final BridgeRevisionClock revisions) {
        final List<Player> players = new ArrayList<>();
        for (final Input input : inputs) {
            if (input.spectator() && input.hideSpectators()
                || input.invisible() && input.hideInvisible()
                || input.mapInvisibilityEquipment() && input.hideMapInvisibilityEquipment()
                || input.hidden() || input.otherwiseHidden()) {
                continue;
            }
            final Player.Builder builder = Player.newBuilder()
                .setName(input.name())
                .setUuid(uuidBytes(input.uuid()))
                .setWorld(WorldIdentity.newBuilder().setNamespace(input.namespace()).setValue(input.world()).setEpoch(input.epoch()));
            if (input.useDisplayName() && input.displayName() != null) builder.setDisplayName(input.displayName());
            if (input.enabled()) {
                builder.setX(input.x()).setY(input.y()).setZ(input.z()).setYaw(input.yaw());
                if (input.armor() != null) builder.setArmor(input.armor());
                if (input.health() != null) builder.setHealth(input.health());
            }
            players.add(builder.build());
        }
        final PlayersReplace candidate = PlayersReplace.newBuilder()
            .addAllPlayers(players.stream().sorted(java.util.Comparator.comparing(player -> hex(player.getUuid().toByteArray()))).toList())
            .setRevision(0).setMaxPlayers(maxPlayers).build();
        return revisions == null ? candidate : candidate.toBuilder().setRevision(revisions.next()).build();
    }

    public record Input(
        String name,
        String uuid,
        String namespace,
        String world,
        long epoch,
        int x,
        int y,
        int z,
        int yaw,
        @Nullable String displayName,
        @Nullable Integer armor,
        @Nullable Integer health,
        boolean enabled,
        boolean hideSpectators,
        boolean hideInvisible,
        boolean hideMapInvisibilityEquipment,
        boolean spectator,
        boolean invisible,
        boolean mapInvisibilityEquipment,
        boolean hidden,
        boolean otherwiseHidden,
        boolean useDisplayName
    ) {}



    private static String hex(final byte[] bytes) {
        final StringBuilder out = new StringBuilder(bytes.length * 2);
        for (final byte value : bytes) out.append(String.format("%02x", value & 0xff));
        return out.toString();
    }

    private static ByteString uuidBytes(final String hex) {
        if (hex.length() != 32) throw new IllegalArgumentException("UUID must contain 32 hexadecimal characters");
        final byte[] bytes = new byte[16];
        for (int i = 0; i < bytes.length; i++) bytes[i] = (byte) Integer.parseInt(hex.substring(i * 2, i * 2 + 2), 16);
        return ByteString.copyFrom(bytes);
    }

    private static int armorPoints(final ServerPlayer player) {
        final @Nullable AttributeInstance attribute = player.getAttribute(Attributes.ARMOR);
        return attribute == null ? 0 : (int) attribute.getValue();
    }

    private static boolean hasMapInvisibilityItemEquipped(final ServerPlayer player) {
        for (final EquipmentSlot slot : EquipmentSlot.values()) {
            if (slot != EquipmentSlot.MAINHAND && slot != EquipmentSlot.OFFHAND && player.getItemBySlot(slot).is(ItemTags.MAP_INVISIBILITY_EQUIPMENT)) return true;
        }
        return false;
    }
}
