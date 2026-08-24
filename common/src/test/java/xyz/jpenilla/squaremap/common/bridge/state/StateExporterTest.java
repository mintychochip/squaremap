package xyz.jpenilla.squaremap.common.bridge.state;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;
import com.google.gson.JsonElement;
import com.google.gson.JsonParser;
import java.awt.Color;
import java.awt.image.BufferedImage;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Collection;
import java.util.List;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.api.Key;
import xyz.jpenilla.squaremap.api.Pair;
import xyz.jpenilla.squaremap.api.Point;
import xyz.jpenilla.squaremap.api.LayerProvider;
import xyz.jpenilla.squaremap.api.marker.MarkerOptions;
import xyz.jpenilla.squaremap.common.task.UpdateMarkers;
import xyz.jpenilla.squaremap.common.task.UpdatePlayers;
import xyz.jpenilla.squaremap.common.task.UpdateWorldData;
import xyz.jpenilla.squaremap.common.util.Util;
import xyz.jpenilla.squaremap.bridge.v1.Icon;
import xyz.jpenilla.squaremap.bridge.v1.IconsReplace;
import xyz.jpenilla.squaremap.bridge.v1.Player;
import xyz.jpenilla.squaremap.bridge.v1.PlayersReplace;
import xyz.jpenilla.squaremap.bridge.v1.PlayerTrackerSettings;
import xyz.jpenilla.squaremap.bridge.v1.Spawn;
import xyz.jpenilla.squaremap.bridge.v1.UiSettings;
import xyz.jpenilla.squaremap.bridge.v1.World;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;
import xyz.jpenilla.squaremap.bridge.v1.WorldStateReplace;
import xyz.jpenilla.squaremap.bridge.v1.ZoomSettings;

final class StateExporterTest {
    private static final Path JAVA_FIXTURES = Path.of(
        System.getProperty("squaremap.task11.root"),
        "testdata/bridge/v1/fixtures/java"
    );
    private static final List<String> LAYER3_JSON = List.of(
        "empty.json",
        "settings.json",
        "world-settings.json",
        "players.json",
        "markers.json",
        "icons.json"
    );

    @Test
    void playerCoreFiltersPrivateStatesAndPreservesPublicFields() {
        final PlayerStateExporter.Input publicPlayer = new PlayerStateExporter.Input(
            "public", "00112233445566778899aabbccddeeff", "minecraft", "overworld", 3,
            1, 64, 2, 90, null, 7, 20, true, false, false, false, false, false, false, false, false, false
        );
        final PlayerStateExporter.Input hidden = new PlayerStateExporter.Input(
            "hidden", "11112233445566778899aabbccddeeff", "minecraft", "overworld", 3,
            1, 64, 2, 90, null, 7, 20, true, false, false, false, false, false, false, true, false, false
        );
        final PlayerStateExporter.Input spectator = new PlayerStateExporter.Input(
            "spectator", "22222233445566778899aabbccddeeff", "minecraft", "overworld", 3,
            1, 64, 2, 90, null, 7, 20, true, true, false, false, true, false, false, false, false, false
        );
        final PlayerStateExporter.Input invisible = new PlayerStateExporter.Input(
            "invisible", "33333333445566778899aabbccddeeff", "minecraft", "overworld", 3,
            1, 64, 2, 90, null, 7, 20, true, false, true, false, false, true, false, false, false, false
        );
        final PlayerStateExporter.Input equipped = new PlayerStateExporter.Input(
            "equipped", "44444444444444448899aabbccddeeff", "minecraft", "overworld", 3,
            1, 64, 2, 90, null, 7, 20, true, false, false, true, false, false, true, false, false, false
        );
        final PlayersReplace snapshot = PlayerStateExporter.snapshot(
            List.of(publicPlayer, hidden, spectator, invisible, equipped), 20, new BridgeRevisionClock()
        );
        assertEquals(1, snapshot.getPlayersCount());
        final Player player = snapshot.getPlayers(0);
        assertEquals("minecraft", player.getWorld().getNamespace());
        assertEquals("overworld", player.getWorld().getValue());
        assertTrue(player.hasArmor());
        assertTrue(player.hasHealth());
        assertFalse(player.hasDisplayName());
    }

