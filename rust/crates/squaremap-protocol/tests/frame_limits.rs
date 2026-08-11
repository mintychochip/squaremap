use prost::Message;
use squaremap_protocol::{
    envelope, read_envelope, write_envelope, ChunkSnapshot, ChunkSnapshotBody, Envelope,
    FrameClass, FrameError, FrameLimits,
};
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};

struct OneByteReader {
    bytes: Vec<u8>,
    position: usize,
}

impl AsyncRead for OneByteReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.position == self.bytes.len() {
            return Poll::Ready(Ok(()));
        }
        buf.put_slice(&self.bytes[self.position..self.position + 1]);
        self.position += 1;
        Poll::Ready(Ok(()))
    }
}

fn control_envelope() -> Envelope {
    Envelope {
        payload: Some(envelope::Payload::Hello(Default::default())),
        ..Default::default()
    }
}

fn snapshot_envelope(uncompressed_length: u32, crc32c: u32) -> Envelope {
    let body = ChunkSnapshotBody::default().encode_to_vec();
    let compressed_body = zstd::stream::encode_all(&body[..], 0).expect("compress empty body");
    Envelope {
        payload: Some(envelope::Payload::ChunkSnapshot(ChunkSnapshot {
            uncompressed_length,
            crc32c,
            compressed_body,
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn frame(class: FrameClass, envelope: &Envelope) -> Vec<u8> {
    let payload = envelope.encode_to_vec();
    let mut bytes = Vec::with_capacity(5 + payload.len());
    bytes.push(class as u8);
    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&payload);
    bytes
}

#[tokio::test]
async fn rejects_unknown_frame_class() {
    let bytes = [2, 0, 0, 0, 1, 0];
    let err = read_envelope(&mut &bytes[..], FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(err, FrameError::MalformedClass { class: 2 }));
}

#[tokio::test]
async fn rejects_zero_length_before_allocating() {
    let bytes = [FrameClass::Control as u8, 0, 0, 0, 0];
    let err = read_envelope(&mut &bytes[..], FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(err, FrameError::ZeroLength));
}

#[tokio::test]
async fn rejects_declared_control_frame_over_limit_before_allocating() {
    let mut bytes = Vec::from([FrameClass::Control as u8]);
    bytes.extend_from_slice(&1_048_577_u32.to_be_bytes());
    let err = read_envelope(&mut bytes.as_slice(), FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FrameError::DeclaredLength {
            length: 1_048_577,
            ..
        }
    ));
}

#[tokio::test]
async fn rejects_declared_snapshot_frame_over_limit_before_allocating() {
    let mut bytes = Vec::from([FrameClass::ChunkSnapshot as u8]);
    bytes.extend_from_slice(&67_108_865_u32.to_be_bytes());
    let err = read_envelope(&mut bytes.as_slice(), FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FrameError::DeclaredLength {
            length: 67_108_865,
            ..
        }
    ));
}

#[tokio::test]
async fn rejects_class_payload_mismatch_in_both_directions() {
    let control = frame(FrameClass::ChunkSnapshot, &control_envelope());
    let err = read_envelope(&mut &control[..], FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(err, FrameError::ClassMismatch { .. }));

    let snapshot = frame(
        FrameClass::Control,
        &snapshot_envelope(1, crc32c::crc32c(&[0])),
    );
    let err = read_envelope(&mut &snapshot[..], FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(err, FrameError::ClassMismatch { .. }));
}

#[tokio::test]
async fn rejects_early_eof_with_expected_and_actual_counts() {
    let prefix = [FrameClass::Control as u8, 0, 0, 0, 1];
    let err = read_envelope(&mut &prefix[..2], FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FrameError::EarlyEof {
            expected: 5,
            actual: 2
        }
    ));

    let bytes = frame(FrameClass::Control, &control_envelope());
    let err = read_envelope(&mut &bytes[..bytes.len() - 1], FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(err, FrameError::EarlyEof { .. }));
}

#[tokio::test]
async fn rejects_trailing_bytes_and_invalid_protobuf() {
    let mut trailing = frame(FrameClass::Control, &control_envelope());
    trailing.push(0);
    let length = (trailing.len() - 5) as u32;
    trailing[1..5].copy_from_slice(&length.to_be_bytes());
    let err = read_envelope(&mut &trailing[..], FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(err, FrameError::Protobuf(_)));

    let invalid = [FrameClass::Control as u8, 0, 0, 0, 1, 0xff];
    let err = read_envelope(&mut &invalid[..], FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(err, FrameError::Protobuf(_)));
}

#[tokio::test]
async fn rejects_snapshot_decompression_above_absolute_limit() {
    let envelope = snapshot_envelope(134_217_729, 0);
    let bytes = frame(FrameClass::ChunkSnapshot, &envelope);
    let err = read_envelope(&mut &bytes[..], FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(err, FrameError::SnapshotValidation { .. }));
}

#[tokio::test]
async fn rejects_snapshot_decompression_ratio_above_4096() {
    let mut envelope = snapshot_envelope(0, 0);
    if let Some(envelope::Payload::ChunkSnapshot(snapshot)) = envelope.payload.as_mut() {
        snapshot.uncompressed_length =
            (snapshot.compressed_body.len() as u32) * 4096 + 1;
    }
    let bytes = frame(FrameClass::ChunkSnapshot, &envelope);
    let err = read_envelope(&mut &bytes[..], FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(err, FrameError::SnapshotValidation { .. }));
}

#[tokio::test]
async fn rejects_empty_or_invalid_zstd_body() {
    for compressed_body in [Vec::new(), vec![0xff]] {
        let envelope = Envelope {
            payload: Some(envelope::Payload::ChunkSnapshot(ChunkSnapshot {
                compressed_body,
                ..Default::default()
            })),
            ..Default::default()
        };
        let bytes = frame(FrameClass::ChunkSnapshot, &envelope);
        let err = read_envelope(&mut &bytes[..], FrameLimits::default())
            .await
            .unwrap_err();
        assert!(matches!(err, FrameError::SnapshotValidation { .. }));
    }
}


#[tokio::test]
async fn rejects_declared_snapshot_length_above_absolute_limit() {
    let envelope = snapshot_envelope(u32::MAX, 0);
    let bytes = frame(FrameClass::ChunkSnapshot, &envelope);
    let err = read_envelope(&mut &bytes[..], FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(err, FrameError::SnapshotValidation { .. }));
}
#[tokio::test]
async fn rejects_snapshot_crc_mismatch() {
    let envelope = snapshot_envelope(0, 1);
    let bytes = frame(FrameClass::ChunkSnapshot, &envelope);
    let err = read_envelope(&mut &bytes[..], FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(err, FrameError::SnapshotValidation { .. }));
}

#[tokio::test]
async fn accepts_valid_frame_split_across_one_byte_reads() {
    let body = ChunkSnapshotBody::default().encode_to_vec();
    let envelope = snapshot_envelope(body.len() as u32, crc32c::crc32c(&body));
    let bytes = frame(FrameClass::ChunkSnapshot, &envelope);
    let mut reader = OneByteReader { bytes, position: 0 };
    let decoded = read_envelope(&mut reader, FrameLimits::default())
        .await
        .expect("split frame should decode");
    assert!(matches!(
        decoded.payload,
        Some(envelope::Payload::ChunkSnapshot(_))
    ));
}

#[tokio::test]
async fn write_rejects_invalid_snapshot_before_writing() {
    let envelope = snapshot_envelope(0, 1);
    let mut sink = tokio::io::sink();
    let err = write_envelope(&mut sink, &envelope, FrameLimits::default())
        .await
        .unwrap_err();
    assert!(matches!(err, FrameError::SnapshotValidation { .. }));
}

#[tokio::test]
async fn write_loops_until_a_split_sink_is_complete() {
    let envelope = control_envelope();
    let mut sink = OneByteWriter::default();
    write_envelope(&mut sink, &envelope, FrameLimits::default())
        .await
        .expect("write should complete");
    assert_eq!(sink.bytes[0], FrameClass::Control as u8);
    assert_eq!(sink.bytes.len(), 5 + envelope.encoded_len());
}

#[derive(Default)]
struct OneByteWriter {
    bytes: Vec<u8>,
}

impl AsyncWrite for OneByteWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.bytes.push(bytes[0]);
        Poll::Ready(Ok(1))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
