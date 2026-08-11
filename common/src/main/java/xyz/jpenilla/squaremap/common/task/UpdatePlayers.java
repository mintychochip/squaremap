package xyz.jpenilla.squaremap.common.task;

import com.google.inject.Inject;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ForkJoinPool;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.framework.qual.DefaultQualifier;
import xyz.jpenilla.squaremap.common.httpd.JsonCache;
import xyz.jpenilla.squaremap.common.util.Util;

@DefaultQualifier(NonNull.class)
public final class UpdatePlayers {
    private static final String JSON_PATH = "/tiles/players.json";
    private final JsonCache jsonCache;

    @Inject
    private UpdatePlayers(final JsonCache jsonCache) {
        this.jsonCache = jsonCache;
    }
    /** Writes an already-collected immutable snapshot without recollecting server state. */
    public void publish(final xyz.jpenilla.squaremap.bridge.v1.PlayersReplace snapshot) {
        ForkJoinPool.commonPool().execute(() -> this.jsonCache.put(JSON_PATH, document(snapshot)));
    }

    public static String document(final xyz.jpenilla.squaremap.bridge.v1.PlayersReplace snapshot) {
        final List<Object> players = new ArrayList<>();
        for (final xyz.jpenilla.squaremap.bridge.v1.Player player : snapshot.getPlayersList()) {
            final Map<String, Object> entry = new HashMap<>();
            entry.put("name", player.getName());
            if (player.hasDisplayName()) entry.put("display_name", player.getDisplayName());
            entry.put("uuid", hex(player.getUuid().toByteArray()));
            entry.put("world", legacyWorldName(player.getWorld()));
            if (player.hasX()) entry.put("x", player.getX());
            if (player.hasY()) entry.put("y", player.getY());
            if (player.hasZ()) entry.put("z", player.getZ());
            if (player.hasYaw()) entry.put("yaw", player.getYaw());
            if (player.hasArmor()) entry.put("armor", player.getArmor());
            if (player.hasHealth()) entry.put("health", player.getHealth());
            players.add(entry);
        }
        return Util.gson().toJson(Map.of("players", players, "max", snapshot.getMaxPlayers()));
    }

    private static String hex(final byte[] bytes) {
        final StringBuilder out = new StringBuilder(bytes.length * 2);
        for (final byte value : bytes) out.append(String.format("%02x", value & 0xff));
        return out.toString();
    }
    private static String legacyWorldName(final xyz.jpenilla.squaremap.bridge.v1.WorldIdentity identity) {
        return identity.getNamespace() + "_" + identity.getValue();
    }

}
