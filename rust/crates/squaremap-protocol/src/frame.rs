use crate::limits::FrameLimits;
use bytes::BytesMut;
use prost::Message;
use std::fmt;
use std::io::{self, Read};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

use crate::wire::{envelope, ChunkSnapshot, ChunkSnapshotBody, Envelope};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FrameClass {
    Control = 0,
    ChunkSnapshot = 1,
}

impl TryFrom<u8> for FrameClass {
    type Error = FrameError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Control),
            1 => Ok(Self::ChunkSnapshot),
            class => Err(FrameError::MalformedClass { class }),
        }
    }
}

#[derive(Debug)]
pub enum SnapshotValidationReason {
    CompressedBodyEmpty,
    CompressedBodyOversize { length: usize, max: u32 },
    DeclaredUncompressedLength { length: u32, max: u32 },
    DecompressionRatio { compressed: usize, uncompressed: u32 },
    InvalidZstd(String),
    DecompressionIo(String),
    DecompressedLength { expected: u32, actual: usize },
    Crc32c { expected: u32, actual: u32 },
    BodyProtobuf(String),
}

#[derive(Debug)]
pub enum FrameError {
    Io(io::Error),
    MalformedClass { class: u8 },
    ZeroLength,
    DeclaredLength { class: FrameClass, length: u32, max: u32 },
    EarlyEof { expected: usize, actual: usize },
    Protobuf(prost::DecodeError),
    ProtobufEncode(prost::EncodeError),
    ClassMismatch { frame_class: FrameClass, payload_class: FrameClass },
    SnapshotValidation { reason: SnapshotValidationReason },
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::MalformedClass { class } => write!(formatter, "unknown frame class {class}"),
            Self::ZeroLength => formatter.write_str("frame length is zero"),
            Self::DeclaredLength { class, length, max } => {
                write!(formatter, "{class:?} frame length {length} exceeds {max}")
            }
            Self::EarlyEof { expected, actual } => {
                write!(formatter, "early EOF: expected {expected} bytes, got {actual}")
            }
            Self::Protobuf(error) => write!(formatter, "invalid envelope protobuf: {error}"),
            Self::ProtobufEncode(error) => write!(formatter, "failed to encode envelope: {error}"),
            Self::ClassMismatch {
                frame_class,
                payload_class,
            } => write!(formatter, "frame class {frame_class:?} mismatches payload {payload_class:?}"),
            Self::SnapshotValidation { reason } => write!(formatter, "invalid chunk snapshot: {reason}"),
        }
    }
}

impl fmt::Display for SnapshotValidationReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CompressedBodyEmpty => formatter.write_str("compressed body is empty"),
            Self::CompressedBodyOversize { length, max } => {
                write!(formatter, "compressed body length {length} exceeds {max}")
            }
            Self::DeclaredUncompressedLength { length, max } => {
                write!(formatter, "declared uncompressed length {length} exceeds {max}")
            }
            Self::DecompressionRatio {
                compressed,
                uncompressed,
            } => write!(
                formatter,
                "declared uncompressed length {uncompressed} exceeds {compressed} * 4096"
            ),
            Self::InvalidZstd(error) => write!(formatter, "invalid zstd body: {error}"),
            Self::DecompressionIo(error) => write!(formatter, "zstd decompression failed: {error}"),
            Self::DecompressedLength { expected, actual } => {
                write!(formatter, "decompressed length {actual}, expected {expected}")
            }
            Self::Crc32c { expected, actual } => {
                write!(formatter, "CRC32C {actual:#x}, expected {expected:#x}")
            }
            Self::BodyProtobuf(error) => write!(formatter, "invalid snapshot body protobuf: {error}"),
        }
    }
}

impl std::error::Error for FrameError {}
impl std::error::Error for SnapshotValidationReason {}

struct CountingReader<'a, R> {
    inner: &'a mut R,
    count: usize,
}

impl<R> Unpin for CountingReader<'_, R> {}

impl<R: AsyncRead + Unpin> AsyncRead for CountingReader<'_, R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.as_mut().get_mut();
        let before = buffer.filled().len();
        let result = Pin::new(&mut *this.inner).poll_read(cx, buffer);
        if let Poll::Ready(Ok(())) = &result {
            this.count += buffer.filled().len() - before;
        }
        result
    }
}

async fn read_exact_counted<R: AsyncRead + Unpin>(
    reader: &mut R,
    buffer: &mut [u8],
) -> Result<(), FrameError> {
    let mut counted = CountingReader {
        inner: reader,
        count: 0,
    };
    match counted.read_exact(buffer).await {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Err(FrameError::EarlyEof {
            expected: buffer.len(),
            actual: counted.count,
        }),
        Err(error) => Err(FrameError::Io(error)),
    }
}

fn class_limit(class: FrameClass, limits: FrameLimits) -> u32 {
    match class {
        FrameClass::Control => limits.max_control_bytes,
        FrameClass::ChunkSnapshot => limits.max_snapshot_bytes,
    }
}

fn payload_class(envelope: &Envelope) -> FrameClass {
    match envelope.payload {
        Some(envelope::Payload::ChunkSnapshot(_)) => FrameClass::ChunkSnapshot,
        _ => FrameClass::Control,
    }
}

fn snapshot_error(reason: SnapshotValidationReason) -> FrameError {
    FrameError::SnapshotValidation { reason }
}