    @Test
    void canonicalServerLevelTokenKeepsEpochStableAcrossSchedulesAndIncrementsOnReload() {
        final WorldEpochRegistry epochs = new WorldEpochRegistry();
        final xyz.jpenilla.squaremap.api.WorldIdentifier identifier = xyz.jpenilla.squaremap.api.WorldIdentifier.create("minecraft", "overworld");
        final Object loadedLevel = new Object();
        assertEquals(1, epochs.epoch(identifier, loadedLevel));
        assertEquals(1, epochs.epoch(identifier, loadedLevel));
        assertEquals(1, epochs.epoch(identifier, loadedLevel));
        assertEquals(2, epochs.epoch(identifier, new Object()));
    }

    @Test
    void markerStateKeepsTimestampWithinLoadAndResetsOnReload() {
        final MarkerStateExporter exporter = new MarkerStateExporter(null, new WorldEpochRegistry(), new BridgeRevisionClock());
        final Key key = Key.of("layer");
        final Object loadedLevel = new Object();
        exporter.bindTokenForTesting(loadedLevel);
        exporter.retainLayerForTesting(key, new byte[] {1}, 42);
        exporter.bindTokenForTesting(loadedLevel);
        assertEquals(1, exporter.retainedLayerCountForTesting());
        assertEquals(42, exporter.retainedTimestampForTesting(key));
        exporter.bindTokenForTesting(new Object());
        assertEquals(0, exporter.retainedLayerCountForTesting());
    }

    @Test
    void frozenLayer3JavaFixturesExist() {
        assertTrue(Files.isDirectory(JAVA_FIXTURES), JAVA_FIXTURES.toString());
        for (final String name : LAYER3_JSON) {
            assertTrue(Files.isRegularFile(JAVA_FIXTURES.resolve(name)), JAVA_FIXTURES.resolve(name).toString());
        }
    }

    @Test
    void checkedInGoldenDocumentsUseProductionExporterAndSinkCores() throws Exception {
        final Path root = JAVA_FIXTURES;
        final JsonElement empty = fixture(root, "empty.json");
        final JsonElement settings = fixture(root, "settings.json");
        final JsonElement worldSettings = fixture(root, "world-settings.json");
        final JsonElement players = fixture(root, "players.json");
        final JsonElement icons = fixture(root, "icons.json");
        final JsonElement markers = fixture(root, "markers.json");

        final UiSettings ui = UiSettings.newBuilder().setTitle("Squaremap").setCoordinatesEnabled(true)
            .setCoordinatesHtml("Coordinates").setLinkEnabled(true).setSidebarPinned("minecraft_overworld")
            .setSidebarPlayerListLabel("Players").setSidebarWorldListLabel("Worlds").build();
        final World overworld = world("minecraft", "overworld", "World", "default", "normal", 0, 0, 0, true);
        final World nether = world("minecraft", "nether", "Nether", "nether", "nether", 1, 0, 0, false);
        final WorldStateReplace worlds = WorldStateExporter.snapshot(List.of(overworld, nether), ui);
        assertEquals(settings, JsonParser.parseString(Util.gson().toJson(UpdateWorldData.document(worlds))));
        assertEquals(worldSettings, JsonParser.parseString(Util.gson().toJson(UpdateWorldData.worldDocument(overworld))));

        final PlayerStateExporter.Input fixturePlayer = new PlayerStateExporter.Input(
            "public", "00112233445566778899aabbccddeeff", "minecraft", "overworld", 0,
            1, 64, 2, 90, null, 7, 20, true, false, false, false, false, false, false, false, false, false
        );
        final PlayersReplace playersSnapshot = PlayerStateExporter.snapshot(List.of(fixturePlayer), 20, new BridgeRevisionClock());
        assertEquals(players, JsonParser.parseString(UpdatePlayers.document(playersSnapshot)));

        final LayerProvider provider = allMarkers();
        final long timestamp = markers.getAsJsonArray().get(0).getAsJsonObject().get("timestamp").getAsLong();
        final var markerSnapshot = MarkerStateExporter.snapshot("minecraft", "overworld", 1, Key.of("all"), provider, timestamp);
        assertEquals(markers, JsonParser.parseString(UpdateMarkers.document(markerSnapshot)));
        final BufferedImage image = new BufferedImage(2, 1, BufferedImage.TYPE_INT_ARGB);
        image.setRGB(0, 0, 0xffff0000);
        image.setRGB(1, 0, 0xff0000ff);
        final IconsReplace iconSnapshot = IconStateExporter.snapshot(List.of(Pair.of(Key.of("spawn"), image)), 1);
        assertEquals("spawn", iconSnapshot.getIcons(0).getId());
        assertEquals("image/rgba", iconSnapshot.getIcons(0).getMimeType());
        assertEquals(2, iconSnapshot.getIcons(0).getWidth());
        assertEquals(1, iconSnapshot.getIcons(0).getHeight());
        assertArrayEquals(new byte[] {(byte) 255, 0, 0, (byte) 255, 0, 0, (byte) 255, (byte) 255},
            iconSnapshot.getIcons(0).getImage().toByteArray());
        assertEquals(empty, JsonParser.parseString(Util.gson().toJson(UpdateWorldData.document(WorldStateExporter.snapshot(List.of(), UiSettings.getDefaultInstance())))));
    }

