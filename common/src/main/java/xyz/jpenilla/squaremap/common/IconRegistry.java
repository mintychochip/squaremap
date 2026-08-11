package xyz.jpenilla.squaremap.common;

import com.google.inject.Inject;
import com.google.inject.Singleton;

import java.awt.image.BufferedImage;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Consumer;
import javax.imageio.ImageIO;
import org.checkerframework.checker.nullness.qual.NonNull;
import org.checkerframework.framework.qual.DefaultQualifier;
import xyz.jpenilla.squaremap.api.Key;
import xyz.jpenilla.squaremap.api.Pair;
import xyz.jpenilla.squaremap.api.Registry;
import xyz.jpenilla.squaremap.common.bridge.process.BridgeBootstrapConfig;
import xyz.jpenilla.squaremap.common.data.DirectoryProvider;
import xyz.jpenilla.squaremap.common.util.FileUtil;

@Singleton
@DefaultQualifier(NonNull.class)
public final class IconRegistry implements Registry<BufferedImage> {
    private final Map<Key, BufferedImage> images = new ConcurrentHashMap<>();
    private final Path directory;
    private volatile Consumer<xyz.jpenilla.squaremap.bridge.v1.IconsReplace> bridgeSink = ignored -> {};
    private final boolean legacyWrites;
    private final xyz.jpenilla.squaremap.common.bridge.state.BridgeRevisionClock revisions;

    public IconRegistry(final DirectoryProvider directoryProvider) {
        this(directoryProvider, true, new xyz.jpenilla.squaremap.common.bridge.state.BridgeRevisionClock());
    }

    public IconRegistry(final DirectoryProvider directoryProvider, final boolean legacyWrites) {
        this(directoryProvider, legacyWrites, new xyz.jpenilla.squaremap.common.bridge.state.BridgeRevisionClock());
    }

    @Inject
    public IconRegistry(
        final DirectoryProvider directoryProvider,
        final BridgeBootstrapConfig bridgeConfig,
        final xyz.jpenilla.squaremap.common.bridge.state.BridgeRevisionClock revisions
    ) {
        this(directoryProvider, bridgeConfig.backendMode() != xyz.jpenilla.squaremap.common.bridge.process.BackendMode.RUST, revisions);
    }

    private IconRegistry(
        final DirectoryProvider directoryProvider,
        final boolean legacyWrites,
        final xyz.jpenilla.squaremap.common.bridge.state.BridgeRevisionClock revisions
    ) {
        this.revisions = revisions;
        this.directory = directoryProvider.webDirectory().resolve("images/icon/registered/");
        this.legacyWrites = legacyWrites;
        if (!legacyWrites) {
            return;
        }
        try {
            if (Files.exists(this.directory)) {
                FileUtil.deleteRecursively(this.directory);
            }
            Files.createDirectories(this.directory);
        } catch (final IOException e) {
            throw failedToCreateRegistry(e);
        }
    }

    @Override
    public void register(final Key key, final BufferedImage value) {
        if (this.hasEntry(key)) {
            throw imageAlreadyRegistered(key);
        }
        if (this.legacyWrites) {
            try {
                ImageIO.write(value, "png", this.directory.resolve(key.getKey() + ".png").toFile());
            } catch (IOException e) {
                throw failedToWriteImage(key, e);
            }
        }
        this.images.put(key, value);
        this.bridgeSink.accept(this.bridgeSnapshot());
    }

    @Override
    public void unregister(final Key key) {
        final BufferedImage removed = this.images.get(key);
        if (removed == null) {
            throw noImageRegistered(key);
        }
        if (this.legacyWrites) {
            try {
                Files.deleteIfExists(this.directory.resolve(key.getKey() + ".png"));
            } catch (IOException e) {
                throw failedToWriteImage(key, e);
            }
        }
        this.images.remove(key, removed);
        this.bridgeSink.accept(this.bridgeSnapshot());
    }

    @Override
    public boolean hasEntry(final Key key) {
        return this.images.containsKey(key);
    }

    @Override
    public BufferedImage get(final Key key) {
        final BufferedImage provider = this.images.get(key);
        if (provider == null) {
            throw noImageRegistered(key);
        }
        return provider;
    }

    @Override
    public Iterable<Pair<Key, BufferedImage>> entries() {
        return this.images.entrySet().stream()
            .map(entry -> Pair.of(entry.getKey(), entry.getValue()))
            .toList();
    }
    public void setBridgeSink(final Consumer<xyz.jpenilla.squaremap.bridge.v1.IconsReplace> bridgeSink) {
        this.bridgeSink = java.util.Objects.requireNonNull(bridgeSink, "bridgeSink");
    }

    public xyz.jpenilla.squaremap.bridge.v1.IconsReplace bridgeSnapshot() {
        return xyz.jpenilla.squaremap.common.bridge.state.IconStateExporter.snapshot(this.entries(), this.revisions.next());
    }

    private static IllegalArgumentException failedToCreateRegistry(final IOException e) {
        return new IllegalArgumentException("Failed to setup icon registry", e);
    }

    private static IllegalArgumentException failedToWriteImage(final Key key, final IOException e) {
        return new IllegalArgumentException(String.format("Failed to write image for key '%s'", key.getKey()), e);
    }

    private static IllegalArgumentException noImageRegistered(final Key key) {
        return new IllegalArgumentException(String.format("No image registered for key '%s'", key.getKey()));
    }

    private static IllegalArgumentException imageAlreadyRegistered(final Key key) {
        throw new IllegalArgumentException(String.format("Image already registered for key '%s'", key.getKey()));
    }
}
