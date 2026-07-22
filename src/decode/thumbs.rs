//! Thumbnail generation with a disk cache under the app data dir.
//!
//! JPEGs are decoded and downscaled directly; RAWs use the embedded preview
//! JPEG (full-size on both the EOS RP and X100V) so no demosaic is needed.

use std::path::PathBuf;

use anyhow::{Context, Result};
use image::DynamicImage;

use crate::catalog::{thumbs_dir, FileKind, ImageRecord};

pub const THUMB_LONG_EDGE: u32 = 512;

fn cache_path(image_id: i64) -> PathBuf {
    thumbs_dir().join(format!("{image_id}.jpg"))
}

/// Produce the thumbnail for `rec`, from cache when possible.
pub fn get_or_create(rec: &ImageRecord) -> Result<image::RgbaImage> {
    let cached = cache_path(rec.id);
    if let Ok(img) = image::open(&cached) {
        return Ok(img.to_rgba8());
    }

    let thumb = generate(rec)?;
    // Cache write failures are not fatal; the thumb can be regenerated.
    if let Err(e) = thumb.to_rgb8().save(&cached) {
        tracing::warn!("failed to write thumb cache {}: {e}", cached.display());
    }
    Ok(thumb.to_rgba8())
}

fn generate(rec: &ImageRecord) -> Result<DynamicImage> {
    let img = match rec.kind {
        FileKind::Jpeg => {
            let img = image::open(&rec.abs_path).context("decode jpeg")?;
            apply_exif_orientation(img, rec.orientation)
        }
        FileKind::Raw => {
            let mut raw =
                libraw_ffi::RawFile::open(&rec.abs_path).context("open raw")?;
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
            apply_libraw_flip(img, flip)
        }
    };
    Ok(img.thumbnail(THUMB_LONG_EDGE, THUMB_LONG_EDGE))
}

/// EXIF orientation values 1-8.
pub fn apply_exif_orientation(img: DynamicImage, orientation: u32) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

/// LibRaw `sizes.flip`: 0 none, 3 = 180, 5 = 270 CW, 6 = 90 CW.
pub fn apply_libraw_flip(img: DynamicImage, flip: i32) -> DynamicImage {
    match flip {
        3 => img.rotate180(),
        5 => img.rotate270(),
        6 => img.rotate90(),
        _ => img,
    }
}
