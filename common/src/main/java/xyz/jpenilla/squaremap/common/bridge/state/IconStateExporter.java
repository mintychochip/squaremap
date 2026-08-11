package xyz.jpenilla.squaremap.common.bridge.state;

import com.google.protobuf.ByteString;
import java.awt.image.BufferedImage;
import java.util.ArrayList;
import java.util.List;
import xyz.jpenilla.squaremap.api.Key;
import xyz.jpenilla.squaremap.api.Pair;
import xyz.jpenilla.squaremap.bridge.v1.Icon;
import xyz.jpenilla.squaremap.bridge.v1.IconsReplace;

/** Normalizes registered images to immutable row-major RGBA snapshots. */
public final class IconStateExporter {
    private IconStateExporter() {}

    public static IconsReplace snapshot(final Iterable<Pair<Key, BufferedImage>> entries) {
        return snapshot(entries, 1);
    }

    public static IconsReplace snapshot(final Iterable<Pair<Key, BufferedImage>> entries, final long revision) {
        final List<Icon> icons = new ArrayList<>();
        for (final Pair<Key, BufferedImage> entry : entries) icons.add(icon(entry.left(), entry.right()));
        icons.sort(java.util.Comparator.comparing(Icon::getId));
        return IconsReplace.newBuilder().addAllIcons(List.copyOf(icons)).setRevision(revision).build();
    }

    public static Icon icon(final Key key, final BufferedImage image) {
        final byte[] rgba = new byte[image.getWidth() * image.getHeight() * 4];
        int offset = 0;
        for (int y = 0; y < image.getHeight(); y++) {
            for (int x = 0; x < image.getWidth(); x++) {
                final int argb = image.getRGB(x, y);
                rgba[offset++] = (byte) ((argb >>> 16) & 0xff);
                rgba[offset++] = (byte) ((argb >>> 8) & 0xff);
                rgba[offset++] = (byte) (argb & 0xff);
                rgba[offset++] = (byte) ((argb >>> 24) & 0xff);
            }
        }
        return Icon.newBuilder().setId(key.getKey()).setImage(ByteString.copyFrom(rgba)).setMimeType("image/rgba")
            .setWidth(image.getWidth()).setHeight(image.getHeight()).build();
    }
}
