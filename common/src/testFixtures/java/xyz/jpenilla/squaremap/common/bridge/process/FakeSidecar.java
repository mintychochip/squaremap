package xyz.jpenilla.squaremap.common.bridge.process;

import com.google.protobuf.ByteString;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.net.InetSocketAddress;
import java.nio.channels.SocketChannel;
import java.nio.charset.StandardCharsets;
import java.util.Base64;
import xyz.jpenilla.squaremap.bridge.v1.Ack;
import xyz.jpenilla.squaremap.bridge.v1.AckStatus;
import xyz.jpenilla.squaremap.bridge.v1.Envelope;
import xyz.jpenilla.squaremap.bridge.v1.Hello;
import static xyz.jpenilla.squaremap.common.bridge.protocol.FrameCodec.read;
import static xyz.jpenilla.squaremap.common.bridge.protocol.FrameCodec.write;

/** Process-level fixture used by SidecarSupervisorTest. */
public final class FakeSidecar {
    private FakeSidecar() {
    }

    public static void main(final String[] args) throws Exception {
        String behavior = "valid";
        String connect = null;
        String expectedRoot = null;
        String stateFile = null;
        for (final String arg : args) {
            if (arg.startsWith("--behavior=")) {
                behavior = arg.substring("--behavior=".length());
            } else if (arg.startsWith("--expected-root=")) {
                expectedRoot = arg.substring("--expected-root=".length());
            } else if (arg.startsWith("--state-file=")) {
                stateFile = arg.substring("--state-file=".length());
            } else if (arg.equals("--connect")) {
                connect = "pending";
            } else if (connect != null && connect.equals("pending")) {
                connect = arg;
            }
        }
        if ("root-check".equals(behavior) && !java.util.Objects.equals(expectedRoot, System.getenv("SQUAREMAP_OUTPUT_ROOT"))) {
            throw new IllegalStateException("child did not observe configured Rust output root");
        }
        final int launch = stateFile == null ? 1 : nextLaunch(java.nio.file.Path.of(stateFile));
        if ("auth-fails-after-restart".equals(behavior)) {
            behavior = launch == 1 ? "disconnect-after-handshake" : "token-mismatch";
        }
        if ("timeout".equals(behavior)) {
            Thread.sleep(60_000L);
            return;
        }
        if (connect == null || connect.equals("pending")) {
            throw new IllegalArgumentException("missing connect address");
        }
        final String tokenLine;
        try (BufferedReader reader = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.US_ASCII))) {
            tokenLine = reader.readLine();
        }
        if (tokenLine == null) {
            throw new IllegalStateException("missing token");
        }
        final byte[] token = Base64.getDecoder().decode(tokenLine);
        final String[] address = connect.split(":", 2);
        try (SocketChannel socket = SocketChannel.open()) {
            socket.connect(new InetSocketAddress(address[0], Integer.parseInt(address[1])));
            final byte[] session = new byte[16];
            for (int i = 0; i < session.length; i++) {
                session[i] = (byte) (i + 1);
            }
            final byte[] helloToken = token.clone();
            if ("token-mismatch".equals(behavior)) {
                helloToken[0] ^= 0x55;
            }
            final int major = "major-mismatch".equals(behavior) ? 2 : 1;
            final int minor = "minor-mismatch".equals(behavior) ? 1 : 0;
            final Envelope hello = Envelope.newBuilder()
                .setProtocolMajor(major)
                .setProtocolMinor(minor)
                .setSessionId(ByteString.copyFrom(session))
                .setSequence(1)
                .setHello(Hello.newBuilder()
                    .setPluginVersion("fixture")
                    .setBootstrapToken(ByteString.copyFrom(helloToken)))
                .build();
            write(socket, hello);
            if ("stderr".equals(behavior)) {
                final byte[] noise = new byte[32 * 1024];
                for (int i = 0; i < noise.length; i++) {
                    noise[i] = 'x';
                }
                System.err.write(noise);
                System.err.flush();
            }
            if ("crash-after-handshake".equals(behavior)) {
                read(socket); // wait for HelloAck before simulating a crash
                return;
            }
            if ("disconnect-after-handshake".equals(behavior)) {
                final Envelope helloAck = read(socket);
                if (!helloAck.hasHelloAck() || !helloAck.getHelloAck().getAccepted()) {
                    throw new IllegalStateException("expected accepted HelloAck");
                }
                return;
            }
            if ("duplicate-inbound-sequence".equals(behavior) || "gap-inbound-sequence".equals(behavior)) {
                read(socket); // authenticated HelloAck
                write(socket, Envelope.newBuilder()
                    .setProtocolMajor(1).setProtocolMinor(minor).setSessionId(ByteString.copyFrom(session))
                    .setSequence(3)
                    .setAck(Ack.newBuilder().setAcknowledgedSequence(0)
                        .setStatus(AckStatus.ACK_STATUS_ACCEPTED))
                    .build());
                write(socket, Envelope.newBuilder()
                    .setProtocolMajor(1).setProtocolMinor(minor).setSessionId(ByteString.copyFrom(session))
                    .setSequence("duplicate-inbound-sequence".equals(behavior) ? 3 : 5)
                    .setAck(Ack.newBuilder().setAcknowledgedSequence(0)
                        .setStatus(AckStatus.ACK_STATUS_ACCEPTED))
                    .build());
                Thread.sleep(60_000L);
            }
            if ("disconnect".equals(behavior)) {
                read(socket);
                return;
            }
            if ("ack-publish".equals(behavior)) {
                read(socket); // authenticated HelloAck
                final Envelope first = read(socket);
                if (first.getPayloadCase() != Envelope.PayloadCase.BRIDGE_IDENTITY_REPLACE) {
                    throw new IllegalStateException("expected framed BridgeIdentityReplace payload");
                }
                write(socket, Envelope.newBuilder()
                    .setProtocolMajor(1).setProtocolMinor(0).setSessionId(ByteString.copyFrom(session))
                    .setSequence(3)
                    .setAck(Ack.newBuilder().setAcknowledgedSequence(first.getSequence())
                        .setStatus(AckStatus.ACK_STATUS_ACCEPTED))
                    .build());
                final Envelope second = read(socket);
                if (second.getPayloadCase() != Envelope.PayloadCase.PLAYERS_REPLACE) {
                    throw new IllegalStateException("expected framed PlayersReplace payload");
                }
                write(socket, Envelope.newBuilder()
                    .setProtocolMajor(1).setProtocolMinor(0).setSessionId(ByteString.copyFrom(session))
                    .setSequence(4)
                    .setAck(Ack.newBuilder().setAcknowledgedSequence(second.getSequence())
                        .setStatus(AckStatus.ACK_STATUS_ACCEPTED))
                    .build());
                Thread.sleep(500L);
                return;
            }
            if ("world-enumeration".equals(behavior)) {
                read(socket); // authenticated HelloAck
                write(socket, Envelope.newBuilder()
                    .setProtocolMajor(1).setProtocolMinor(0).setSessionId(ByteString.copyFrom(session))
                    .setSequence(3)
                    .setWorldEnumerationRequest(xyz.jpenilla.squaremap.bridge.v1.WorldEnumerationRequest.newBuilder()
                        .build())
                    .build());
                Thread.sleep(500L);
                return;
            }
            try {
                read(socket);
            } catch (Exception ignored) {
                // Supervisor may close immediately after rejection.
            }
        }
    }
    private static int nextLaunch(final java.nio.file.Path stateFile) throws Exception {
        final int launch;
        if (java.nio.file.Files.exists(stateFile)) {
            launch = Integer.parseInt(java.nio.file.Files.readString(stateFile).trim()) + 1;
        } else {
            launch = 1;
        }
        final java.nio.file.Path temporary = stateFile.resolveSibling(stateFile.getFileName() + ".tmp-" + ProcessHandle.current().pid());
        java.nio.file.Files.writeString(
            temporary,
            Integer.toString(launch),
            java.nio.file.StandardOpenOption.CREATE,
            java.nio.file.StandardOpenOption.TRUNCATE_EXISTING
        );
        java.nio.file.Files.move(
            temporary,
            stateFile,
            java.nio.file.StandardCopyOption.ATOMIC_MOVE,
            java.nio.file.StandardCopyOption.REPLACE_EXISTING
        );
        return launch;
    }

}
