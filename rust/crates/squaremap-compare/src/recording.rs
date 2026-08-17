use std::io::{self, Cursor, Read};

pub const FORMAT_VERSION: u8 = 1;
const MAGIC: &[u8; 4] = b"SMRC";
const MAX_FRAMES: usize = 1_000_000;
const MAX_FRAME_BYTES: usize = 67_108_864;
const MAX_RECORDING_BYTES: usize = 134_217_728;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame { pub monotonic_nanos: u64, pub direction: u8, pub bytes: Vec<u8> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording { pub protocol_version: u8, pub frames: Vec<Frame> }

#[derive(Debug)]
pub enum RecordingError { Io(io::Error), Invalid(&'static str), Truncated }
impl std::fmt::Display for RecordingError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { match self { Self::Io(e)=>write!(f,"I/O error: {e}"), Self::Invalid(s)=>f.write_str(s), Self::Truncated=>f.write_str("truncated recording") } } }
impl std::error::Error for RecordingError {}
impl From<io::Error> for RecordingError { fn from(e: io::Error)->Self { Self::Io(e) } }


pub fn validate(recording: &Recording) -> Result<(), RecordingError> {
    let mut previous = 0u64;
    for (index, frame) in recording.frames.iter().enumerate() {
        if frame.direction > 1 {
            return Err(RecordingError::Invalid("invalid frame direction"));
        }
        if index > 0 && frame.monotonic_nanos < previous {
            return Err(RecordingError::Invalid("non-monotonic frame timestamp"));
        }
        previous = frame.monotonic_nanos;
    }
    Ok(())
}
pub fn encode(recording: &Recording) -> Vec<u8> {
    encode_checked(recording).expect("recording passed to encode must satisfy format bounds")
}
pub fn encode_checked(recording: &Recording) -> Result<Vec<u8>, RecordingError> {
    validate(recording)?;
    if recording.frames.len() > MAX_FRAMES {
        return Err(RecordingError::Invalid("too many frames"));
    }
    let frame_count = u32::try_from(recording.frames.len()).map_err(|_| RecordingError::Invalid("too many frames"))?;
    let mut total = 0usize;
    for frame in &recording.frames {
        if frame.bytes.len() > MAX_FRAME_BYTES {
            return Err(RecordingError::Invalid("frame payload exceeds limit"));
        }
        total = total
            .checked_add(frame.bytes.len())
            .filter(|size| *size <= MAX_RECORDING_BYTES)
            .ok_or(RecordingError::Invalid("recording payload exceeds limit"))?;
    }
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(FORMAT_VERSION);
    out.push(recording.protocol_version);
    out.extend_from_slice(&frame_count.to_le_bytes());
    for frame in &recording.frames {
        let length = u32::try_from(frame.bytes.len()).map_err(|_| RecordingError::Invalid("frame payload exceeds limit"))?;
        out.extend_from_slice(&frame.monotonic_nanos.to_le_bytes());
        out.push(frame.direction);
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&frame.bytes);
    }
    Ok(out)
}

pub fn decode(bytes: &[u8]) -> Result<Recording, RecordingError> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0; 4];
    cursor.read_exact(&mut magic).map_err(|_| RecordingError::Truncated)?;
    if &magic != MAGIC {
        return Err(RecordingError::Invalid("invalid recording magic"));
    }
    let format = read_u8(&mut cursor)?;
    if format != FORMAT_VERSION {
        return Err(RecordingError::Invalid("unsupported recording version"));
    }
    let protocol = read_u8(&mut cursor)?;
    if protocol != 1 {
        return Err(RecordingError::Invalid("unsupported protocol version"));
    }
    let count = read_u32(&mut cursor)?;
    if count as usize > MAX_FRAMES {
        return Err(RecordingError::Invalid("too many frames"));
    }
    let mut frames = Vec::with_capacity(count as usize);
    let mut total = 0usize;
    for _ in 0..count {
        let nanos = read_u64(&mut cursor)?;
        let direction = read_u8(&mut cursor)?;
        if direction > 1 {
            return Err(RecordingError::Invalid("invalid frame direction"));
        }
        let len = read_u32(&mut cursor)? as usize;
        if len > MAX_FRAME_BYTES || total.checked_add(len).filter(|size| *size <= MAX_RECORDING_BYTES).is_none() {
            return Err(RecordingError::Invalid("recording payload exceeds limit"));
        }
        total += len;
        let mut frame_bytes = vec![0; len];
        cursor.read_exact(&mut frame_bytes).map_err(|_| RecordingError::Truncated)?;
        frames.push(Frame { monotonic_nanos: nanos, direction, bytes: frame_bytes });
    }
    if cursor.position() as usize != bytes.len() {
        return Err(RecordingError::Invalid("trailing recording data"));
    }
    let recording = Recording { protocol_version: protocol, frames };
    validate(&recording)?;
    Ok(recording)
}
fn read_u8(c:&mut Cursor<&[u8]>)->Result<u8,RecordingError>{let mut b=[0];c.read_exact(&mut b).map_err(|_|RecordingError::Truncated)?;Ok(b[0])}
fn read_u32(c:&mut Cursor<&[u8]>)->Result<u32,RecordingError>{let mut b=[0;4];c.read_exact(&mut b).map_err(|_|RecordingError::Truncated)?;Ok(u32::from_le_bytes(b))}
fn read_u64(c:&mut Cursor<&[u8]>)->Result<u64,RecordingError>{let mut b=[0;8];c.read_exact(&mut b).map_err(|_|RecordingError::Truncated)?;Ok(u64::from_le_bytes(b))}
