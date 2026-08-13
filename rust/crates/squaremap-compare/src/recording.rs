use std::io::{self, Cursor, Read};

pub const FORMAT_VERSION: u8 = 1;
const MAGIC: &[u8; 4] = b"SMRC";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame { pub monotonic_nanos: u64, pub direction: u8, pub bytes: Vec<u8> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording { pub protocol_version: u8, pub frames: Vec<Frame> }

#[derive(Debug)]
pub enum RecordingError { Io(io::Error), Invalid(&'static str), Truncated }
impl std::fmt::Display for RecordingError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { match self { Self::Io(e)=>write!(f,"I/O error: {e}"), Self::Invalid(s)=>f.write_str(s), Self::Truncated=>f.write_str("truncated recording") } } }
impl std::error::Error for RecordingError {}
impl From<io::Error> for RecordingError { fn from(e: io::Error)->Self { Self::Io(e) } }

pub fn encode(recording: &Recording) -> Vec<u8> {
 let mut out=Vec::new(); out.extend_from_slice(MAGIC); out.push(FORMAT_VERSION); out.push(recording.protocol_version); out.extend_from_slice(&(recording.frames.len() as u32).to_le_bytes());
 for f in &recording.frames { out.extend_from_slice(&f.monotonic_nanos.to_le_bytes()); out.push(f.direction); out.extend_from_slice(&(f.bytes.len() as u32).to_le_bytes()); out.extend_from_slice(&f.bytes); } out
}
pub fn decode(bytes: &[u8]) -> Result<Recording, RecordingError> {
 let mut c=Cursor::new(bytes); let mut magic=[0;4]; c.read_exact(&mut magic).map_err(|_|RecordingError::Truncated)?; if &magic!=MAGIC{return Err(RecordingError::Invalid("invalid recording magic"))}; let format=read_u8(&mut c)?; if format!=FORMAT_VERSION{return Err(RecordingError::Invalid("unsupported recording version"))}; let protocol=read_u8(&mut c)?; if protocol != 1 { return Err(RecordingError::Invalid("unsupported protocol version")) }; let count=read_u32(&mut c)?; if count > 1_000_000 { return Err(RecordingError::Invalid("too many frames")) }; let mut frames=Vec::with_capacity(count as usize); let mut total=0usize;
 for _ in 0..count { let nanos=read_u64(&mut c)?; let direction=read_u8(&mut c)?; if direction>1{return Err(RecordingError::Invalid("invalid frame direction"))}; let len=read_u32(&mut c)? as usize; if len > 67_108_864 || total.checked_add(len).filter(|n| *n <= 134_217_728).is_none() { return Err(RecordingError::Invalid("recording payload exceeds limit")) }; total += len; let mut b=vec![0;len]; c.read_exact(&mut b).map_err(|_|RecordingError::Truncated)?; frames.push(Frame{monotonic_nanos:nanos,direction,bytes:b}); } if c.position() as usize != bytes.len(){return Err(RecordingError::Invalid("trailing recording data"))}; Ok(Recording{protocol_version:protocol,frames})
}
fn read_u8(c:&mut Cursor<&[u8]>)->Result<u8,RecordingError>{let mut b=[0];c.read_exact(&mut b).map_err(|_|RecordingError::Truncated)?;Ok(b[0])}
fn read_u32(c:&mut Cursor<&[u8]>)->Result<u32,RecordingError>{let mut b=[0;4];c.read_exact(&mut b).map_err(|_|RecordingError::Truncated)?;Ok(u32::from_le_bytes(b))}
fn read_u64(c:&mut Cursor<&[u8]>)->Result<u64,RecordingError>{let mut b=[0;8];c.read_exact(&mut b).map_err(|_|RecordingError::Truncated)?;Ok(u64::from_le_bytes(b))}
