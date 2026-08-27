package xyz.jpenilla.squaremap.test;

import java.awt.Color;
import java.awt.image.BufferedImage;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.logging.Level;
import java.util.logging.Logger;
import org.bukkit.World;
import org.bukkit.command.Command;
import org.bukkit.command.CommandSender;
import org.bukkit.command.PluginCommand;
import org.bukkit.event.EventHandler;
import org.bukkit.event.Listener;
import org.bukkit.event.world.WorldLoadEvent;
import org.bukkit.event.world.WorldUnloadEvent;
import org.bukkit.plugin.java.JavaPlugin;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.framework.qual.DefaultQualifier;
import xyz.jpenilla.squaremap.api.BukkitAdapter;
import xyz.jpenilla.squaremap.api.Key;
import xyz.jpenilla.squaremap.api.LayerProvider;
import xyz.jpenilla.squaremap.api.MapWorld;
import xyz.jpenilla.squaremap.api.Point;
import xyz.jpenilla.squaremap.api.SimpleLayerProvider;
import xyz.jpenilla.squaremap.api.Squaremap;
import xyz.jpenilla.squaremap.api.SquaremapProvider;
import xyz.jpenilla.squaremap.api.WorldIdentifier;
import xyz.jpenilla.squaremap.api.marker.Marker;
import xyz.jpenilla.squaremap.api.marker.MarkerOptions;

@DefaultQualifier(NonNull.class)
public final class SquaremapTestPlugin extends JavaPlugin implements Listener {

    private static final Key TEST_LAYER_KEY = Key.of("squaremap-test.test-layer");
    private static final Key TEST_ICON_KEY = Key.of("squaremap-test.test-icon");

    private final Logger logger = this.getLogger();
    private final AtomicBoolean apiAvailable = new AtomicBoolean(false);

    @Override
    public void onEnable() {
        this.getServer().getPluginManager().registerEvents(this, this);
        final PluginCommand command = this.getCommand("squaremaptest");
        if (command != null) {
            command.setExecutor(this::onCommand);
        }

        // Attempt to obtain the API now; if squaremap has not yet registered it
        // (it loads at STARTUP priority, before normal plugins), retry on world load.
        this.tryInitApi();

        // Periodically verify the API is still reachable and report state.
        this.getServer().getScheduler().runTaskTimer(this, this::selfCheck, 20L, 20L * 60L);
    }

    @Override
    public void onDisable() {
        if (this.apiAvailable.get()) {
            try {
                final Squaremap api = SquaremapProvider.get();
                if (api.iconRegistry().hasEntry(TEST_ICON_KEY)) {
                    api.iconRegistry().unregister(TEST_ICON_KEY);
                }
                for (final MapWorld world : api.mapWorlds()) {
                    if (world.layerRegistry().hasEntry(TEST_LAYER_KEY)) {
                        world.layerRegistry().unregister(TEST_LAYER_KEY);
                    }
                }
            } catch (final IllegalStateException ignored) {
                // squaremap already shut down; nothing to clean up
            }
        }
        this.apiAvailable.set(false);
    }

    private void tryInitApi() {
        if (this.apiAvailable.get()) {
            return;
        }
        final Squaremap api;
        try {
            api = SquaremapProvider.get();
        } catch (final IllegalStateException ex) {
            this.logger.log(Level.FINE, "squaremap API not yet available, will retry on world load", ex);
            return;
        }
        this.apiAvailable.set(true);
        this.logger.log(Level.INFO, "squaremap API available, registering test layer");

        // Register an icon so icon markers can reference it.
        final BufferedImage icon = new BufferedImage(16, 16, BufferedImage.TYPE_INT_ARGB);
        final var graphics = icon.createGraphics();
        graphics.setColor(Color.RED);
        graphics.fillOval(0, 0, 16, 16);
        graphics.dispose();
        if (api.iconRegistry().hasEntry(TEST_ICON_KEY)) {
            api.iconRegistry().unregister(TEST_ICON_KEY);
        }
        api.iconRegistry().register(TEST_ICON_KEY, icon);

        // Add markers to every currently enabled world.
        for (final MapWorld world : api.mapWorlds()) {
            this.registerMarkers(world);
        }
    }

