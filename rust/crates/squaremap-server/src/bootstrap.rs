use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rand::RngCore;
use squaremap_protocol::wire::{envelope, Ack, AckStatus, Envelope, Hello, ProtocolError, ProtocolErrorCode, Shutdown};
use squaremap_protocol::{read_envelope, write_envelope, FrameClass, FrameError, FrameLimits};
use crate::session::{Session, SessionError};
use prost::Message;
use std::fmt;
use std::io::{self, BufRead, Read};
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use zeroize::{Zeroize, Zeroizing};

const PROTOCOL_MAJOR: u32 = 1;
const PROTOCOL_MINOR: u32 = 0;
const SESSION_ID_BYTES: usize = 16;
const BOOTSTRAP_TOKEN_BYTES: usize = 32;
const MAX_TOKEN_LINE_BYTES: usize = 88;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub enum BootstrapError {
    InvalidConnect(String),
    Token(String),
    Io(io::Error),
    Frame(FrameError),
    Rejected(String),
    ConnectTimeout,
}

impl fmt::Display for BootstrapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConnect(message) => write!(formatter, "invalid loopback connect address: {message}"),
            Self::Token(message) => write!(formatter, "invalid bootstrap token: {message}"),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Frame(error) => write!(formatter, "bridge frame error: {error}"),
            Self::Rejected(message) => write!(formatter, "bridge rejected handshake: {message}"),
            Self::ConnectTimeout => formatter.write_str("bridge connection timed out"),
        }
    }
}

impl std::error::Error for BootstrapError {}

impl From<io::Error> for BootstrapError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<FrameError> for BootstrapError {
    fn from(error: FrameError) -> Self {
        Self::Frame(error)
    }
}

pub fn parse_connect(value: &str) -> Result<SocketAddr, BootstrapError> {
    let address = SocketAddr::from_str(value)
        .map_err(|error| BootstrapError::InvalidConnect(error.to_string()))?;
    if !address.ip().is_loopback() {
        return Err(BootstrapError::InvalidConnect("address is not loopback".to_string()));
    }
    Ok(address)
}

pub fn read_token<R: BufRead>(reader: R) -> Result<Zeroizing<Vec<u8>>, BootstrapError> {
    let mut bounded = reader.take((MAX_TOKEN_LINE_BYTES + 1) as u64);
    let mut line = Zeroizing::new(Vec::with_capacity(MAX_TOKEN_LINE_BYTES + 1));
    bounded.read_to_end(&mut line)?;
    if !line.ends_with(b"\n") {
        return Err(BootstrapError::Token("expected one newline-terminated line of at most 88 characters".to_string()));
    }
    line.pop();
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    if line.len() > MAX_TOKEN_LINE_BYTES || !line.is_ascii() {
        return Err(BootstrapError::Token("token line is too long or non-ASCII".to_string()));
    }
    let decoded = Zeroizing::new(
        STANDARD
            .decode(&line)
            .map_err(|error| BootstrapError::Token(error.to_string()))?,
    );
    if decoded.len() != BOOTSTRAP_TOKEN_BYTES {
        return Err(BootstrapError::Token("token must decode to exactly 32 bytes".to_string()));
    }
    Ok(decoded)
}

async fn write_bootstrap_hello<W: AsyncWrite + Unpin>(
    writer: &mut W,
    plugin_version: &str,
    session_id: &[u8; SESSION_ID_BYTES],
    token: &mut Zeroizing<Vec<u8>>,
) -> Result<(), BootstrapError> {
    let result = write_bootstrap_hello_inner(writer, plugin_version, session_id, token.as_slice()).await;
    token.zeroize();
    result
}

async fn write_bootstrap_hello_inner<W: AsyncWrite + Unpin>(
    writer: &mut W,
    plugin_version: &str,
    session_id: &[u8; SESSION_ID_BYTES],
    token: &[u8],
) -> Result<(), BootstrapError> {
    let mut hello = Envelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        session_id: session_id.to_vec(),
        sequence: 1,
        correlation_id: 0,
        payload: Some(envelope::Payload::Hello(Hello {
            plugin_version: plugin_version.to_string(),
            bootstrap_token: token.to_vec(),
        })),
    };
    let mut encoded_payload = Vec::with_capacity(hello.encoded_len());
    let encoded = hello.encode(&mut encoded_payload);
    zeroize_hello_payload(&mut hello);
    let payload = Zeroizing::new(encoded_payload);
    encoded.map_err(FrameError::ProtobufEncode).map_err(BootstrapError::Frame)?;
    if payload.is_empty() {
        return Err(BootstrapError::Frame(FrameError::ZeroLength));
    }
    if payload.len() > FrameLimits::MAX_CONTROL_BYTES as usize {
        return Err(BootstrapError::Frame(FrameError::DeclaredLength {
            class: FrameClass::Control,
            length: payload.len() as u32,
            max: FrameLimits::MAX_CONTROL_BYTES,
        }));
    }
    let mut prefix = [0_u8; 5];
    prefix[0] = FrameClass::Control as u8;
    prefix[1..].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    writer.write_all(&prefix).await?;
    writer.write_all(payload.as_slice()).await?;
    Ok(())
}

