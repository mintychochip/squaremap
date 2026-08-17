package xyz.jpenilla.squaremap.common.config;

import com.google.inject.Inject;
import com.google.inject.Singleton;
import it.unimi.dsi.fastutil.objects.Reference2IntMap;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.biome.BiomeManager;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.bridge.v1.AdvancedSettings;
import xyz.jpenilla.squaremap.bridge.v1.ColorOverride;
import xyz.jpenilla.squaremap.bridge.v1.ConfigReplace;
import xyz.jpenilla.squaremap.bridge.v1.GlobalSettings;
import xyz.jpenilla.squaremap.bridge.v1.LocaleSettings;
import xyz.jpenilla.squaremap.bridge.v1.RenderSettings;
import xyz.jpenilla.squaremap.bridge.v1.UiSettings;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;
import xyz.jpenilla.squaremap.bridge.v1.WorldSettings;
import xyz.jpenilla.squaremap.common.ServerAccess;
import xyz.jpenilla.squaremap.common.bridge.state.BridgeRevisionClock;
import xyz.jpenilla.squaremap.common.bridge.state.WorldEpochRegistry;
import xyz.jpenilla.squaremap.common.util.Util;
import xyz.jpenilla.squaremap.common.visibilitylimit.VisibilityLimitProtocol;

/** Converts the validated YAML-derived Java configuration into protocol values. */
@Singleton
public final class ConfigBridgeExporter {
    private final ServerAccess serverAccess;
    private final ConfigManager configManager;
    private final WorldEpochRegistry epochs;
    private final BridgeRevisionClock revisions;

    @Inject
    public ConfigBridgeExporter(final ServerAccess serverAccess, final ConfigManager configManager,
                                final WorldEpochRegistry epochs, final BridgeRevisionClock revisions) {
        this.serverAccess = serverAccess;
        this.configManager = configManager;
        this.epochs = epochs;
        this.revisions = revisions;
    }

    public ConfigReplace export() {
        final long revision = this.revisions.next();
        final List<ServerLevel> levels = this.serverAccess.levels().stream()
            .sorted(Comparator.comparing(level -> Util.worldIdentifier(level).asString())).toList();
        final WorldConfig defaultConfig = levels.isEmpty() ? null : this.configManager.worldConfig(levels.get(0));
        final GlobalSettings global = GlobalSettings.newBuilder()
            .setLanguageFile(Config.LANGUAGE_FILE).setDebugMode(Config.DEBUG_MODE)
            .setUpdateChecker(Config.UPDATE_CHECKER).setWebAddress(Config.WEB_ADDRESS)
            .setWebDirectory(Config.WEB_DIR).setUpdateWebDirectory(Config.UPDATE_WEB_DIR)
            .setCompressImages(Config.COMPRESS_IMAGES).setCompressionRatio(Config.COMPRESSION_RATIO)
            .setHttpEnabled(Config.HTTPD_ENABLED)
            .setHttpBind(Config.HTTPD_BIND).setHttpPort(Config.HTTPD_PORT)
            .setFlushJsonImmediately(Config.FLUSH_JSON_IMMEDIATELY)
            .setMainCommandLabel(Config.MAIN_COMMAND_LABEL).addAllMainCommandAliases(Config.MAIN_COMMAND_ALIASES)
            .build();
        final WorldSettings defaultWorld = defaultConfig == null ? WorldSettings.getDefaultInstance() : worldSettings(defaultConfig);
        final ConfigReplace.Builder result = ConfigReplace.newBuilder().setRevision(revision).setGlobal(global)
            .setAdvanced(advancedSettings(levels))
            .setWorld(defaultWorld)
            .setLocale(LocaleSettings.newBuilder()
                .setLanguage(Config.LANGUAGE_FILE).setUiTitle(Messages.UI_TITLE).setCoordinatesHtml(Messages.UI_COORDINATES_HTML)
                .setPlayerTrackerLabel(Messages.UI_PLAYER_TRACKER_LABEL).setSidebarPlayerListLabel(Messages.UI_SIDEBAR_PLAYER_LIST_LABEL)
                .setSidebarWorldListLabel(Messages.UI_SIDEBAR_WORLD_LIST_LABEL).setSpawnMarkerLabel(Messages.UI_SPAWN_MARKER_ICON_LABEL)
                .setWorldBorderMarkerLabel(Messages.UI_WORLDBORDER_MARKER_LABEL).build())
            .setRender(RenderSettings.newBuilder().setProgressLoggingEnabled(Config.PROGRESS_LOGGING)
                .setProgressLoggingIntervalSeconds(Math.max(1, Config.PROGRESS_LOGGING_INTERVAL))
                .setBackgroundEnabled(defaultConfig == null || defaultConfig.BACKGROUND_RENDER_ENABLED)
                .setBackgroundMaxChunksPerInterval(defaultConfig == null ? 1024 : Math.max(1, defaultConfig.BACKGROUND_RENDER_MAX_CHUNKS_PER_INTERVAL))
                .setBackgroundIntervalSeconds(defaultConfig == null ? 15 : Math.max(1, defaultConfig.BACKGROUND_RENDER_INTERVAL_SECONDS))
                .setBackgroundMaxThreads(defaultConfig == null ? -1 : defaultConfig.BACKGROUND_RENDER_MAX_THREADS).build())
            .setUi(UiSettings.newBuilder().setCoordinatesEnabled(Config.UI_COORDINATES_ENABLED)
                .setLinkEnabled(Config.UI_LINK_ENABLED).setSidebarPinned(Config.UI_SIDEBAR_PINNED)
                .setTitle(Messages.UI_TITLE).setCoordinatesHtml(Messages.UI_COORDINATES_HTML)
                .setSidebarPlayerListLabel(Messages.UI_SIDEBAR_PLAYER_LIST_LABEL).setSidebarWorldListLabel(Messages.UI_SIDEBAR_WORLD_LIST_LABEL).build())
            .setPlayerPrivacyEnabled(Config.BRIDGE_PLAYER_PRIVACY_ENABLED)
            .setEventCaptureEnabled(Config.BRIDGE_EVENT_CAPTURE_ENABLED);
        for (final ServerLevel level : levels) {
            final WorldIdentifier id = Util.worldIdentifier(level);
            result.addWorlds(xyz.jpenilla.squaremap.bridge.v1.WorldConfig.newBuilder()
                .setIdentity(WorldIdentity.newBuilder().setNamespace(id.namespace()).setValue(id.value())
                    .setEpoch(this.epochs.epoch(id, level)))
                .setSettings(worldSettings(
                    this.configManager.worldConfig(level),
                    BiomeManager.obfuscateSeed(level.getSeed())
                )).build());
        }
        return result.build();
    }

