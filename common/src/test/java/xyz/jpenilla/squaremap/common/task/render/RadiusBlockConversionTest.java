package xyz.jpenilla.squaremap.common.task.render;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.common.util.Numbers;

final class RadiusBlockConversionTest {
    @Test
    void paperRadiusRenderConvertsBlockCenterAndRadiusWithBlockToChunk() {
        assertEquals(16, Numbers.blockToChunk(256));
        assertEquals(50, Numbers.blockToChunk(800));
        assertEquals(-2, Numbers.blockToChunk(-17));
        assertEquals(1, Numbers.blockToChunk(16));
        assertEquals(0, Numbers.blockToChunk(1));
    }
}
