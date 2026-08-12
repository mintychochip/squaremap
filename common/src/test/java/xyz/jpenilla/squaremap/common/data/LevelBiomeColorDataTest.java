package xyz.jpenilla.squaremap.common.data;

import it.unimi.dsi.fastutil.objects.Reference2IntOpenHashMap;
import java.nio.file.Path;
import java.util.List;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;
import net.minecraft.SharedConstants;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.registries.Registries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.Biomes;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

class LevelBiomeColorDataTest {
    private static HolderLookup.RegistryLookup<Biome> biomes;
    private static LevelBiomeColorData.ColorTables tables;

    @BeforeAll
    static void bootstrap() {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        tables = LevelBiomeColorData.readImages(Path.of(System.getProperty("squaremap.task11.root"), "web", "public", "images"));
        biomes = VanillaRegistries.createLookup().lookupOrThrow(Registries.BIOME);
    }

    @Test
    void worldIndependentFactoryHonorsOverridesAndSpecialEffects() {
        final Holder<Biome> plains = biomes.getOrThrow(Biomes.PLAINS);
        final Holder<Biome> desert = biomes.getOrThrow(Biomes.DESERT);
        final Reference2IntOpenHashMap<Biome> empty = new Reference2IntOpenHashMap<>();
        final LevelBiomeColorData baseline = LevelBiomeColorData.create(
            List.of(plains.value(), desert.value()), empty, empty, empty, tables
        );
        assertNotEquals(baseline.grassColor(plains.value()), baseline.grassColor(desert.value()));

        final Reference2IntOpenHashMap<Biome> grassOverrides = new Reference2IntOpenHashMap<>();
        grassOverrides.put(plains.value(), 0);
        final LevelBiomeColorData overridden = LevelBiomeColorData.create(
            List.of(plains.value(), desert.value()), grassOverrides, empty, empty, tables
        );
        assertEquals(0, overridden.grassColor(plains.value()));
        assertThrows(IllegalStateException.class, () -> overridden.grassColor(biomes.getOrThrow(Biomes.BADLANDS).value()));
        assertEquals(baseline.grassColor(desert.value()), overridden.grassColor(desert.value()));
        final Reference2IntOpenHashMap<Biome> undeclared = new Reference2IntOpenHashMap<>();
        undeclared.put(desert.value(), 7);
        assertThrows(IllegalArgumentException.class, () -> LevelBiomeColorData.create(List.of(plains.value()), undeclared, empty, empty, tables));

        final Biome special = biomes.listElements()
            .map(Holder::value)
            .filter(biome -> biome.getSpecialEffects().grassColorOverride().isPresent())
            .findFirst()
            .orElse(null);
        if (special != null) {
            final LevelBiomeColorData specialData = LevelBiomeColorData.create(List.of(special), empty, empty, empty, tables);
            assertEquals(special.getSpecialEffects().grassColorOverride().get().intValue(), specialData.grassColors().getInt(special));
        }
        assertTrue(tables.grass().length == 65536 && tables.foliage().length == 65536);
    }
}
