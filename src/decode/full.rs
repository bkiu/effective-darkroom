//! Full-resolution decode for the loupe/develop views.
//!
//! JPEGs decode directly. For RAWs this currently uses the embedded full-size
//! JPEG preview (fast); M3 replaces it with the linear LibRaw decode while
//! keeping this as the instant first-paint.

use anyhow::{Context, Result};

use crate::catalog::{FileKind, ImageRecord};
use crate::decode::thumbs::{apply_exif_orientation, apply_libraw_flip};

pub enum FullPixels {
    /// sRGB-encoded 8-bit RGBA.
    Srgb8(Vec<u8>),
    /// Linear f16 RGBA bit patterns (RAW linear decode, M3).
    LinearF16(Vec<u16>),
}

pub struct FullImage {
    pub width: u32,
    pub height: u32,
    pub pixels: FullPixels,
}

pub fn decode_full(rec: &ImageRecord) -> Result<FullImage> {
    match rec.kind {
        FileKind::Jpeg => {
            let img = image::open(&rec.abs_path).context("decode jpeg")?;
            let img = apply_exif_orientation(img, rec.orientation).to_rgba8();
            Ok(FullImage {
                width: img.width(),
                height: img.height(),
                pixels: FullPixels::Srgb8(img.into_raw()),
            })
        }
        FileKind::Raw => {
            let mut raw = libraw_ffi::RawFile::open(&rec.abs_path).context("open raw")?;
            let flip = raw.metadata().flip;
            let img = match raw.extract_thumb().context("extract raw preview")? {
                libraw_ffi::Thumb::Jpeg(bytes) => {
                    image::load_from_memory(&bytes).context("decode embedded jpeg")?
                }
                libraw_ffi::Thumb::Rgb8 {
                    width,
                    height,
                    data,
                } => image::RgbImage::from_raw(width, height, data)
                    .context("bad embedded bitmap")?
                    .into(),
            };
            let img = apply_libraw_flip(img, flip).to_rgba8();
            Ok(FullImage {
                width: img.width(),
                height: img.height(),
                pixels: FullPixels::Srgb8(img.into_raw()),
            })
        }
    }
}
