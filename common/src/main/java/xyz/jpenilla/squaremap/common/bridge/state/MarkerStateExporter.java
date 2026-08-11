package xyz.jpenilla.squaremap.common.bridge.state;

import com.google.inject.Inject;
import java.awt.Color;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import xyz.jpenilla.squaremap.api.Key;
import xyz.jpenilla.squaremap.api.LayerProvider;
import xyz.jpenilla.squaremap.api.Point;
import xyz.jpenilla.squaremap.api.Registry;
import xyz.jpenilla.squaremap.api.marker.Circle;
import xyz.jpenilla.squaremap.api.marker.Ellipse;
import xyz.jpenilla.squaremap.api.marker.Icon;
import xyz.jpenilla.squaremap.api.marker.IPolygon;
import xyz.jpenilla.squaremap.api.marker.Marker;
import xyz.jpenilla.squaremap.api.marker.MarkerOptions;
import xyz.jpenilla.squaremap.api.marker.MultiPolygon;
import xyz.jpenilla.squaremap.api.marker.Polygon;
import xyz.jpenilla.squaremap.api.marker.Polyline;
import xyz.jpenilla.squaremap.api.marker.Rectangle;
import xyz.jpenilla.squaremap.bridge.v1.MarkerCircle;
import xyz.jpenilla.squaremap.bridge.v1.MarkerEllipse;
import xyz.jpenilla.squaremap.bridge.v1.MarkerIcon;
import xyz.jpenilla.squaremap.bridge.v1.MarkerLayer;
import xyz.jpenilla.squaremap.bridge.v1.MarkerLayersReplace;
import xyz.jpenilla.squaremap.bridge.v1.MarkerMultiPolygon;
import xyz.jpenilla.squaremap.bridge.v1.MarkerPolygon;
import xyz.jpenilla.squaremap.bridge.v1.MarkerPolyline;
import xyz.jpenilla.squaremap.bridge.v1.MarkerRectangle;
import xyz.jpenilla.squaremap.bridge.v1.MarkerStyle;
import xyz.jpenilla.squaremap.bridge.v1.MarkerTooltip;
import xyz.jpenilla.squaremap.bridge.v1.PointList;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;

/** Collects all marker geometry and options into an immutable protocol snapshot. */
public final class MarkerStateExporter {
    private MapWorldInternal mapWorld;
    private Object loadToken;
    private final WorldEpochRegistry epochs;
    private final BridgeRevisionClock revisions;
    private MarkerLayersReplace last;
    private final Map<Key, LayerState> layerState = new HashMap<>();

    @Inject
    public MarkerStateExporter(
        final MapWorldInternal mapWorld,
        final WorldEpochRegistry epochs,
        final BridgeRevisionClock revisions
    ) {
        this.mapWorld = mapWorld;
        this.epochs = epochs;
        this.revisions = revisions;
    }
    public synchronized void bind(final MapWorldInternal mapWorld) {
        final MapWorldInternal next = java.util.Objects.requireNonNull(mapWorld, "mapWorld");
        final Object nextToken = next.serverLevel();
        if (this.loadToken != null && this.loadToken != nextToken) {
            this.layerState.clear();
            this.last = null;
        }
        this.loadToken = nextToken;
        this.mapWorld = next;
    }
    synchronized void bindTokenForTesting(final Object token) {
        if (this.loadToken != null && this.loadToken != token) {
            this.layerState.clear();
            this.last = null;
        }
        this.loadToken = token;
    }

    synchronized void retainLayerForTesting(final Key key, final byte[] content, final long timestamp) {
        this.layerState.put(key, new LayerState(content, timestamp));
    }

    synchronized int retainedLayerCountForTesting() {
        return this.layerState.size();
    }
    synchronized long retainedTimestampForTesting(final Key key) {
        return this.layerState.get(key).timestamp();
    }
    public synchronized MarkerLayersReplace export() {
        final List<MarkerLayer> layers = new ArrayList<>();
        final Registry<LayerProvider> registry = this.mapWorld.layerRegistry();
        final List<Key> active = new ArrayList<>();
        final List<xyz.jpenilla.squaremap.api.Pair<Key, LayerProvider>> entries = new ArrayList<>();
        registry.entries().forEach(entries::add);
        entries.sort(java.util.Comparator.comparing(entry -> entry.left().getKey()));
        entries.forEach(entry -> {
                final Key key = entry.left();
                active.add(key);
                final LayerProvider provider = entry.right();
                final MarkerLayer content = content(key, provider);
                final byte[] encoded = content.toByteArray();
                final LayerState previous = this.layerState.get(key);
                final long timestamp;
                if (previous != null && Arrays.equals(previous.content(), encoded)) {
                    timestamp = previous.timestamp();
                } else if (previous == null) {
                    timestamp = System.currentTimeMillis();
                } else {
                    timestamp = Math.max(System.currentTimeMillis(), previous.timestamp() + 1);
                }
                this.layerState.put(key, new LayerState(encoded, timestamp));
                layers.add(content.toBuilder().setTimestamp(timestamp).build());
            });
        this.layerState.keySet().removeIf(key -> !active.contains(key));
        final long epoch = this.epochs.epoch(this.mapWorld.identifier(), this.mapWorld.serverLevel());
        final MarkerLayersReplace candidate = MarkerLayersReplace.newBuilder().addAllLayers(List.copyOf(layers))
            .setRevision(0)
            .setWorld(xyz.jpenilla.squaremap.bridge.v1.WorldIdentity.newBuilder()
                .setNamespace(this.mapWorld.identifier().namespace()).setValue(this.mapWorld.identifier().value()).setEpoch(epoch).build())
            .build();
        if (this.last != null && this.last.toBuilder().setRevision(0).build().equals(candidate)) return this.last;
        this.last = candidate.toBuilder().setRevision(this.revisions.next()).build();
        return this.last;
    }

