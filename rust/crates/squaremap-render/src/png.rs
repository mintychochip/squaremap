use crate::tile::{TILE_RGBA_BYTES, TILE_SIZE};
use std::fmt;
use std::io::Cursor;

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct PngOptions {
    pub compression: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PngError {
    InvalidRgbaLength { actual: usize },
    Decode(String),
    InvalidDimensions { width: u32, height: u32 },
    InvalidColorType { actual: String },
    InvalidBitDepth { actual: String },
    Encode(String),
}

impl fmt::Display for PngError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRgbaLength { actual } => {
                write!(
                    formatter,
                    "RGBA tile has {actual} bytes; expected {TILE_RGBA_BYTES}"
                )
            }
            Self::Decode(message) => write!(formatter, "PNG decode failed: {message}"),
            Self::InvalidDimensions { width, height } => {
                write!(formatter, "PNG is {width}x{height}; expected 512x512")
            }
            Self::InvalidColorType { actual } => {
                write!(formatter, "PNG color type is {actual}; expected Rgba")
            }
            Self::InvalidBitDepth { actual } => {
                write!(formatter, "PNG bit depth is {actual}; expected Eight")
            }
            Self::Encode(message) => write!(formatter, "PNG encode failed: {message}"),
        }
    }
}

impl std::error::Error for PngError {}

pub fn encode_rgba_png(rgba: &[u8], options: PngOptions) -> Result<Vec<u8>, PngError> {
    if rgba.len() != TILE_RGBA_BYTES {
        return Err(PngError::InvalidRgbaLength { actual: rgba.len() });
    }

    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, TILE_SIZE as u32, TILE_SIZE as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(if options.compression {
            png::Compression::Balanced
        } else {
            png::Compression::NoCompression
        });
        let mut writer = encoder
            .write_header()
            .map_err(|error| PngError::Encode(error.to_string()))?;
        writer
            .write_image_data(rgba)
            .map_err(|error| PngError::Encode(error.to_string()))?;
        writer
            .finish()
            .map_err(|error| PngError::Encode(error.to_string()))?;
    }
    Ok(encoded)
}

pub fn decode_rgba_png(encoded: &[u8]) -> Result<Vec<u8>, PngError> {
    let decoder = png::Decoder::new_with_limits(
        Cursor::new(encoded),
        png::Limits {
            bytes: TILE_RGBA_BYTES * 2,
        },
    );
    let mut reader = decoder
        .read_info()
        .map_err(|error| PngError::Decode(error.to_string()))?;
    let info = reader.info();
    if info.width != TILE_SIZE as u32 || info.height != TILE_SIZE as u32 {
        return Err(PngError::InvalidDimensions {
            width: info.width,
            height: info.height,
        });
    }
    if info.color_type != png::ColorType::Rgba {
        return Err(PngError::InvalidColorType {
            actual: format!("{:?}", info.color_type),
        });
    }
    if info.bit_depth != png::BitDepth::Eight {
        return Err(PngError::InvalidBitDepth {
            actual: format!("{:?}", info.bit_depth),
        });
    }

    let output_size = reader
        .output_buffer_size()
        .ok_or_else(|| PngError::Decode("decoded size overflow".to_owned()))?;
    if output_size != TILE_RGBA_BYTES {
        return Err(PngError::Decode(format!(
            "decoded size is {output_size}; expected {TILE_RGBA_BYTES}"
        )));
    }
    let mut rgba = vec![0_u8; output_size];
    let frame = reader
        .next_frame(&mut rgba)
        .map_err(|error| PngError::Decode(error.to_string()))?;
    if frame.buffer_size() != TILE_RGBA_BYTES {
        return Err(PngError::Decode(format!(
            "decoded frame has {} bytes; expected {TILE_RGBA_BYTES}",
            frame.buffer_size()
        )));
    }
    reader
        .finish()
        .map_err(|error| PngError::Decode(error.to_string()))?;
    Ok(rgba)
}