    private static JsonElement fixture(final Path root, final String name) throws Exception {
        return JsonParser.parseString(Files.readString(root.resolve(name)));
    }

    private static World world(final String namespace, final String value, final String display, final String icon,
                               final String environment, final int order, final int spawnX, final int spawnZ, final boolean tracker) {
        return World.newBuilder().setIdentity(WorldIdentity.newBuilder().setNamespace(namespace).setValue(value))
            .setDisplayName(display).setIcon(icon).setEnvironment(environment).setOrder(order)
            .setSpawn(Spawn.newBuilder().setX(spawnX).setZ(spawnZ))
            .setPlayerTracker(PlayerTrackerSettings.newBuilder().setEnabled(tracker).setUpdateInterval(1).setLabel("Players")
                .setShowControls(true).setPriority(0).setZIndex(100).setNameplateEnabled(tracker).setNameplateShowHeads(tracker)
                .setNameplateHeadsUrl("/heads/{uuid}").setNameplateShowArmor(tracker).setNameplateShowHealth(tracker))
            .setZoom(ZoomSettings.newBuilder().setMax(5).setDef(3).setExtra(1))
            .setMarkerUpdateInterval(5).setTilesUpdateInterval(10).build();
    }

    private static LayerProvider allMarkers() {
        final List<Point> triangle = List.of(Point.of(0, 0), Point.of(10, 0), Point.of(10, 10));
        final List<Point> hole = List.of(Point.of(2, 2), Point.of(3, 2), Point.of(3, 3));
        final MarkerOptions options = MarkerOptions.builder().stroke(false).strokeColor(Color.RED).strokeWeight(2)
            .strokeOpacity(0.5).fill(false).fillColor(Color.GREEN).fillOpacity(0.4)
            .fillRule(MarkerOptions.FillRule.NONZERO).clickTooltip("click").hoverTooltip("hover").build();
        final List<xyz.jpenilla.squaremap.api.marker.Marker> markers = List.of(
            xyz.jpenilla.squaremap.api.marker.Marker.icon(Point.of(1, 2), Point.of(0, 0), Point.of(8, 16), Key.of("spawn"), 16, 16),
            xyz.jpenilla.squaremap.api.marker.Marker.circle(Point.of(3, 4), 5.5),
            xyz.jpenilla.squaremap.api.marker.Marker.ellipse(Point.of(6, 7), 8.5, 9.5),
            xyz.jpenilla.squaremap.api.marker.Marker.rectangle(Point.of(0, 0), Point.of(10, 10)),
            xyz.jpenilla.squaremap.api.marker.Marker.polyline(Point.of(0, 0), Point.of(1, 1)),
            xyz.jpenilla.squaremap.api.marker.Marker.multiPolyline(List.of(List.of(Point.of(2, 2)), List.of(Point.of(3, 3)))),
            xyz.jpenilla.squaremap.api.marker.Marker.polygon(triangle, List.of(hole)),
            xyz.jpenilla.squaremap.api.marker.Marker.multiPolygon(
                xyz.jpenilla.squaremap.api.marker.MultiPolygon.part(List.of(Point.of(0, 0), Point.of(1, 0), Point.of(1, 1))),
                xyz.jpenilla.squaremap.api.marker.MultiPolygon.part(List.of(Point.of(4, 4), Point.of(5, 4), Point.of(5, 5)))
            ).markerOptions(options)
        );
        return new LayerProvider() {
            public String getLabel() { return "All"; }
            public boolean showControls() { return true; }
            public boolean defaultHidden() { return false; }
            public int layerPriority() { return 0; }
            public int zIndex() { return 10; }
            public Collection<xyz.jpenilla.squaremap.api.marker.Marker> getMarkers() { return markers; }
        };
    }
}
