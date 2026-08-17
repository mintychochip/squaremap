package xyz.jpenilla.squaremap.common.data;

import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;
import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.storage.LevelData;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.checker.nullness.qual.Nullable;
import org.checkerframework.framework.qual.DefaultQualifier;
import xyz.jpenilla.squaremap.api.LayerProvider;
import xyz.jpenilla.squaremap.api.MapWorld;
import xyz.jpenilla.squaremap.api.Registry;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.common.LayerRegistry;
import xyz.jpenilla.squaremap.common.config.ConfigManager;
import xyz.jpenilla.squaremap.common.config.WorldAdvanced;
import xyz.jpenilla.squaremap.common.config.WorldConfig;
import xyz.jpenilla.squaremap.common.layer.SpawnIconLayer;
import xyz.jpenilla.squaremap.common.layer.WorldBorderLayer;
import xyz.jpenilla.squaremap.common.util.Colors;
import xyz.jpenilla.squaremap.common.util.Util;
import xyz.jpenilla.squaremap.common.visibilitylimit.VisibilityLimitImpl;

@DefaultQualifier(NonNull.class)
public abstract class MapWorldInternal implements MapWorld {
    private static final Map<WorldIdentifier, LayerRegistry> LAYER_REGISTRIES = new HashMap<>();

    private final ServerLevel level;
    private final WorldConfig worldConfig;
    private final WorldAdvanced advancedWorldConfig;
    private final Path dataPath;
    private final Path tilesPath;
    private final BlockColors blockColors;
    private final LevelBiomeColorData levelBiomeColorData;
    private final VisibilityLimitImpl visibilityLimit;
    private volatile long lastReset = -1;

    protected MapWorldInternal(
        final ServerLevel level,
        final DirectoryProvider directoryProvider,
        final ConfigManager configManager
    ) {
        this.level = level;

        this.worldConfig = configManager.worldConfig(this.level);
        this.advancedWorldConfig = configManager.worldAdvanced(this.level);

        this.blockColors = BlockColors.create(this);
        this.levelBiomeColorData = LevelBiomeColorData.create(this);

        this.dataPath = directoryProvider.getAndCreateDataDirectory(this.serverLevel());
        this.tilesPath = directoryProvider.getAndCreateTilesDirectory(this.serverLevel());

        this.layerRegistry(); // init the layer registry
        if (this.config().SPAWN_MARKER_ICON_ENABLED) {
            this.layerRegistry().register(SpawnIconLayer.KEY, new SpawnIconLayer(this));
        }
        if (this.config().WORLDBORDER_MARKER_ENABLED) {
            this.layerRegistry().register(WorldBorderLayer.KEY, new WorldBorderLayer(this));
        }

        this.visibilityLimit = new VisibilityLimitImpl(this);
        this.visibilityLimit.load(this.config().VISIBILITY_LIMITS);
    }

    @Override
    public Registry<LayerProvider> layerRegistry() {
        return LAYER_REGISTRIES.computeIfAbsent(this.identifier(), $ -> new LayerRegistry());
    }

    @Override
    public WorldIdentifier identifier() {
        return Util.worldIdentifier(this.level);
    }

    public Path dataPath() {
        return this.dataPath;
    }

    /**
     * Get the map visibility limit of the world. Only these regions are drawn,
     * even if more chunks exist on disk.
     *
     * @return The visibility limit.
     */
    //@Override
    public VisibilityLimitImpl visibilityLimit() {
        return this.visibilityLimit;
    }

    public LevelBiomeColorData levelBiomeColorData() {
        return this.levelBiomeColorData;
    }

    public WorldConfig config() {
        return this.worldConfig;
    }

    public WorldAdvanced advanced() {
        return this.advancedWorldConfig;
    }

    public ServerLevel serverLevel() {
        return this.level;
    }

    public Path tilesPath() {
        return this.tilesPath;
    }

    public @Nullable BlockPos getSpawnPos() {
        final LevelData.RespawnData respawnData = this.level.getServer().getRespawnData();
        if (respawnData.dimension().equals(this.level.dimension())) {
            return respawnData.pos();
        }
        return null;
    }

    public final BlockPos getAnySpawnPos() {
        @Nullable BlockPos pos = this.getSpawnPos();
        if (pos == null) {
            pos = this.level.getServer().getRespawnData().pos();
        }
        return pos;
    }

    public int getMapColor(final BlockState state) {
        final int special = this.blockColors.color(state);
        if (special != -1) {
            return special;
        }
        // getMapColor params are never used by vanilla - check on update
        // They are however used by certain mods like framed blocks, so we pass dummy values to avoid errors.
        // Proper framed blocks compatibility would require including block entities in the snapshot and passing the real position.
        // We would probably want to whitelist block entity types for performance and safety reasons.
        // Generally, we can't support 100% of possible modded uses of these parameters because of our off-main-thread chunk snapshot use.
        return Colors.rgb(state.getMapColor(EmptyBlockGetter.INSTANCE, BlockPos.ZERO));
    }

    public boolean shouldRenderDirtyChunk(final ChunkCoordinate coord) {
        return this.config().BACKGROUND_RENDER_ENABLED
            && this.visibilityLimit().shouldRenderChunk(coord);
    }

    public void shutdown() {
        if (this.layerRegistry().hasEntry(SpawnIconLayer.KEY)) {
            this.layerRegistry().unregister(SpawnIconLayer.KEY);
        }
        if (this.layerRegistry().hasEntry(WorldBorderLayer.KEY)) {
            this.layerRegistry().unregister(WorldBorderLayer.KEY);
        }
    }

    public void didReset() {
        this.lastReset = System.currentTimeMillis();
    }

    public long lastReset() {
        return this.lastReset;
    }

    public interface Factory {
        MapWorldInternal create(ServerLevel level);
    }
}
