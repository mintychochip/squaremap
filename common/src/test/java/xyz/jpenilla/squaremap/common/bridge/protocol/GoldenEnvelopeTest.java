package xyz.jpenilla.squaremap.common.bridge.protocol;

import com.google.protobuf.ByteString;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HexFormat;
import org.junit.jupiter.api.Test;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.Hello;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;

class GoldenEnvelopeTest {
    @Test
    void encodesV1HelloGoldenFrame() throws Exception {
        final Envelope envelope = Envelope.newBuilder()
            .setProtocolMajor(1)
            .setProtocolMinor(0)
            .setSessionId(ByteString.copyFrom(HexFormat.of().parseHex("00112233445566778899aabbccddeeff")))
            .setSequence(1)
            .setHello(Hello.newBuilder()
                .setPluginVersion("test")
                .setBootstrapToken(ByteString.copyFromUtf8("token")))
            .build();
        assertArrayEquals(
            Files.readAllBytes(Path.of("../testdata/bridge/v1/hello.bin")),
            envelope.toByteArray()
        );
    }
}
