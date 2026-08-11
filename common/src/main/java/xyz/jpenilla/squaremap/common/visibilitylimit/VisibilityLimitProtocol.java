package xyz.jpenilla.squaremap.common.visibilitylimit;

import java.util.Comparator;
import java.util.List;
import java.util.stream.Collectors;
import xyz.jpenilla.squaremap.bridge.v1.Point;
import xyz.jpenilla.squaremap.bridge.v1.VisibilityLimit;
import xyz.jpenilla.squaremap.bridge.v1.VisibilityLimitKind;

/** Converts the validated Java visibility shapes into bridge protocol values. */
public final class VisibilityLimitProtocol {
    private VisibilityLimitProtocol() {}

    public static List<VisibilityLimit> serialize(final List<VisibilityShape> shapes) {
        return shapes.stream().map(VisibilityLimitProtocol::serializeOne)
            .sorted(Comparator.comparing(VisibilityLimitProtocol::sortKey))
            .collect(Collectors.toUnmodifiableList());
    }

    private static VisibilityLimit serializeOne(final VisibilityShape shape) {
        if (shape instanceof WorldBorderShape) {
            return VisibilityLimit.newBuilder().setKind(VisibilityLimitKind.VISIBILITY_LIMIT_KIND_WORLD_BORDER).build();
        }
        if (shape instanceof CircleShape circle) {
            return VisibilityLimit.newBuilder().setKind(VisibilityLimitKind.VISIBILITY_LIMIT_KIND_CIRCLE)
                .setCenterX(circle.centerX()).setCenterZ(circle.centerZ()).setRadius(circle.radius()).build();
        }
        if (shape instanceof RectangleShape rectangle) {
            return VisibilityLimit.newBuilder().setKind(VisibilityLimitKind.VISIBILITY_LIMIT_KIND_RECTANGLE)
                .addPoints(Point.newBuilder().setX(rectangle.minBlockX()).setZ(rectangle.minBlockZ()))
                .addPoints(Point.newBuilder().setX(rectangle.maxBlockX()).setZ(rectangle.maxBlockZ())).build();
        }
        if (shape instanceof PolygonShape polygon) {
            final VisibilityLimit.Builder builder = VisibilityLimit.newBuilder().setKind(VisibilityLimitKind.VISIBILITY_LIMIT_KIND_POLYGON);
            for (final xyz.jpenilla.squaremap.api.Point point : polygon.points()) {
                builder.addPoints(Point.newBuilder().setX((int) point.x()).setZ((int) point.z()));
            }
            return builder.build();
        }
        throw new IllegalArgumentException("Unsupported visibility shape: " + shape.getClass().getName());
    }

    private static String sortKey(final VisibilityLimit limit) {
        return limit.getKindValue() + ":" + limit.getCenterX() + ":" + limit.getCenterZ() + ":" + limit.getRadius() + ":" + limit.getPointsList();
    }
}