    private void registerMarkers(final MapWorld world) {
        final SimpleLayerProvider layer = SimpleLayerProvider.builder("squaremap-test")
            .defaultHidden(false)
            .layerPriority(100)
            .zIndex(100)
            .build();

        final Point center = Point.of(0, 0);
        layer.addMarker(
            Key.of("squaremap-test.circle"),
            Marker.circle(center, 32)
                .markerOptions(MarkerOptions.builder()
                    .strokeColor(Color.RED)
                    .fillColor(new Color(255, 0, 0, 100))
                    .clickTooltip("<b>squaremap-test circle</b>")
                    .build())
        );
        layer.addMarker(
            Key.of("squaremap-test.rectangle"),
            Marker.rectangle(Point.of(-50, -50), Point.of(50, 50))
                .markerOptions(MarkerOptions.builder()
                    .strokeColor(Color.GREEN)
                    .fillColor(new Color(0, 255, 0, 60))
                    .build())
        );
        layer.addMarker(
            Key.of("squaremap-test.icon"),
            Marker.icon(Point.of(100, 100), TEST_ICON_KEY, 16)
        );

        if (world.layerRegistry().hasEntry(TEST_LAYER_KEY)) {
            world.layerRegistry().unregister(TEST_LAYER_KEY);
        }
        world.layerRegistry().register(TEST_LAYER_KEY, layer);
        this.logger.log(Level.INFO, "Registered test layer for world {0}", world.identifier().asString());
    }

    private void selfCheck() {
        if (!this.apiAvailable.get()) {
            this.tryInitApi();
            return;
        }
        try {
            final Squaremap api = SquaremapProvider.get();
            final int worldCount = api.mapWorlds().size();
            this.logger.log(Level.INFO, "squaremap-test self-check: {0} enabled worlds, API reachable", worldCount);
        } catch (final IllegalStateException ex) {
            this.apiAvailable.set(false);
            this.logger.log(Level.WARNING, "squaremap API became unavailable", ex);
        }
    }

    public boolean onCommand(final CommandSender sender, final Command command, final String label, final String[] args) {
        if (!this.apiAvailable.get()) {
            sender.sendMessage("squaremap API is not currently available");
            return true;
        }
        final Squaremap api = SquaremapProvider.get();
        sender.sendMessage("squaremap-test: " + api.mapWorlds().size() + " enabled worlds");
        for (final MapWorld world : api.mapWorlds()) {
            final WorldIdentifier id = world.identifier();
            final LayerProvider layer = world.layerRegistry().get(TEST_LAYER_KEY);
            sender.sendMessage("  " + id.asString() + " -> layer '" + layer.getLabel() + "', markers: " + layer.getMarkers().size());
        }
        return true;
    }

    @EventHandler
    public void onWorldLoad(final WorldLoadEvent event) {
        this.tryInitApi();
        // (Re)register markers for the newly loaded world if the API is up.
        if (this.apiAvailable.get()) {
            final World world = event.getWorld();
            final Squaremap api = SquaremapProvider.get();
            api.getWorldIfEnabled(BukkitAdapter.worldIdentifier(world)).ifPresent(this::registerMarkers);
        }
    }

    @EventHandler
    public void onWorldUnload(final WorldUnloadEvent event) {
        if (!this.apiAvailable.get()) {
            return;
        }
        final World world = event.getWorld();
        final Squaremap api = SquaremapProvider.get();
        api.getWorldIfEnabled(BukkitAdapter.worldIdentifier(world)).ifPresent(mapWorld -> {
            if (mapWorld.layerRegistry().hasEntry(TEST_LAYER_KEY)) {
                mapWorld.layerRegistry().unregister(TEST_LAYER_KEY);
            }
        });
    }
}