    private AdvancedSettings advancedSettings(final List<ServerLevel> levels) {
        final Map<String, Integer> invisible = new TreeMap<>();
        final Map<String, Integer> iterateUp = new TreeMap<>();
        final Map<String, Integer> foliage = new TreeMap<>();
        final Map<String, Integer> grass = new TreeMap<>();
        final Map<String, Integer> water = new TreeMap<>();
        final Map<String, Integer> blocks = new TreeMap<>();
        for (final ServerLevel level : levels) {
            final WorldAdvanced advanced = this.configManager.worldAdvanced(level);
            advanced.invisibleBlocks.forEach(block -> invisible.put(key(BuiltInRegistries.BLOCK.getKey(block)), 1));
            advanced.iterateUpBaseBlocks.forEach(block -> iterateUp.put(key(BuiltInRegistries.BLOCK.getKey(block)), 1));
            addColors(foliage, advanced.COLOR_OVERRIDES_BIOME_FOLIAGE, Util.biomeRegistry(level));
            addColors(grass, advanced.COLOR_OVERRIDES_BIOME_GRASS, Util.biomeRegistry(level));
            addColors(water, advanced.COLOR_OVERRIDES_BIOME_WATER, Util.biomeRegistry(level));
            addColors(blocks, advanced.COLOR_OVERRIDES_BLOCKS, BuiltInRegistries.BLOCK);
        }
        final AdvancedSettings.Builder builder = AdvancedSettings.newBuilder()
            .addAllInvisibleBlocks(invisible.keySet()).addAllIterateUpBaseBlocks(iterateUp.keySet());
        foliage.forEach((key, color) -> builder.addBiomeFoliageColorOverrides(ColorOverride.newBuilder().setRegistryKey(key).setColor(color)));
        grass.forEach((key, color) -> builder.addBiomeGrassColorOverrides(ColorOverride.newBuilder().setRegistryKey(key).setColor(color)));
        water.forEach((key, color) -> builder.addBiomeWaterColorOverrides(ColorOverride.newBuilder().setRegistryKey(key).setColor(color)));
        blocks.forEach((key, color) -> builder.addBlockColorOverrides(ColorOverride.newBuilder().setRegistryKey(key).setColor(color)));
        return builder.build();
    }

    private static <T> void addColors(final Map<String, Integer> target, final Reference2IntMap<T> values, final net.minecraft.core.Registry<T> registry) {
        for (final Reference2IntMap.Entry<T> entry : values.reference2IntEntrySet()) {
            final Identifier id = registry.getKey(entry.getKey());
            if (id != null) target.put(key(id), entry.getIntValue());
        }
    }

    private static String key(final Identifier id) { return id == null ? "" : id.toString(); }

    private static WorldSettings worldSettings(final WorldConfig config) {
        return worldSettings(config, 0L);
    }

