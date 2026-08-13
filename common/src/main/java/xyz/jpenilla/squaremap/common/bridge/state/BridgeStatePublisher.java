package xyz.jpenilla.squaremap.common.bridge.state;

import com.google.inject.Singleton;

import com.google.inject.Inject;
import com.google.inject.Provider;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.common.bridge.process.BackendMode;
import xyz.jpenilla.squaremap.common.bridge.process.BridgeBootstrapConfig;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Objects;
import java.util.function.Consumer;
import java.util.function.Supplier;
import xyz.jpenilla.squaremap.bridge.v1.MarkerLayersReplace;
import xyz.jpenilla.squaremap.bridge.v1.PlayersReplace;
import xyz.jpenilla.squaremap.bridge.v1.WorldStateReplace;
import xyz.jpenilla.squaremap.bridge.v1.IconsReplace;
import xyz.jpenilla.squaremap.common.bridge.outbox.BridgeEvent;
import xyz.jpenilla.squaremap.common.bridge.process.SidecarSupervisor;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.common.data.MapWorldInternal;

@Singleton
public final class BridgeStatePublisher {
    private final BackendMode mode;
    private final Supplier<WorldStateReplace> worlds;
    private final Supplier<PlayersReplace> players;
    private final MarkerRouter markers;
    private final Consumer<WorldStateReplace> legacyWorlds;
    private final Consumer<PlayersReplace> legacyPlayers;
    private final SnapshotSink<MarkerLayersReplace> legacyMarkers;
    private final Consumer<WorldStateReplace> bridgeWorlds;
    private final Consumer<PlayersReplace> bridgePlayers;
    private final Consumer<MarkerLayersReplace> bridgeMarkers;
    private Consumer<IconsReplace> bridgeIcons;
    private WorldStateReplace publishedWorlds;
    private PlayersReplace publishedPlayers;
    private final Map<String, MarkerLayersReplace> publishedMarkers = new HashMap<>();
    private IconsReplace publishedIcons;
    public BridgeStatePublisher(
        final BackendMode mode,
        final Supplier<WorldStateReplace> worlds,
        final Supplier<PlayersReplace> players,
        final FunctionWorld<MarkerLayersReplace> markers,
        final Consumer<WorldStateReplace> legacyWorlds,
        final Consumer<PlayersReplace> legacyPlayers,
        final SnapshotSink<MarkerLayersReplace> legacyMarkers,
        final Consumer<WorldStateReplace> bridgeWorlds,
        final Consumer<PlayersReplace> bridgePlayers,
        final Consumer<MarkerLayersReplace> bridgeMarkers
    ) {
        this.mode = Objects.requireNonNull(mode, "mode");
        this.worlds = Objects.requireNonNull(worlds, "worlds");
        this.players = Objects.requireNonNull(players, "players");
        this.markers = markers instanceof MarkerRouter router ? router : MarkerRouter.wrap(Objects.requireNonNull(markers, "markers"));
        this.legacyWorlds = Objects.requireNonNull(legacyWorlds, "legacyWorlds");
        this.legacyPlayers = Objects.requireNonNull(legacyPlayers, "legacyPlayers");
        this.legacyMarkers = Objects.requireNonNull(legacyMarkers, "legacyMarkers");
        this.bridgeWorlds = Objects.requireNonNull(bridgeWorlds, "bridgeWorlds");
        this.bridgePlayers = Objects.requireNonNull(bridgePlayers, "bridgePlayers");
        this.bridgeMarkers = Objects.requireNonNull(bridgeMarkers, "bridgeMarkers");
        this.bridgeIcons = ignored -> {};
    }
    @Inject
    public BridgeStatePublisher(
        final Provider<WorldStateExporter> worlds,
        final Provider<PlayerStateExporter> players,
        final Provider<xyz.jpenilla.squaremap.common.task.UpdateWorldData> legacyWorlds,
        final Provider<xyz.jpenilla.squaremap.common.task.UpdatePlayers> legacyPlayers,
        final xyz.jpenilla.squaremap.common.task.TaskFactory markers,
        final BridgeBootstrapConfig config,
        final SidecarSupervisor supervisor,
        final WorldEpochRegistry epochs,
        final BridgeRevisionClock revisions
    ) {
        this(config.backendMode(), worlds.get()::export, players.get()::export, markerExporter(epochs, revisions),
            legacyWorlds.get()::publish, legacyPlayers.get()::publish,
            (world, snapshot) -> markers.createUpdateMarkers(world).publish(snapshot),
            payload -> supervisor.publish(new BridgeEvent.ReplaceState("worlds", Envelope.newBuilder().setWorldStateReplace(payload).build())),
            payload -> supervisor.publish(new BridgeEvent.ReplaceState("players", Envelope.newBuilder().setPlayersReplace(payload).build())),
            payload -> supervisor.publish(new BridgeEvent.ReplaceState("markers:" + payload.getWorld().getNamespace() + ":" + payload.getWorld().getValue(),
                Envelope.newBuilder().setMarkerLayersReplace(payload).build())));
        this.bridgeIcons = payload -> supervisor.publish(new BridgeEvent.ReplaceState("icons", Envelope.newBuilder().setIconsReplace(payload).build()));
        supervisor.setReconnectListener(connection -> {
            this.bridgeWorlds.accept(this.publishedWorlds == null ? this.worlds.get() : this.publishedWorlds);
            this.bridgePlayers.accept(this.publishedPlayers == null ? this.players.get() : this.publishedPlayers);
            this.publishedMarkers.values().forEach(this.bridgeMarkers);
            if (this.publishedIcons != null) this.bridgeIcons.accept(this.publishedIcons);
        });
    }