fn validate_snapshot(
    snapshot: &ChunkSnapshot,
    limits: FrameLimits,
) -> Result<(), FrameError> {
    let compressed_length = snapshot.compressed_body.len();
    if compressed_length == 0 {
        return Err(snapshot_error(SnapshotValidationReason::CompressedBodyEmpty));
    }
    if compressed_length > limits.max_snapshot_bytes as usize {
        return Err(snapshot_error(SnapshotValidationReason::CompressedBodyOversize {
            length: compressed_length,
            max: limits.max_snapshot_bytes,
        }));
    }

    let declared = snapshot.uncompressed_length;
    if declared > limits.max_uncompressed_snapshot_bytes {
        return Err(snapshot_error(
            SnapshotValidationReason::DeclaredUncompressedLength {
                length: declared,
                max: limits.max_uncompressed_snapshot_bytes,
            },
        ));
    }
    let ratio_limit = (compressed_length as u64)
        .checked_mul(FrameLimits::MAX_SNAPSHOT_DECOMPRESSION_RATIO)
        .unwrap_or(u64::MAX);
    if declared as u64 > ratio_limit {
        return Err(snapshot_error(SnapshotValidationReason::DecompressionRatio {
            compressed: compressed_length,
            uncompressed: declared,
        }));
    }

    let mut decoder = zstd::stream::read::Decoder::new(snapshot.compressed_body.as_slice())
        .map_err(|error| snapshot_error(SnapshotValidationReason::InvalidZstd(error.to_string())))?;
    let mut uncompressed = Vec::with_capacity(declared as usize);
    let mut chunk = [0_u8; 8192];
    loop {
        let count = decoder.read(&mut chunk).map_err(|error| {
            snapshot_error(SnapshotValidationReason::DecompressionIo(error.to_string()))
        })?;
        if count == 0 {
            break;
        }
        if count > declared as usize - uncompressed.len() {
            return Err(snapshot_error(SnapshotValidationReason::DecompressedLength {
                expected: declared,
                actual: uncompressed.len() + count,
            }));
        }
        uncompressed.extend_from_slice(&chunk[..count]);
    }
    if uncompressed.len() != declared as usize {
        return Err(snapshot_error(SnapshotValidationReason::DecompressedLength {
            expected: declared,
            actual: uncompressed.len(),
        }));
    }

    let actual_crc = crc32c::crc32c(&uncompressed);
    if actual_crc != snapshot.crc32c {
        return Err(snapshot_error(SnapshotValidationReason::Crc32c {
            expected: snapshot.crc32c,
            actual: actual_crc,
        }));
    }
    ChunkSnapshotBody::decode(uncompressed.as_slice()).map_err(|error| {
        snapshot_error(SnapshotValidationReason::BodyProtobuf(error.to_string()))
    })?;
    Ok(())
}

/// Reads one class/length-prefixed envelope from an asynchronous stream.
pub async fn read_envelope<R: AsyncRead + Unpin>(
    reader: &mut R,
    limits: FrameLimits,
) -> Result<Envelope, FrameError> {
    let mut prefix = [0_u8; 5];
    read_exact_counted(reader, &mut prefix).await?;
    let class = FrameClass::try_from(prefix[0])?;
    let length = u32::from_be_bytes([prefix[1], prefix[2], prefix[3], prefix[4]]);
    if length == 0 {
        return Err(FrameError::ZeroLength);
    }
    let max = class_limit(class, limits);
    if length > max {
        return Err(FrameError::DeclaredLength { class, length, max });
    }

    let mut payload = BytesMut::zeroed(length as usize);
    read_exact_counted(reader, &mut payload).await?;
    let decoded = Envelope::decode(payload.as_ref()).map_err(FrameError::Protobuf)?;
    let decoded_class = payload_class(&decoded);
    if decoded_class != class {
        return Err(FrameError::ClassMismatch {
            frame_class: class,
            payload_class: decoded_class,
        });
    }
    if let Some(envelope::Payload::ChunkSnapshot(snapshot)) = decoded.payload.as_ref() {
        validate_snapshot(snapshot, limits)?;
    }
    Ok(decoded)
}

/// Writes one class/length-prefixed envelope to an asynchronous stream.
pub async fn write_envelope<W: AsyncWrite + Unpin>(
    writer: &mut W,
    envelope: &Envelope,
    limits: FrameLimits,
) -> Result<(), FrameError> {
    let class = payload_class(envelope);
    if let Some(envelope::Payload::ChunkSnapshot(snapshot)) = envelope.payload.as_ref() {
        validate_snapshot(snapshot, limits)?;
    }
    let payload_length = envelope.encoded_len();
    if payload_length == 0 {
        return Err(FrameError::ZeroLength);
    }
    let max = class_limit(class, limits);
    if payload_length > max as usize {
        return Err(FrameError::DeclaredLength {
            class,
            length: payload_length as u32,
            max,
        });
    }
    let mut payload = BytesMut::with_capacity(payload_length);
    envelope
        .encode(&mut payload)
        .map_err(FrameError::ProtobufEncode)?;
    let mut prefix = [0_u8; 5];
    prefix[0] = class as u8;
    prefix[1..].copy_from_slice(&(payload_length as u32).to_be_bytes());
    writer.write_all(&prefix).await.map_err(FrameError::Io)?;
    writer.write_all(&payload).await.map_err(FrameError::Io)?;
    Ok(())
}
