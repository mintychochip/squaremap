package xyz.jpenilla.squaremap.common.bridge.snapshot;

import com.google.protobuf.Descriptors;
import net.minecraft.SharedConstants;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.Registries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.Biomes;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.bridge.v1.BlockStateDescriptor;
import xyz.jpenilla.squaremap.bridge.v1.BlockTransparency;
import xyz.jpenilla.squaremap.bridge.v1.FluidClass;
import xyz.jpenilla.squaremap.bridge.v1.WorldIdentity;

import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;
final class RegistryDescriptorExporterTest {
    private static final WorldIdentity WORLD = WorldIdentity.newBuilder()
        .setNamespace("minecraft").setValue("overworld").setEpoch(1).build();

    @BeforeAll
    static void bootstrap() {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
    }

    @Test
    void blockDescriptorAirFieldIsBooleanAndRoundTripsBothValues() throws Exception {
        final Descriptors.FieldDescriptor air = BlockStateDescriptor.getDescriptor().findFieldByName("air");
        assertNotNull(air);
        assertEquals(8, air.getNumber());
        assertEquals(Descriptors.FieldDescriptor.JavaType.BOOLEAN, air.getJavaType());
        final BlockStateDescriptor trueValue = BlockStateDescriptor.newBuilder().setAir(true).build();
        final BlockStateDescriptor falseValue = BlockStateDescriptor.newBuilder().setAir(false).build();
        assertTrue(BlockStateDescriptor.parseFrom(trueValue.toByteArray()).getAir());
        assertFalse(BlockStateDescriptor.parseFrom(falseValue.toByteArray()).getAir());
    }

    @Test
    void fixtureRejectsDescriptorWhoseRegistryKeyDiffersFromActualState() {
        final BlockState state = Blocks.STONE.defaultBlockState();
        assertThrows(IllegalStateException.class, () -> RegistryDescriptorExporter.createFixture(
            WORLD, 1, List.of(new RegistryDescriptorExporter.FixtureState(state, descriptor("minecraft:dirt", List.of(state.toString())))),
            List.of(new RegistryDescriptorExporter.FixtureBiome(plains(), biomeDescriptor("minecraft:plains")))
        ));
    }

    @Test
    void fixtureRejectsDescriptorWhosePropertyKeyDiffersFromActualState() {
        final BlockState state = Blocks.STONE.defaultBlockState();
        assertThrows(IllegalStateException.class, () -> RegistryDescriptorExporter.createFixture(
            WORLD, 1, List.of(new RegistryDescriptorExporter.FixtureState(state, descriptor("minecraft:stone", List.of("wrong")))),
            List.of(new RegistryDescriptorExporter.FixtureBiome(plains(), biomeDescriptor("minecraft:plains")))
        ));
    }

    @Test
    void fixtureRejectsBiomeRegistryKeyMismatch() {
        final BlockState state = Blocks.STONE.defaultBlockState();
        assertThrows(IllegalStateException.class, () -> RegistryDescriptorExporter.createFixture(
            WORLD, 1, List.of(new RegistryDescriptorExporter.FixtureState(state, descriptor("minecraft:stone", List.of(state.toString())))),
            List.of(new RegistryDescriptorExporter.FixtureBiome(plains(), biomeDescriptor("minecraft:dirt")))
        ));
    }

    @Test
    void fixtureAssignsStableIdsAndRetainsHolderIdentityAcrossInputOrder() {
        final BlockState stone = Blocks.STONE.defaultBlockState();
        final BlockState dirt = Blocks.DIRT.defaultBlockState();
        final Holder<Biome> plains = plains();
        final Holder<Biome> desert = biome(Biomes.DESERT);
        final RegistryDescriptorExporter first = RegistryDescriptorExporter.createFixture(WORLD, 7,
            List.of(new RegistryDescriptorExporter.FixtureState(dirt, descriptor("minecraft:dirt", List.of(dirt.toString()))),
                new RegistryDescriptorExporter.FixtureState(stone, descriptor("minecraft:stone", List.of(stone.toString())))),
            List.of(new RegistryDescriptorExporter.FixtureBiome(desert, biomeDescriptor("minecraft:desert")),
                new RegistryDescriptorExporter.FixtureBiome(plains, biomeDescriptor("minecraft:plains"))));
        final RegistryDescriptorExporter second = RegistryDescriptorExporter.createFixture(WORLD, 7,
            List.of(new RegistryDescriptorExporter.FixtureState(stone, descriptor("minecraft:stone", List.of(stone.toString()))),
                new RegistryDescriptorExporter.FixtureState(dirt, descriptor("minecraft:dirt", List.of(dirt.toString())))),
            List.of(new RegistryDescriptorExporter.FixtureBiome(plains, biomeDescriptor("minecraft:plains")),
                new RegistryDescriptorExporter.FixtureBiome(desert, biomeDescriptor("minecraft:desert"))));
        assertEquals(first.snapshot(), second.snapshot());
        assertEquals(first.blockId(stone), second.blockId(stone));
        assertEquals(first.biomeId(plains), second.biomeId(plains));
    }

    @Test
    void fixtureRejectsDuplicateActualStatesAndHoldersAndUnknownLookups() {
        final BlockState stone = Blocks.STONE.defaultBlockState();
        final Holder<Biome> plains = plains();
        final var state = new RegistryDescriptorExporter.FixtureState(stone, descriptor("minecraft:stone", List.of(stone.toString())));
        final var biome = new RegistryDescriptorExporter.FixtureBiome(plains, biomeDescriptor("minecraft:plains"));
        assertThrows(IllegalStateException.class, () -> RegistryDescriptorExporter.createFixture(WORLD, 1, List.of(state, state), List.of(biome)));
        assertThrows(IllegalStateException.class, () -> RegistryDescriptorExporter.createFixture(WORLD, 1, List.of(state), List.of(biome, biome)));
        assertThrows(IllegalStateException.class, () -> RegistryDescriptorExporter.createFixture(WORLD, 1, List.of(state),
            List.of(biome, new RegistryDescriptorExporter.FixtureBiome(plains(), biomeDescriptor("minecraft:plains")))));
        final RegistryDescriptorExporter exporter = RegistryDescriptorExporter.createFixture(WORLD, 1, List.of(state), List.of(biome));
        assertThrows(IllegalStateException.class, () -> exporter.blockId(Blocks.DIRT.defaultBlockState()));
        assertThrows(IllegalStateException.class, () -> exporter.biomeId(biome(Biomes.DESERT)));
    }

    private static Holder<Biome> plains() { return biome(Biomes.PLAINS); }
    private static Holder<Biome> biome(final net.minecraft.resources.ResourceKey<Biome> key) {
        return VanillaRegistries.createLookup().lookupOrThrow(Registries.BIOME).getOrThrow(key);
    }
    private static RegistryDescriptorExporter.BlockDescriptor descriptor(final String id, final List<String> properties) {
        return new RegistryDescriptorExporter.BlockDescriptor(id, properties, 0xff777777,
            BlockTransparency.BLOCK_TRANSPARENCY_OPAQUE, false, 0, FluidClass.FLUID_CLASS_NONE, false, 0);
    }
    private static RegistryDescriptorExporter.BiomeDescriptorInput biomeDescriptor(final String id) {
        return new RegistryDescriptorExporter.BiomeDescriptorInput(id, 1, 2, 3, 0,
            xyz.jpenilla.squaremap.bridge.v1.GrassColorModifier.GRASS_COLOR_MODIFIER_NONE);
    }
}
