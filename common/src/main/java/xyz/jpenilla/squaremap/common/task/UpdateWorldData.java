package xyz.jpenilla.squaremap.common.task;

import com.google.inject.Inject;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.framework.qual.DefaultQualifier;
import xyz.jpenilla.squaremap.common.WorldManager;
import xyz.jpenilla.squaremap.common.data.DirectoryProvider;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;
import xyz.jpenilla.squaremap.common.util.FileUtil;

@DefaultQualifier(NonNull.class)
public final class UpdateWorldData {
    private final WorldManager worldManager;
    private final DirectoryProvider directoryProvider;

    @Inject
    private UpdateWorldData(
        final WorldManager worldManager,
        final DirectoryProvider directoryProvider
    ) {
        this.worldManager = worldManager;
        this.directoryProvider = directoryProvider;
    }
    /** Writes an already-collected immutable snapshot without recollecting worlds. */
    public void publish(final xyz.jpenilla.squaremap.bridge.v1.WorldStateReplace snapshot) {
        FileUtil.atomicWriteJsonAsync(this.directoryProvider.tilesDirectory().resolve("settings.json"), document(snapshot));
        for (final MapWorldInternal mapWorld : this.worldManager.worlds()) {
            snapshot.getWorldsList().stream()
                .filter(world -> world.getIdentity().getNamespace().equals(mapWorld.identifier().namespace())
                    && world.getIdentity().getValue().equals(mapWorld.identifier().value()))
                .findFirst().ifPresent(world -> writeWorldSettings(mapWorld, world));
        }
    }

    public static Map<String, Object> document(final xyz.jpenilla.squaremap.bridge.v1.WorldStateReplace snapshot) {
        final List<Object> worlds = new ArrayList<>();
        for (final xyz.jpenilla.squaremap.bridge.v1.World world : snapshot.getWorldsList()) {
            final Map<String, Object> entry = new HashMap<>();
            entry.put("name", legacyWorldName(world.getIdentity()));
            entry.put("display_name", world.getDisplayName());
            entry.put("icon", world.getIcon());
            entry.put("type", world.getEnvironment());
            entry.put("order", world.getOrder());
            worlds.add(entry);
        }
        final xyz.jpenilla.squaremap.bridge.v1.UiSettings ui = snapshot.getUi();
        final Map<String, Object> coordinates = Map.of("enabled", ui.getCoordinatesEnabled(), "html", ui.getCoordinatesHtml());
        final Map<String, Object> sidebar = Map.of("pinned", ui.getSidebarPinned(), "player_list_label", ui.getSidebarPlayerListLabel(),
            "world_list_label", ui.getSidebarWorldListLabel());
        final Map<String, Object> uiDocument = Map.of("title", ui.getTitle(), "coordinates", coordinates,
            "link", Map.of("enabled", ui.getLinkEnabled()), "sidebar", sidebar);
        return Map.of("worlds", worlds, "ui", uiDocument);
    }

    private static String legacyWorldName(final xyz.jpenilla.squaremap.bridge.v1.WorldIdentity identity) {
        return identity.getNamespace() + "_" + identity.getValue();
    }

    private void writeWorldSettings(final MapWorldInternal mapWorld, final xyz.jpenilla.squaremap.bridge.v1.World world) {
        FileUtil.atomicWriteJsonAsync(mapWorld.tilesPath().resolve("settings.json"), worldDocument(world));
    }

    public static Map<String, Object> worldDocument(final xyz.jpenilla.squaremap.bridge.v1.World world) {
        final xyz.jpenilla.squaremap.bridge.v1.Spawn spawn = world.getSpawn();
        final xyz.jpenilla.squaremap.bridge.v1.PlayerTrackerSettings tracker = world.getPlayerTracker();
        final Map<String, Object> nameplates = Map.of("enabled", tracker.getNameplateEnabled(), "show_heads", tracker.getNameplateShowHeads(),
            "heads_url", tracker.getNameplateHeadsUrl(), "show_armor", tracker.getNameplateShowArmor(), "show_health", tracker.getNameplateShowHealth());
        final Map<String, Object> playerTracker = Map.of("enabled", tracker.getEnabled(), "update_interval", tracker.getUpdateInterval(),
            "label", tracker.getLabel(), "show_controls", tracker.getShowControls(), "default_hidden", tracker.getDefaultHidden(),
            "priority", tracker.getPriority(), "z_index", tracker.getZIndex(), "nameplates", nameplates);
        final Map<String, Object> zoom = Map.of("max", world.getZoom().getMax(), "def", world.getZoom().getDef(), "extra", world.getZoom().getExtra());
        return Map.of("spawn", Map.of("x", spawn.getX(), "z", spawn.getZ()), "player_tracker", playerTracker,
            "zoom", zoom, "marker_update_interval", world.getMarkerUpdateInterval(), "tiles_update_interval", world.getTilesUpdateInterval());
    }
}