    private static MarkerLayer content(final Key key, final LayerProvider provider) {
        final MarkerLayer.Builder layer = MarkerLayer.newBuilder()
            .setId(key.getKey()).setLabel(provider.getLabel())
            .setShowControls(provider.showControls()).setDefaultHidden(provider.defaultHidden())
            .setLayerPriority(provider.layerPriority()).setZIndex(provider.zIndex());
        for (final Marker marker : List.copyOf(provider.getMarkers())) layer.addMarkers(serialize(marker));
        return layer.build();
    }

    static MarkerLayersReplace snapshot(
        final String namespace,
        final String world,
        final long epoch,
        final Key key,
        final LayerProvider provider,
        final long timestamp
    ) {
        return MarkerLayersReplace.newBuilder()
            .setWorld(xyz.jpenilla.squaremap.bridge.v1.WorldIdentity.newBuilder().setNamespace(namespace).setValue(world).setEpoch(epoch))
            .addLayers(content(key, provider).toBuilder().setTimestamp(timestamp))
            .build();
    }

    public static xyz.jpenilla.squaremap.bridge.v1.Marker serialize(final Marker marker) {
        final xyz.jpenilla.squaremap.bridge.v1.Marker.Builder out = xyz.jpenilla.squaremap.bridge.v1.Marker.newBuilder().setStyle(style(marker.markerOptions()));
        final MarkerOptions options = marker.markerOptions();
        final MarkerTooltip.Builder tooltip = MarkerTooltip.newBuilder();
        if (options.clickTooltip() != null) tooltip.setClick(options.clickTooltip());
        if (options.hoverTooltip() != null) tooltip.setHover(options.hoverTooltip());
        if (options.clickTooltip() != null || options.hoverTooltip() != null) out.setTooltip(tooltip);
        if (marker instanceof Icon icon) {
            out.setIcon(MarkerIcon.newBuilder().setPoint(point(icon.point())).setTooltipAnchor(point(icon.tooltipAnchor()))
                .setAnchor(point(icon.anchor())).setImage(icon.image().getKey()).setSizeX(icon.sizeX()).setSizeZ(icon.sizeZ()));
        } else if (marker instanceof Circle circle) {
            out.setCircle(MarkerCircle.newBuilder().setCenter(point(circle.center())).setRadius(circle.radius()));
        } else if (marker instanceof Ellipse ellipse) {
            out.setEllipse(MarkerEllipse.newBuilder().setCenter(point(ellipse.center())).setRadiusX(ellipse.radiusX()).setRadiusZ(ellipse.radiusZ()));
        } else if (marker instanceof Rectangle rectangle) {
            out.setRectangle(MarkerRectangle.newBuilder().setPoint1(point(rectangle.point1())).setPoint2(point(rectangle.point2())));
        } else if (marker instanceof Polyline line) {
            final MarkerPolyline.Builder geometry = MarkerPolyline.newBuilder();
            line.points().forEach(points -> geometry.addLines(pointList(points)));
            out.setPolyline(geometry);
        } else if (marker instanceof Polygon polygon) {
            out.setPolygon(polygon(polygon));
        } else if (marker instanceof MultiPolygon multi) {
            final MarkerMultiPolygon.Builder geometry = MarkerMultiPolygon.newBuilder();
            multi.subPolygons().forEach(part -> geometry.addPolygons(polygon(part)));
            out.setMultiPolygon(geometry);
        } else {
            throw new IllegalArgumentException("unknown marker type " + marker.getClass().getName());
        }
        return out.build();
    }

    private static MarkerPolygon polygon(final IPolygon polygon) {
        final MarkerPolygon.Builder out = MarkerPolygon.newBuilder();
        polygon.mainPolygon().forEach(point -> out.addMainPolygon(point(point)));
        polygon.negativeSpace().forEach(points -> out.addNegativeSpace(pointList(points)));
        return out.build();
    }

    private static PointList pointList(final List<Point> points) {
        final PointList.Builder out = PointList.newBuilder();
        points.forEach(point -> out.addPoints(point(point)));
        return out.build();
    }

    private record LayerState(byte[] content, long timestamp) {}
    private static xyz.jpenilla.squaremap.bridge.v1.Point point(final Point point) {
        return xyz.jpenilla.squaremap.bridge.v1.Point.newBuilder().setX((int) point.x()).setZ((int) point.z()).build();
    }

    private static MarkerStyle style(final MarkerOptions options) {
        final MarkerStyle.Builder out = MarkerStyle.newBuilder().setStroke(options.stroke()).setStrokeColor(toHex(options.strokeColor()))
            .setStrokeWeight(options.strokeWeight()).setStrokeOpacity(options.strokeOpacity()).setFill(options.fill())
            .setFillOpacity(options.fillOpacity()).setFillRule(options.fillRule().toString().toLowerCase(Locale.ENGLISH));
        if (options.fillColor() != null) out.setFillColor(toHex(options.fillColor()));
        return out.build();
    }

    private static String toHex(final Color color) {
        return "#" + Integer.toHexString(color.getRGB()).substring(2);
    }
}
