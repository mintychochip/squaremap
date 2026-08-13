package xyz.jpenilla.squaremap.common.bridge.state;

import com.google.inject.Inject;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.core.BlockPos;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.biome.BiomeManager;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.bridge.v1.PlayerTrackerSettings;
import xyz.jpenilla.squaremap.bridge.v1.Spawn;
import xyz.jpenilla.squaremap.bridge.v1.UiSettings;
import xyz.jpenilla.squaremap.bridge.v1.World;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;
import xyz.jpenilla.squaremap.bridge.v1.WorldSettings;
import xyz.jpenilla.squaremap.bridge.v1.WorldStateReplace;
import xyz.jpenilla.squaremap.bridge.v1.ZoomSettings;
import xyz.jpenilla.squaremap.common.WorldManager;
import xyz.jpenilla.squaremap.common.config.Config;
import xyz.jpenilla.squaremap.common.config.Messages;
import xyz.jpenilla.squaremap.common.config.WorldConfig;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;
import xyz.jpenilla.squaremap.common.util.Util;

/** Collects world settings into an immutable protocol snapshot. */
public final class WorldStateExporter {
    private final WorldManager worldManager;
    private final WorldEpochRegistry epochs;
    private final BridgeRevisionClock revisions;
    private WorldStateReplace last;

    @Inject
    public WorldStateExporter(
        final WorldManager worldManager,
        final WorldEpochRegistry epochs,
        final BridgeRevisionClock revisions
    ) {
        this.worldManager = worldManager;
        this.epochs = epochs;
        this.revisions = revisions;
    }

    public WorldStateReplace export() {
        final List<World> worlds = new ArrayList<>();
        this.worldManager.worlds().stream()
            .sorted(java.util.Comparator.comparing((MapWorldInternal world) -> world.identifier().namespace())
                .thenComparing(world -> world.identifier().value()))
            .forEach(mapWorld -> worlds.add(world(mapWorld, this.epochs.epoch(mapWorld.identifier(), mapWorld.serverLevel()))));
        final WorldStateReplace candidate = snapshot(worlds, ui());
        if (this.last != null && this.last.toBuilder().setRevision(0).build().equals(candidate)) return this.last;
        this.last = candidate.toBuilder().setRevision(this.revisions.next()).build();
        return this.last;
    }

    static WorldStateReplace snapshot(final List<World> worlds, final UiSettings ui) {
        return WorldStateReplace.newBuilder().addAllWorlds(List.copyOf(worlds)).setRevision(0).setUi(ui).build();
    }
    private static UiSettings ui() {
        return UiSettings.newBuilder()
            .setTitle(Messages.UI_TITLE)
            .setCoordinatesEnabled(Config.UI_COORDINATES_ENABLED)
            .setCoordinatesHtml(Messages.UI_COORDINATES_HTML)
            .setLinkEnabled(Config.UI_LINK_ENABLED)
            .setSidebarPinned(Config.UI_SIDEBAR_PINNED)
            .setSidebarPlayerListLabel(Messages.UI_SIDEBAR_PLAYER_LIST_LABEL)
            .setSidebarWorldListLabel(Messages.UI_SIDEBAR_WORLD_LIST_LABEL)
            .build();
    }