fn zeroize_hello_payload(envelope: &mut Envelope) {
    if let Some(envelope::Payload::Hello(hello)) = envelope.payload.as_mut() {
        hello.bootstrap_token.zeroize();
        hello.bootstrap_token.clear();
    }
}

pub async fn run_bridge(
    connect: SocketAddr,
    plugin_version: &str,
    mut token: Zeroizing<Vec<u8>>,
) -> Result<(), BootstrapError> {
    let mut stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(connect))
        .await
        .map_err(|_| BootstrapError::ConnectTimeout)??;
    let mut session_id = [0_u8; SESSION_ID_BYTES];
    rand::rng().fill_bytes(&mut session_id);
    session_id[6] = (session_id[6] & 0x0f) | 0x40;
    session_id[8] = (session_id[8] & 0x3f) | 0x80;
    tracing::info!(
        session_id = %hex::encode(session_id),
        protocol_major = PROTOCOL_MAJOR,
        plugin_version,
        "bridge hello sent"
    );
    write_bootstrap_hello(&mut stream, plugin_version, &session_id, &mut token).await?;

    let ack = read_envelope(&mut stream, FrameLimits::default()).await?;
    let ack_payload = match ack.payload {
        Some(envelope::Payload::HelloAck(payload)) => payload,
        _ => return Err(BootstrapError::Rejected("expected HelloAck".to_string())),
    };
    if ack.protocol_major != PROTOCOL_MAJOR
        || ack.session_id.as_slice() != session_id
        || ack_payload.protocol_major != PROTOCOL_MAJOR
        || !ack_payload.accepted
    {
        return Err(BootstrapError::Rejected(ack_payload.rejection_reason));
    }
    let configured_root = std::env::var("SQUAREMAP_OUTPUT_ROOT")
        .map_err(|_| BootstrapError::Rejected("SQUAREMAP_OUTPUT_ROOT is required for bridge mode".to_string()))?;
    let root = squaremap_server::output::OutputRoot::new(configured_root)?;
    let mut session = Session::from_session_id(&session_id).map_err(|error| BootstrapError::Rejected(error.to_string()))?;
    loop {
        match read_envelope(&mut stream, FrameLimits::default()).await {
            Ok(envelope) => {
                if envelope.protocol_major != PROTOCOL_MAJOR || envelope.session_id.as_slice() != session.session_id() {
                    let error = Envelope {
                        protocol_major: PROTOCOL_MAJOR,
                        protocol_minor: PROTOCOL_MINOR,
                        session_id: session_id.to_vec(),
                        sequence: envelope.sequence.saturating_add(1),
                        correlation_id: envelope.correlation_id,
                        payload: Some(envelope::Payload::ProtocolError(ProtocolError {
                            code: if envelope.protocol_major != PROTOCOL_MAJOR {
                                ProtocolErrorCode::UnsupportedVersion as i32
                            } else {
                                ProtocolErrorCode::InvalidMessage as i32
                            },
                            message: "bridge envelope failed authenticated protocol/session validation".to_string(),
                            fatal: true,
                            offending_sequence: envelope.sequence,
                        })),
                    };
                    write_envelope(&mut stream, &error, FrameLimits::default()).await?;
                    return Err(BootstrapError::Rejected("bridge envelope failed protocol/session validation".to_string()));
                }
                if matches!(envelope.payload, Some(envelope::Payload::Shutdown(Shutdown { .. }))) {
                    break;
                }
                let sequence = envelope.sequence;
                let handler_root = root.clone();
                let handler_envelope = envelope.clone();
                let outcome = match session.process(envelope.clone(), move |_| {
                    let handler_root = handler_root.clone();
                    let handler_envelope = handler_envelope.clone();
                    async move {
                        squaremap_server::views::apply_replacement(&handler_root, &handler_envelope)
                            .map(|_| ())
                            .map_err(|error| SessionError::HandlerFailed(error.to_string()))
                    }
                }).await {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        let protocol_error = ProtocolError {
                            code: ProtocolErrorCode::Internal as i32,
                            message: error.to_string(),
                            fatal: true,
                            offending_sequence: sequence,
                        };
                        let response = Envelope {
                            protocol_major: PROTOCOL_MAJOR,
                            protocol_minor: PROTOCOL_MINOR,
                            session_id: session_id.to_vec(),
                            sequence: sequence.saturating_add(1),
                            correlation_id: envelope.correlation_id,
                            payload: Some(envelope::Payload::ProtocolError(protocol_error)),
                        };
                        write_envelope(&mut stream, &response, FrameLimits::default()).await?;
                        return Err(BootstrapError::Rejected(error.to_string()));
                    }
                };
                if let Some(protocol_error) = outcome.protocol_error() {
                    let error = Envelope {
                        protocol_major: PROTOCOL_MAJOR,
                        protocol_minor: PROTOCOL_MINOR,
                        session_id: session_id.to_vec(),
                        sequence: sequence.saturating_add(1),
                        correlation_id: envelope.correlation_id,
                        payload: Some(envelope::Payload::ProtocolError(protocol_error.clone())),
                    };
                    write_envelope(&mut stream, &error, FrameLimits::default()).await?;
                    return Err(BootstrapError::Rejected(protocol_error.message.clone()));
                }
                if let Some(ack_payload) = outcome.ack() {
                    let ack = Envelope {
                        protocol_major: PROTOCOL_MAJOR,
                        protocol_minor: PROTOCOL_MINOR,
                        session_id: session_id.to_vec(),
                        sequence: sequence.saturating_add(1),
                        correlation_id: envelope.correlation_id,
                        payload: Some(envelope::Payload::Ack(ack_payload.clone())),
                    };
                    write_envelope(&mut stream, &ack, FrameLimits::default()).await?;
                }
            }
            Err(FrameError::EarlyEof { actual: 0, .. }) => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_connect, read_token};
    use std::io::Cursor;
    use std::net::SocketAddr;

    #[test]
    fn accepts_ipv4_loopback_only() {
        assert_eq!(parse_connect("127.0.0.1:1234").unwrap(), "127.0.0.1:1234".parse::<SocketAddr>().unwrap());
        assert!(parse_connect("192.0.2.1:1234").is_err());
        assert!(parse_connect("0.0.0.0:1234").is_err());
    }

    #[test]
    fn accepts_ipv6_loopback() {
        assert_eq!(parse_connect("[::1]:1234").unwrap(), "[::1]:1234".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn requires_one_base64_line_of_exactly_32_bytes() {
        let encoded = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let token = read_token(Cursor::new(format!("{encoded}\n"))).unwrap();
        assert_eq!(token.len(), 32);
        assert!(read_token(Cursor::new("AAAA\n".to_string())).is_err());
    }

    #[test]
    fn rejects_overlong_token_line() {
        let line = format!("{}\n", "A".repeat(89));
        assert!(read_token(Cursor::new(line)).is_err());
    }
}
#[cfg(test)]
mod protocol_tests {
    use super::{run_bridge, write_bootstrap_hello, BootstrapError};
    use squaremap_protocol::wire::{envelope, Envelope, HelloAck, Shutdown, ShutdownReason};
    use squaremap_protocol::{read_envelope, write_envelope, FrameLimits};
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::AsyncWrite;
    use tokio::net::TcpListener;
    use zeroize::Zeroizing;

    #[tokio::test]
    async fn accepted_handshake_stays_alive_until_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let hello = read_envelope(&mut socket, FrameLimits::default()).await.unwrap();
            let ack = Envelope {
                protocol_major: 1,
                protocol_minor: 0,
                session_id: hello.session_id.clone(),
                sequence: 2,
                correlation_id: 0,
                payload: Some(envelope::Payload::HelloAck(HelloAck {
                    protocol_major: 1,
                    protocol_minor: 0,
                    backend_version: "fixture".to_string(),
                    accepted: true,
                    rejection_reason: String::new(),
                })),
            };
            write_envelope(&mut socket, &ack, FrameLimits::default()).await.unwrap();
            let shutdown = Envelope {
                protocol_major: 1,
                protocol_minor: 0,
                session_id: hello.session_id,
                sequence: 3,
                correlation_id: 0,
                payload: Some(envelope::Payload::Shutdown(Shutdown {
                    reason: ShutdownReason::Requested as i32,
                    message: String::new(),
                })),
            };
            write_envelope(&mut socket, &shutdown, FrameLimits::default()).await.unwrap();
        });
        let result = run_bridge(address, "fixture", Zeroizing::new(vec![0; 32])).await;
        server.await.unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn rejected_ack_fails_bootstrap() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let hello = read_envelope(&mut socket, FrameLimits::default()).await.unwrap();
            let ack = Envelope {
                protocol_major: 1,
                protocol_minor: 0,
                session_id: hello.session_id,
                sequence: 2,
                correlation_id: 0,
                payload: Some(envelope::Payload::HelloAck(HelloAck {
                    protocol_major: 1,
                    protocol_minor: 0,
                    backend_version: String::new(),
                    accepted: false,
                    rejection_reason: "rejected by fixture".to_string(),
                })),
            };
            write_envelope(&mut socket, &ack, FrameLimits::default()).await.unwrap();
        });
        let result = run_bridge(address, "fixture", Zeroizing::new(vec![0; 32])).await;
        server.await.unwrap();
        assert!(matches!(result, Err(BootstrapError::Rejected(_))));
    }
    struct FailingWriter;

    impl AsyncWrite for FailingWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, "fixture failure")))
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn secret_hello_writer_zeroizes_token_on_write_error() {
        let mut token = Zeroizing::new(vec![0xa5; 32]);
        let mut writer = FailingWriter;
        let session_id = [0_u8; 16];
        let result = write_bootstrap_hello(&mut writer, "fixture", &session_id, &mut token).await;
        assert!(result.is_err());
        assert!(token.iter().all(|byte| *byte == 0));
    }
}
