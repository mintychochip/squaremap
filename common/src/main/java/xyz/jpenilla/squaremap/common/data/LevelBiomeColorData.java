package xyz.jpenilla.squaremap.common.data;

import it.unimi.dsi.fastutil.objects.Reference2IntMap;
import it.unimi.dsi.fastutil.objects.Reference2IntMaps;
import it.unimi.dsi.fastutil.objects.Reference2IntOpenHashMap;
import java.awt.image.BufferedImage;
import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import javax.imageio.ImageIO;
import com.mojang.serialization.JsonOps;
import com.mojang.serialization.Dynamic;
import net.minecraft.util.Mth;
import net.minecraft.world.level.biome.Biome;
import xyz.jpenilla.squaremap.common.util.Colors;
import xyz.jpenilla.squaremap.common.util.Util;

public record LevelBiomeColorData(
    Reference2IntMap<Biome> grassColors,
    Reference2IntMap<Biome> foliageColors,
    Reference2IntMap<Biome> waterColors
) {
    private static volatile ColorTables DEFAULT_TABLES;

    public static final class ColorTables {
        private final int[] grass;
        private final int[] foliage;

        private ColorTables(final int[] grass, final int[] foliage) {
            if (grass.length != 256 * 256 || foliage.length != 256 * 256) {
                throw new IllegalArgumentException("biome color tables must contain exactly 65536 entries");
            }
            this.grass = grass.clone();
            this.foliage = foliage.clone();
        }

        public int[] grass() { return this.grass.clone(); }
        public int[] foliage() { return this.foliage.clone(); }
        private int grass(final int index) { return this.grass[index]; }
        private int foliage(final int index) { return this.foliage[index]; }
    }

    public static void loadImages(final DirectoryProvider directoryProvider) {
        DEFAULT_TABLES = readImages(directoryProvider.webDirectory().resolve("images"));
    }

    public static ColorTables readImages(final Path imagesDir) {
        final BufferedImage imgGrass;
        final BufferedImage imgFoliage;
        try {
            imgGrass = ImageIO.read(imagesDir.resolve("grass.png").toFile());
            imgFoliage = ImageIO.read(imagesDir.resolve("foliage.png").toFile());
        } catch (final IOException e) {
            throw new IllegalStateException("Failed to read biome images", e);
        }
        if (imgGrass == null || imgFoliage == null
            || imgGrass.getWidth() != 256 || imgGrass.getHeight() != 256
            || imgFoliage.getWidth() != 256 || imgFoliage.getHeight() != 256) {
            throw new IllegalArgumentException("biome color images must be present and exactly 256x256");
        }
        return new ColorTables(toArray(imgGrass), toArray(imgFoliage));
    }

    public static LevelBiomeColorData create(final MapWorldInternal world) {
        return create(Util.biomeRegistry(world.serverLevel()),
            world.advanced().COLOR_OVERRIDES_BIOME_GRASS,
            world.advanced().COLOR_OVERRIDES_BIOME_FOLIAGE,
            world.advanced().COLOR_OVERRIDES_BIOME_WATER,
            requireDefaultTables());
    }

    public static LevelBiomeColorData create(final Iterable<Biome> biomes,
        final Reference2IntMap<Biome> grassOverrides,
        final Reference2IntMap<Biome> foliageOverrides,
        final Reference2IntMap<Biome> waterOverrides,
        final ColorTables tables) {
        final List<Biome> declared = new ArrayList<>();
        biomes.forEach(declared::add);
        final Reference2IntMap<Biome> grassColors = new Reference2IntOpenHashMap<>();
        final Reference2IntMap<Biome> foliageColors = new Reference2IntOpenHashMap<>();
        final Reference2IntMap<Biome> waterColors = new Reference2IntOpenHashMap<>();
        for (final Biome biome : declared) {
            final float temperature = Mth.clamp(biome.getBaseTemperature(), 0.0F, 1.0F);
            final float humidity = Mth.clamp(downfall(biome), 0.0F, 1.0F);
            grassColors.put(biome, biome.getSpecialEffects().grassColorOverride()
                .orElse(defaultGrassColor(temperature, humidity, tables)).intValue());
            foliageColors.put(biome, biome.getSpecialEffects().foliageColorOverride()
                .orElse(Colors.mix(Colors.plantMapColor(), defaultFoliageColor(temperature, humidity, tables), 0.85f)).intValue());
            waterColors.put(biome, biome.getSpecialEffects().waterColor());
        }
        validateOverrides(declared, grassOverrides, "grass");
        validateOverrides(declared, foliageOverrides, "foliage");
        validateOverrides(declared, waterOverrides, "water");
        grassColors.putAll(grassOverrides);
        foliageColors.putAll(foliageOverrides);
        waterColors.putAll(waterOverrides);
        return new LevelBiomeColorData(Reference2IntMaps.unmodifiable(grassColors),
            Reference2IntMaps.unmodifiable(foliageColors), Reference2IntMaps.unmodifiable(waterColors));
    }
    public int grassColor(final Biome biome) { return requiredColor(this.grassColors, biome, "grass"); }
    public int foliageColor(final Biome biome) { return requiredColor(this.foliageColors, biome, "foliage"); }
    public int waterColor(final Biome biome) { return requiredColor(this.waterColors, biome, "water"); }
    private static int requiredColor(final Reference2IntMap<Biome> colors, final Biome biome, final String category) {
        if (!colors.containsKey(biome)) throw new IllegalStateException("missing " + category + " biome color");
        return colors.getInt(biome);
    }
    private static void validateOverrides(final List<Biome> declared, final Reference2IntMap<Biome> overrides, final String category) {
        for (final Biome biome : overrides.keySet()) if (!declared.contains(biome)) throw new IllegalArgumentException("undeclared " + category + " biome override");
    }

    private static ColorTables requireDefaultTables() {
        final ColorTables tables = DEFAULT_TABLES;
        if (tables == null) throw new IllegalStateException("biome color images have not been loaded");
        return tables;
    }

    private static float downfall(final Biome biome) {
        return Biome.NETWORK_CODEC.encodeStart(JsonOps.INSTANCE, biome)
            .flatMap(value -> new Dynamic<>(JsonOps.INSTANCE, value).get("downfall").asNumber())
            .getOrThrow(error -> new IllegalStateException("Failed to encode biome downfall: " + error)).floatValue();
    }

    private static int[] toArray(final BufferedImage image) {
        final int[] array = new int[256 * 256];
        for (int x = 0; x < 256; ++x) for (int y = 0; y < 256; ++y) {
            final int color = image.getRGB(x, y);
            array[x + y * 256] = (0xFF << 24) | (color & 0x00FFFFFF);
        }
        return array;
    }

    private static int defaultGrassColor(final double temperature, final double humidity, final ColorTables tables) {
        final int j = (int) ((1.0 - humidity * temperature) * 255.0);
        final int i = (int) ((1.0 - temperature) * 255.0);
        final int k = j << 8 | i;
        if (k >= 256 * 256) return 0;
        return tables.grass(k);
    }

    private static int defaultFoliageColor(final double temperature, final double humidity, final ColorTables tables) {
        final int i = (int) ((1.0 - temperature) * 255.0);
        final int j = (int) ((1.0 - humidity * temperature) * 255.0);
        return tables.foliage(j << 8 | i);
    }
}