    public static World world(final MapWorldInternal mapWorld, final long epoch) {
        final ServerLevel level = mapWorld.serverLevel();
        final WorldConfig config = mapWorld.config();
        final BlockPos spawn = mapWorld.getAnySpawnPos();
        final WorldIdentifier id = mapWorld.identifier();
        final PlayerTrackerSettings tracker = PlayerTrackerSettings.newBuilder()
            .setEnabled(config.PLAYER_TRACKER_ENABLED)
            .setUpdateInterval(config.PLAYER_TRACKER_UPDATE_INTERVAL)
            .setLabel(Messages.UI_PLAYER_TRACKER_LABEL)
            .setShowControls(config.PLAYER_TRACKER_SHOW_CONTROLS)
            .setDefaultHidden(config.PLAYER_TRACKER_DEFAULT_HIDDEN)
            .setPriority(config.PLAYER_TRACKER_PRIORITY)
            .setZIndex(config.PLAYER_TRACKER_Z_INDEX)
            .setNameplateEnabled(config.PLAYER_TRACKER_NAMEPLATE_ENABLED)
            .setNameplateShowHeads(config.PLAYER_TRACKER_NAMEPLATE_SHOW_HEAD)
            .setNameplateHeadsUrl(config.PLAYER_TRACKER_NAMEPLATE_HEADS_URL)
            .setNameplateShowArmor(config.PLAYER_TRACKER_NAMEPLATE_SHOW_ARMOR)
            .setNameplateShowHealth(config.PLAYER_TRACKER_NAMEPLATE_SHOW_HEALTH)
            .setHideInvisible(config.PLAYER_TRACKER_HIDE_INVISIBLE)
            .setHideSpectators(config.PLAYER_TRACKER_HIDE_SPECTATORS)
            .setHideMapInvisibilityEquipment(config.PLAYER_TRACKER_HIDE_MAP_INVISIBILITY_EQUIPMENT)
            .setUseDisplayNames(config.PLAYER_TRACKER_USE_DISPLAY_NAME)
            .build();
        final WorldSettings settings = WorldSettings.newBuilder()
            .setMapEnabled(config.MAP_ENABLED)
            .setMapDisplayName(config.MAP_DISPLAY_NAME)
            .setMapOrder(config.MAP_ORDER)
            .setMapIcon(config.MAP_ICON)
            .setMaxRenderThreads(config.MAX_RENDER_THREADS)
            .setMapIterateUp(config.MAP_ITERATE_UP)
            .setMapMaxHeight(config.MAP_MAX_HEIGHT)
            .setMapBiomesEnabled(config.MAP_BIOMES)
            .setMapBiomesBlend(config.MAP_BIOMES_BLEND)
            .setMapGlassClear(config.MAP_GLASS_CLEAR)
            .setMapLavaCheckerboard(config.MAP_LAVA_CHECKERBOARD)
            .setMapWaterClear(config.MAP_WATER_CLEAR)
            .setMapWaterCheckerboard(config.MAP_WATER_CHECKERBOARD)
            .setZoomMax(config.ZOOM_MAX)
            .setZoomDefault(config.ZOOM_DEFAULT)
            .setZoomExtra(config.ZOOM_EXTRA)
            .setBackgroundRenderEnabled(config.BACKGROUND_RENDER_ENABLED)
            .setBackgroundRenderMaxChunksPerInterval(config.BACKGROUND_RENDER_MAX_CHUNKS_PER_INTERVAL)
            .setBackgroundRenderIntervalSeconds(config.BACKGROUND_RENDER_INTERVAL_SECONDS)
            .setBackgroundRenderMaxThreads(config.BACKGROUND_RENDER_MAX_THREADS)
            .setBiomeZoomSeed(BiomeManager.obfuscateSeed(level.getSeed()))
            .setPlayerTrackerEnabled(config.PLAYER_TRACKER_ENABLED)
            .setPlayerTrackerUpdateInterval(config.PLAYER_TRACKER_UPDATE_INTERVAL)
            .setPlayerTrackerShowControls(config.PLAYER_TRACKER_SHOW_CONTROLS)
            .setPlayerTrackerDefaultHidden(config.PLAYER_TRACKER_DEFAULT_HIDDEN)
            .setPlayerTrackerPriority(config.PLAYER_TRACKER_PRIORITY)
            .setPlayerTrackerZIndex(config.PLAYER_TRACKER_Z_INDEX)
            .setPlayerTrackerNameplateEnabled(config.PLAYER_TRACKER_NAMEPLATE_ENABLED)
            .setPlayerTrackerNameplateShowHead(config.PLAYER_TRACKER_NAMEPLATE_SHOW_HEAD)
            .setPlayerTrackerNameplateHeadsUrl(config.PLAYER_TRACKER_NAMEPLATE_HEADS_URL)
            .setPlayerTrackerNameplateShowArmor(config.PLAYER_TRACKER_NAMEPLATE_SHOW_ARMOR)
            .setPlayerTrackerNameplateShowHealth(config.PLAYER_TRACKER_NAMEPLATE_SHOW_HEALTH)
            .setMarkerApiUpdateIntervalSeconds(config.MARKER_API_UPDATE_INTERVAL_SECONDS)
            .build();
        return World.newBuilder()
            .setIdentity(WorldIdentity.newBuilder().setNamespace(id.namespace()).setValue(id.value()).setEpoch(epoch))
            .setDisplayName(config.MAP_DISPLAY_NAME.replace("{world}", Util.levelConfigName(level)))
            .setIcon(config.MAP_ICON)
            .setOrder(config.MAP_ORDER)
            .setEnvironment(environment(level))
            .setEnabled(config.MAP_ENABLED)
            .setSpawn(Spawn.newBuilder().setX(spawn.getX()).setZ(spawn.getZ()))
            .setPlayerTracker(tracker)
            .setZoom(ZoomSettings.newBuilder().setMax(config.ZOOM_MAX).setDef(config.ZOOM_DEFAULT).setExtra(config.ZOOM_EXTRA))
            .setMarkerUpdateInterval(config.MARKER_API_UPDATE_INTERVAL_SECONDS)
            .setTilesUpdateInterval(config.BACKGROUND_RENDER_INTERVAL_SECONDS)
            .setSettings(settings)
            .build();
    }

    private static String environment(final ServerLevel level) {
        final ResourceKey<Level> key = level.dimension();
        if (key == Level.NETHER) return "nether";
        if (key == Level.END) return "the_end";
        if (key == Level.OVERWORLD) return "normal";
        return "custom";
    }
}