    private static MarkerRouter markerExporter(final WorldEpochRegistry epochs, final BridgeRevisionClock revisions) {
        return new MarkerRouter(epochs, revisions);
    }
    public synchronized void publishIcons(final IconsReplace snapshot) {
        if (this.publishedIcons != null && this.publishedIcons.toBuilder().setRevision(0).build().equals(snapshot.toBuilder().setRevision(0).build())) return;
        this.publishedIcons = snapshot;
        if (this.mode != BackendMode.JAVA) this.bridgeIcons.accept(snapshot);
    }
    public static BridgeStatePublisher forTesting(final BackendMode mode, final Supplier<PlayersReplace> players,
                                                   final Consumer<PlayersReplace> legacy, final Consumer<PlayersReplace> bridge) {
        return new BridgeStatePublisher(mode, WorldStateReplace::getDefaultInstance, players, world -> MarkerLayersReplace.getDefaultInstance(),
            ignored -> {}, legacy, (world, value) -> {}, ignored -> {}, bridge, ignored -> {});
    }

    public synchronized void publishWorlds() {
        final WorldStateReplace value = this.worlds.get();
        if (this.publishedWorlds != null && this.publishedWorlds.toBuilder().setRevision(0).build().equals(value.toBuilder().setRevision(0).build())) return;
        this.publishedWorlds = value;
        this.markers.prune(value);
        if (this.mode != BackendMode.RUST) this.legacyWorlds.accept(value);
        if (this.mode != BackendMode.JAVA) this.bridgeWorlds.accept(value);
    }

    public synchronized void publishPlayers() {
        final PlayersReplace value = this.players.get();
        if (this.publishedPlayers != null && this.publishedPlayers.toBuilder().setRevision(0).build().equals(value.toBuilder().setRevision(0).build())) return;
        this.publishedPlayers = value;
        if (this.mode != BackendMode.RUST) this.legacyPlayers.accept(value);
        if (this.mode != BackendMode.JAVA) this.bridgePlayers.accept(value);
    }

    public synchronized void publishMarkers(final MapWorldInternal world) {
        final MarkerLayersReplace value = this.markers.apply(world);
        final String key = value.getWorld().getNamespace() + ":" + value.getWorld().getValue();
        if (this.publishedMarkers.containsKey(key)
            && this.publishedMarkers.get(key).toBuilder().setRevision(0).build().equals(value.toBuilder().setRevision(0).build())) return;
        this.publishedMarkers.put(key, value);
        if (this.mode != BackendMode.RUST) this.legacyMarkers.accept(world, value);
        if (this.mode != BackendMode.JAVA) this.bridgeMarkers.accept(value);
    }

    private static final class MarkerRouter implements FunctionWorld<MarkerLayersReplace> {
        private final WorldEpochRegistry epochs;
        private final BridgeRevisionClock revisions;
        private final FunctionWorld<MarkerLayersReplace> delegate;
        private final Map<WorldIdentifier, MarkerStateExporter> exporters = new HashMap<>();

        private MarkerRouter(final WorldEpochRegistry epochs, final BridgeRevisionClock revisions) {
            this.epochs = epochs;
            this.revisions = revisions;
            this.delegate = null;
        }

        private MarkerRouter(final FunctionWorld<MarkerLayersReplace> delegate) {
            this.epochs = null;
            this.revisions = null;
            this.delegate = delegate;
        }

        private static MarkerRouter wrap(final FunctionWorld<MarkerLayersReplace> delegate) {
            return new MarkerRouter(delegate);
        }

        @Override
        public synchronized MarkerLayersReplace apply(final MapWorldInternal world) {
            if (this.delegate != null) return this.delegate.apply(world);
            final WorldIdentifier identifier = world.identifier();
            final MarkerStateExporter exporter = this.exporters.computeIfAbsent(identifier,
                ignored -> new MarkerStateExporter(world, this.epochs, this.revisions));
            exporter.bind(world);
            return exporter.export();
        }

        private synchronized void prune(final WorldStateReplace snapshot) {
            if (this.delegate != null) return;
            final java.util.Set<WorldIdentifier> active = new HashSet<>();
            snapshot.getWorldsList().forEach(world -> {
                if (world.hasIdentity()) active.add(WorldIdentifier.create(world.getIdentity().getNamespace(), world.getIdentity().getValue()));
            });
            this.exporters.keySet().removeIf(identifier -> !active.contains(identifier));
        }
    }

    @FunctionalInterface
    public interface FunctionWorld<T> {
        T apply(MapWorldInternal world);
    }

    @FunctionalInterface
    public interface SnapshotSink<T> {
        void accept(MapWorldInternal world, T snapshot);
    }
}