    private static WorldSettings worldSettings(final WorldConfig config, final long biomeZoomSeed) {
        return WorldSettings.newBuilder()
            .setMapEnabled(config.MAP_ENABLED).setMapDisplayName(config.MAP_DISPLAY_NAME).setMapOrder(config.MAP_ORDER)
            .setMapIcon(config.MAP_ICON).setMaxRenderThreads(config.MAX_RENDER_THREADS).setMapIterateUp(config.MAP_ITERATE_UP)
            .setMapMaxHeight(config.MAP_MAX_HEIGHT).setMapBiomesEnabled(config.MAP_BIOMES).setMapBiomesBlend(config.MAP_BIOMES_BLEND)
            .setMapGlassClear(config.MAP_GLASS_CLEAR).setMapLavaCheckerboard(config.MAP_LAVA_CHECKERBOARD)
            .setMapWaterClear(config.MAP_WATER_CLEAR).setMapWaterCheckerboard(config.MAP_WATER_CHECKERBOARD)
            .setZoomMax(config.ZOOM_MAX).setZoomDefault(config.ZOOM_DEFAULT).setZoomExtra(config.ZOOM_EXTRA)
            .setBackgroundRenderEnabled(config.BACKGROUND_RENDER_ENABLED)
            .setBackgroundRenderMaxChunksPerInterval(config.BACKGROUND_RENDER_MAX_CHUNKS_PER_INTERVAL)
            .setBackgroundRenderIntervalSeconds(config.BACKGROUND_RENDER_INTERVAL_SECONDS)
            .setBackgroundRenderMaxThreads(config.BACKGROUND_RENDER_MAX_THREADS)
            .setPlayerTrackerEnabled(config.PLAYER_TRACKER_ENABLED).setPlayerTrackerUpdateInterval(config.PLAYER_TRACKER_UPDATE_INTERVAL)
            .setPlayerTrackerShowControls(config.PLAYER_TRACKER_SHOW_CONTROLS).setPlayerTrackerDefaultHidden(config.PLAYER_TRACKER_DEFAULT_HIDDEN)
            .setPlayerTrackerPriority(config.PLAYER_TRACKER_PRIORITY).setPlayerTrackerZIndex(config.PLAYER_TRACKER_Z_INDEX)
            .setPlayerTrackerNameplateEnabled(config.PLAYER_TRACKER_NAMEPLATE_ENABLED)
            .setPlayerTrackerNameplateShowHead(config.PLAYER_TRACKER_NAMEPLATE_SHOW_HEAD)
            .setPlayerTrackerNameplateHeadsUrl(config.PLAYER_TRACKER_NAMEPLATE_HEADS_URL)
            .setPlayerTrackerNameplateShowArmor(config.PLAYER_TRACKER_NAMEPLATE_SHOW_ARMOR)
            .setPlayerTrackerNameplateShowHealth(config.PLAYER_TRACKER_NAMEPLATE_SHOW_HEALTH)
            .setPlayerTrackerHideInvisible(config.PLAYER_TRACKER_HIDE_INVISIBLE)
            .setPlayerTrackerHideSpectators(config.PLAYER_TRACKER_HIDE_SPECTATORS)
            .setPlayerTrackerHideMapInvisibilityEquipment(config.PLAYER_TRACKER_HIDE_MAP_INVISIBILITY_EQUIPMENT)
            .setPlayerTrackerUseDisplayName(config.PLAYER_TRACKER_USE_DISPLAY_NAME)
            .setMarkerApiUpdateIntervalSeconds(config.MARKER_API_UPDATE_INTERVAL_SECONDS)
            .setSpawnMarkerIconEnabled(config.SPAWN_MARKER_ICON_ENABLED).setSpawnMarkerIconShowControls(config.SPAWN_MARKER_ICON_SHOW_CONTROLS)
            .setSpawnMarkerIconDefaultHidden(config.SPAWN_MARKER_ICON_DEFAULT_HIDDEN)
            .setSpawnMarkerIconLayerPriority(config.SPAWN_MARKER_ICON_LAYER_PRIORITY).setSpawnMarkerIconZIndex(config.SPAWN_MARKER_ICON_Z_INDEX)
            .setWorldborderMarkerEnabled(config.WORLDBORDER_MARKER_ENABLED).setWorldborderMarkerShowControls(config.WORLDBORDER_MARKER_SHOW_CONTROLS)
            .setWorldborderMarkerDefaultHidden(config.WORLDBORDER_MARKER_DEFAULT_HIDDEN)
            .setWorldborderMarkerLayerPriority(config.WORLDBORDER_MARKER_LAYER_PRIORITY).setWorldborderMarkerZIndex(config.WORLDBORDER_MARKER_Z_INDEX)
            .addAllVisibilityLimits(VisibilityLimitProtocol.serialize(config.VISIBILITY_LIMITS))
            .setBiomeZoomSeed(biomeZoomSeed)
            .build();
    }
}
