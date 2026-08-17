package xyz.jpenilla.squaremap.common.task;

import java.awt.Color;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import xyz.jpenilla.squaremap.common.util.Util;

public final class UpdateMarkers {
    private UpdateMarkers() {
    }

    public static String document(final xyz.jpenilla.squaremap.bridge.v1.MarkerLayersReplace snapshot) {
        final List<Map<String, Object>> layers = new ArrayList<>();
        for (final xyz.jpenilla.squaremap.bridge.v1.MarkerLayer layer : snapshot.getLayersList()) {
            final Map<String, Object> value = new HashMap<>();
            value.put("id", layer.getId());
            value.put("name", layer.getLabel());
            value.put("control", layer.getShowControls());
            value.put("hide", layer.getDefaultHidden());
            value.put("order", layer.getLayerPriority());
            value.put("z_index", layer.getZIndex());
            value.put("timestamp", layer.getTimestamp());
            value.put("markers", layer.getMarkersList().stream().map(UpdateMarkers::legacyMarker).toList());
            layers.add(value);
        }
        return Util.gson().toJson(layers);
    }

    private static Map<String, Object> legacyMarker(final xyz.jpenilla.squaremap.bridge.v1.Marker marker) {
        final Map<String, Object> value = new HashMap<>();
        final xyz.jpenilla.squaremap.bridge.v1.MarkerStyle style = marker.getStyle();
        if (!style.getStroke()) value.put("stroke", false);
        if (!style.getStrokeColor().equals("#0000ff")) value.put("color", style.getStrokeColor());
        if (style.getStrokeWeight() != 3) value.put("weight", style.getStrokeWeight());
        if (style.getStrokeOpacity() != 1.0) value.put("opacity", style.getStrokeOpacity());
        if (!style.getFill()) value.put("fill", false);
        if (style.hasFillColor()) value.put("fillColor", style.getFillColor());
        if (style.getFillOpacity() != 0.2) value.put("fillOpacity", style.getFillOpacity());
        if (!style.getFillRule().equals("evenodd")) value.put("fillRule", style.getFillRule());
        if (marker.hasTooltip()) {
            if (marker.getTooltip().hasClick()) value.put("popup", marker.getTooltip().getClick());
            if (marker.getTooltip().hasHover()) value.put("tooltip", marker.getTooltip().getHover());
        }
        switch (marker.getGeometryCase()) {
            case ICON -> {
                final var icon = marker.getIcon();
                value.put("type", "icon");
                value.put("point", point(icon.getPoint()));
                value.put("size", Map.of("x", icon.getSizeX(), "z", icon.getSizeZ()));
                value.put("anchor", point(icon.getAnchor())); value.put("tooltip_anchor", point(icon.getTooltipAnchor()));
                value.put("icon", icon.getImage());
            }
            case CIRCLE -> { value.put("type", "circle"); value.put("center", point(marker.getCircle().getCenter())); value.put("radius", marker.getCircle().getRadius()); }
            case ELLIPSE -> { value.put("type", "ellipse"); value.put("center", point(marker.getEllipse().getCenter())); value.put("radiusX", marker.getEllipse().getRadiusX()); value.put("radiusZ", marker.getEllipse().getRadiusZ()); }
            case RECTANGLE -> { value.put("type", "rectangle"); value.put("points", List.of(point(marker.getRectangle().getPoint1()), point(marker.getRectangle().getPoint2()))); }
            case POLYLINE -> {
                value.put("type", "polyline");
                final List<List<Map<String, Integer>>> lines = marker.getPolyline().getLinesList().stream().map(UpdateMarkers::points).toList();
                value.put("points", lines.size() == 1 ? lines.get(0) : lines);
            }
            case POLYGON -> { value.put("type", "polygon"); value.put("points", polygon(marker.getPolygon())); }
            case MULTI_POLYGON -> { value.put("type", "polygon"); value.put("points", marker.getMultiPolygon().getPolygonsList().stream().map(UpdateMarkers::polygon).toList()); }
            case GEOMETRY_NOT_SET -> throw new IllegalArgumentException("marker geometry not set");
        }
        return value;
    }

    private static Map<String, Integer> point(final xyz.jpenilla.squaremap.bridge.v1.Point point) {
        return Map.of("x", point.getX(), "z", point.getZ());
    }
    private static List<Map<String, Integer>> points(final xyz.jpenilla.squaremap.bridge.v1.PointList points) {
        return points.getPointsList().stream().map(UpdateMarkers::point).toList();
    }
    private static List<Object> polygon(final xyz.jpenilla.squaremap.bridge.v1.MarkerPolygon polygon) {
        final List<Object> points = new ArrayList<>();
        points.add(polygon.getMainPolygonList().stream().map(UpdateMarkers::point).toList());
        points.addAll(polygon.getNegativeSpaceList().stream().map(UpdateMarkers::points).toList());
        return points;
    }
}
