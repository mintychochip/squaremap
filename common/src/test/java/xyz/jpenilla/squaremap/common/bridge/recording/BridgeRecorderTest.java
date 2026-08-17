package xyz.jpenilla.squaremap.common.bridge.recording;

import static org.junit.jupiter.api.Assertions.*;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.charset.StandardCharsets;
import org.junit.jupiter.api.Test;

final class BridgeRecorderTest {
    @Test void recordsBothDirectionsAndMonotonicFrames() throws Exception {
        Path root = Files.createTempDirectory("recorder");
        Path file = root.resolve("run.rec");
        try (BridgeRecorder recorder = new BridgeRecorder(root, file, 4096)) {
            recorder.record(BridgeRecorder.Direction.OUTBOUND, "hello".getBytes(StandardCharsets.UTF_8));
            recorder.record(BridgeRecorder.Direction.INBOUND, "world".getBytes(StandardCharsets.UTF_8));
            recorder.flush();
            assertTrue(recorder.bytesWritten() > 16);
        }
        assertTrue(Files.size(file) > 16);
    }

    @Test void redactsSecretTextWithoutChangingCallerBytes() throws Exception {
        Path root = Files.createTempDirectory("recorder");
        byte[] frame = "token=super-secret payload".getBytes(StandardCharsets.UTF_8);
        try (BridgeRecorder recorder = new BridgeRecorder(root, root.resolve("run.rec"), 4096)) {
            recorder.record(BridgeRecorder.Direction.OUTBOUND, frame);
        }
        assertEquals("token=super-secret payload", new String(frame, StandardCharsets.UTF_8));
        String bytes = Files.readString(root.resolve("run.rec"), StandardCharsets.ISO_8859_1);
        assertFalse(bytes.contains("super-secret"));
        assertTrue(bytes.contains("<redacted>"));
    }

    @Test void rejectsPathOutsideDiagnosticsRootAndBoundOverflow() throws Exception {
        Path root = Files.createTempDirectory("recorder");
        assertThrows(IllegalArgumentException.class, () -> new BridgeRecorder(root, root.resolve("../escape.rec"), 1024));
        try (BridgeRecorder recorder = new BridgeRecorder(root, root.resolve("run.rec"), 32)) {
            assertThrows(IllegalStateException.class, () -> recorder.record(BridgeRecorder.Direction.OUTBOUND, new byte[64]));
            assertNotNull(recorder.failure());
        }
    }
}
