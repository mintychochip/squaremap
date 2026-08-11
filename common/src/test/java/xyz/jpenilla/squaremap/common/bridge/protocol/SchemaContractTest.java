package xyz.jpenilla.squaremap.common.bridge.protocol;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.google.protobuf.ByteString;
import java.util.HexFormat;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.bridge.v1.AdvancedSettings;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSection;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshot;
import xyz.jpenilla.squaremap.bridge.v1.ChunkSnapshotBody;
import xyz.jpenilla.squaremap.bridge.v1.ConfigReplace;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.GlobalSettings;
import xyz.jpenilla.squaremap.bridge.v1.LocaleSettings;
import xyz.jpenilla.squaremap.bridge.v1.Marker;
import xyz.jpenilla.squaremap.bridge.v1.MarkerCircle;
import xyz.jpenilla.squaremap.bridge.v1.MarkerEllipse;
import xyz.jpenilla.squaremap.bridge.v1.MarkerIcon;
import xyz.jpenilla.squaremap.bridge.v1.MarkerLayer;
import xyz.jpenilla.squaremap.bridge.v1.MarkerMultiPolygon;
import xyz.jpenilla.squaremap.bridge.v1.MarkerPolygon;
import xyz.jpenilla.squaremap.bridge.v1.MarkerPolyline;
import xyz.jpenilla.squaremap.bridge.v1.MarkerRectangle;
import xyz.jpenilla.squaremap.bridge.v1.MarkerStyle;
import xyz.jpenilla.squaremap.bridge.v1.MarkerTooltip;
import xyz.jpenilla.squaremap.bridge.v1.Player;
import xyz.jpenilla.squaremap.bridge.v1.RenderSettings;
import xyz.jpenilla.squaremap.bridge.v1.PlayerTrackerSettings;
import xyz.jpenilla.squaremap.bridge.v1.PlayersReplace;
import xyz.jpenilla.squaremap.bridge.v1.Point;
import xyz.jpenilla.squaremap.bridge.v1.PointList;
import xyz.jpenilla.squaremap.bridge.v1.Spawn;
import xyz.jpenilla.squaremap.bridge.v1.UiSettings;
import xyz.jpenilla.squaremap.bridge.v1.World;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;
import xyz.jpenilla.squaremap.bridge.v1.WorldSettings;
import xyz.jpenilla.squaremap.bridge.v1.ZoomSettings;

class SchemaContractTest {
    @Test
    void exposesTypedStateAndPresenceAwarePayloads() {
        final ConfigReplace config = ConfigReplace.newBuilder()
            .setGlobal(GlobalSettings.newBuilder().setLanguageFile("lang-en.yml"))
            .setAdvanced(AdvancedSettings.newBuilder().addInvisibleBlocks("minecraft:short_grass"))
            .setWorld(WorldSettings.newBuilder().setMapEnabled(true))
            .setLocale(LocaleSettings.newBuilder()
                .setUiTitle("squaremap")
                .setSpawnMarkerLabel("Spawn")
                .setWorldBorderMarkerLabel("World Border"))
            .setRender(RenderSettings.newBuilder().setProgressLoggingEnabled(true))
            .setUi(UiSettings.newBuilder().setCoordinatesEnabled(true))
            .build();
        final World world = World.newBuilder()
            .setIdentity(WorldIdentity.newBuilder().setNamespace("minecraft").setValue("overworld"))
            .setIcon("world")
            .setOrder(1)
            .setSpawn(Spawn.newBuilder().setX(1).setZ(-2))
            .setPlayerTracker(PlayerTrackerSettings.newBuilder().setEnabled(true))
            .setZoom(ZoomSettings.newBuilder().setMax(3))
            .build();
        final Player player = Player.newBuilder()
            .setUuid(ByteString.copyFrom(HexFormat.of().parseHex("00112233445566778899aabbccddeeff")))
            .setWorld(world.getIdentity())
            .setDisplayName("Display")
            .setArmor(20)
            .setHealth(20)
            .build();
        final PlayersReplace players = PlayersReplace.newBuilder().addPlayers(player).setMaxPlayers(20).build();
        final Marker marker = Marker.newBuilder()
            .setIcon(MarkerIcon.newBuilder().setImage("pin"))
            .setStyle(MarkerStyle.newBuilder().setStrokeColor("#0000ff").setFillColor("#0000ff"))
            .setTooltip(MarkerTooltip.newBuilder().setHover("hover"))
            .build();
        final Point point = Point.newBuilder().setX(-1.25).setZ(2.5).build();
        final PointList points = PointList.newBuilder().addPoints(point).build();
        final MarkerLayer layer = MarkerLayer.newBuilder()
            .setVisible(true)
            .setShowControls(true)
            .setDefaultHidden(true)
            .setLayerPriority(-1)
            .setZIndex(2)
            .build();
        final Marker circle = Marker.newBuilder().setCircle(MarkerCircle.newBuilder().setCenter(point)).build();
        final Marker ellipse = Marker.newBuilder().setEllipse(MarkerEllipse.newBuilder().setCenter(point)).build();
        final Marker rectangle = Marker.newBuilder()
            .setRectangle(MarkerRectangle.newBuilder().setPoint1(point).setPoint2(point))
            .build();
        final Marker polyline = Marker.newBuilder()
            .setPolyline(MarkerPolyline.newBuilder().addLines(points))
            .build();
        final Marker polygon = Marker.newBuilder()
            .setPolygon(MarkerPolygon.newBuilder().addMainPolygon(point).addNegativeSpace(points))
            .build();
        final Marker multipolygon = Marker.newBuilder()
            .setMultiPolygon(MarkerMultiPolygon.newBuilder().addPolygons(polygon.getPolygon()))
            .build();
        final ChunkSnapshotBody body = ChunkSnapshotBody.newBuilder().addSections(ChunkSection.newBuilder()).build();
        final ChunkSnapshot snapshot = ChunkSnapshot.newBuilder()
            .setCompressedBody(body.toByteString())
            .setUncompressedLength(body.getSerializedSize())
            .setCrc32C(1)
            .build();
        final Envelope configEnvelope = Envelope.newBuilder().setConfigReplace(config).build();
        final Envelope playersEnvelope = Envelope.newBuilder().setPlayersReplace(players).build();
        final Envelope snapshotEnvelope = Envelope.newBuilder().setChunkSnapshot(snapshot).build();
        assertTrue(configEnvelope.hasConfigReplace());
        assertTrue(playersEnvelope.hasPlayersReplace());
        assertTrue(snapshotEnvelope.hasChunkSnapshot());
        assertFalse(player.hasX());
        assertTrue(marker.hasIcon());
        assertTrue(layer.getShowControls());
        assertTrue(layer.getVisible());
        assertEquals(-1.25, point.getX());
        assertEquals(2.5, point.getZ());
        assertTrue(circle.hasCircle());
        assertTrue(ellipse.hasEllipse());
        assertTrue(rectangle.hasRectangle());
        assertTrue(polyline.hasPolyline());
        assertTrue(polygon.hasPolygon());
        assertTrue(multipolygon.hasMultiPolygon());
    }
}
