//! Full-resolution decode for the loupe/develop views.
//!
//! JPEGs decode directly to sRGB8. RAWs decode in two phases: the embedded
//! full-size JPEG preview first (instant paint), then the real linear 16-bit
//! LibRaw decode (demosaic, camera WB, linear sRGB primaries) as f16.

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::catalog::{FileKind, ImageRecord};
use crate::decode::thumbs::{apply_exif_orientation, apply_libraw_flip};
use crate::engine::color;

pub enum FullPixels {
    /// sRGB-encoded 8-bit RGBA.
    Srgb8(Vec<u8>),
    /// Linear f16 RGBA bit patterns (RAW linear decode).
    LinearF16(Vec<u16>),
}

/// As-shot white balance estimate for a RAW file, in slider units.
#[derive(Debug, Clone, Copy)]
pub struct AsShotWb {
    pub temp: f32,
    pub tint: f32,
}

pub struct FullImage {
    pub width: u32,
    pub height: u32,
    pub pixels: FullPixels,
    pub as_shot: Option<AsShotWb>,
    /// Capture metadata for the info overlay.
    pub shot_info: crate::importer::metadata::CaptureInfo,
}

/// Fast first paint: the full JPEG for JPEG files, the embedded preview for
/// RAWs.
pub fn decode_preview(rec: &ImageRecord) -> Result<FullImage> {
    let shot_info = crate::importer::metadata::read(&rec.abs_path, rec.kind);
    match rec.kind {
        FileKind::Jpeg => {
            let img = image::open(&rec.abs_path).context("decode jpeg")?;
            let img = apply_exif_orientation(img, rec.orientation).to_rgba8();
            Ok(FullImage {
                width: img.width(),
                height: img.height(),
                pixels: FullPixels::Srgb8(img.into_raw()),
                as_shot: None,
                shot_info,
            })
        }
        FileKind::Raw => {
            let mut raw = libraw_ffi::RawFile::open(&rec.abs_path).context("open raw")?;
            let md = raw.metadata();
            let flip = md.flip;
            let as_shot = color::as_shot_wb(raw.cam_mul(), raw.cam_xyz(), &md.model)
                .map(|(temp, tint)| AsShotWb { temp, tint });
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
                as_shot,
                shot_info,
            })
        }
    }
}

/// The real linear decode for RAW files (seconds for X-Trans; run on the
/// decode thread).
pub fn decode_linear_raw(rec: &ImageRecord) -> Result<FullImage> {
    let shot_info = crate::importer::metadata::read(&rec.abs_path, rec.kind);
    let mut raw = libraw_ffi::RawFile::open(&rec.abs_path).context("open raw")?;
    let model = raw.metadata().model;
    let as_shot = color::as_shot_wb(raw.cam_mul(), raw.cam_xyz(), &model)
        .map(|(temp, tint)| AsShotWb { temp, tint });
    let decoded = raw.decode_linear(3).context("linear decode")?;
    let rgba = rgb16_to_rgba_f16(&decoded.rgb16);
    Ok(FullImage {
        width: decoded.width,
        height: decoded.height,
        pixels: FullPixels::LinearF16(rgba),
        as_shot,
        shot_info,
    })
}

/// Interleaved u16 RGB (0..65535 linear) → RGBA f16 bit patterns.
fn rgb16_to_rgba_f16(rgb: &[u16]) -> Vec<u16> {
    const ONE_F16: u16 = 0x3C00;
    let px_count = rgb.len() / 3;
    let mut out = vec![0u16; px_count * 4];
    out.par_chunks_mut(4 * 4096)
        .zip(rgb.par_chunks(3 * 4096))
        .for_each(|(dst, src)| {
            for (d, s) in dst.chunks_exact_mut(4).zip(s_chunks(src)) {
                d[0] = half::f16::from_f32(s[0] as f32 / 65535.0).to_bits();
                d[1] = half::f16::from_f32(s[1] as f32 / 65535.0).to_bits();
                d[2] = half::f16::from_f32(s[2] as f32 / 65535.0).to_bits();
                d[3] = ONE_F16;
            }
        });
    out
}

fn s_chunks(src: &[u16]) -> impl Iterator<Item = &[u16]> {
    src.chunks_exact(3)
}
