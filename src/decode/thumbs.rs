//! Thumbnail generation with a disk cache under the app data dir.
//!
//! JPEGs are decoded and downscaled directly; RAWs use the embedded preview
//! JPEG (full-size on both the EOS RP and X100V) so no demosaic is needed.
//!
//! Photos with a saved edit get a second thumbnail rendered through the
//! develop pipeline from that same source (`get_or_create_edited`). Its cache
//! file is keyed by a hash of the edit JSON, so any change to the edit, from
//! a slider, a paste or an imported bundle, names a new file and the stale
//! one is never served.

use std::path::PathBuf;

use anyhow::{Context, Result};
use image::DynamicImage;

use crate::catalog::{thumbs_dir, FileKind, ImageRecord};
use crate::decode::full::{FullImage, FullPixels};

pub const THUMB_LONG_EDGE: u32 = 512;

fn cache_path(image_id: i64) -> PathBuf {
    thumbs_dir().join(format!("{image_id}.jpg"))
}

/// Bump to invalidate every edited thumbnail after a pipeline change that
/// alters how saved edits render.
const EDITED_RENDER_VERSION: u32 = 1;

fn edited_prefix(image_id: i64) -> String {
    format!("{image_id}-e")
}

fn edited_cache_path(image_id: i64, edit_json: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (EDITED_RENDER_VERSION, edit_json).hash(&mut h);
    thumbs_dir().join(format!("{}{:016x}.jpg", edited_prefix(image_id), h.finish()))
}

/// Delete every cached thumbnail of `image_id`, edited ones included.
pub fn remove_cached(image_id: i64) {
    let _ = std::fs::remove_file(cache_path(image_id));
    remove_edited_except(image_id, None);
}

fn remove_edited_except(image_id: i64, keep: Option<&std::path::Path>) {
    let prefix = edited_prefix(image_id);
    let Ok(entries) = std::fs::read_dir(thumbs_dir()) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let ours = entry.file_name().to_str().is_some_and(|n| n.starts_with(&prefix));
        if ours && Some(path.as_path()) != keep {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Long edge the source is cut to when the GPU cannot take the full-size
/// preview; still enough that clarity and texture radii look about right.
const FALLBACK_SOURCE_EDGE: u32 = 2048;

/// The thumbnail of `rec` with its saved edit (`edit_json`) applied, from
/// cache when possible. `render` runs the develop pipeline over a decode.
///
/// The source is the same fast decode the develop view paints first (the
/// camera JPEG, or a RAW's embedded preview, at full size, so pixel-radius
/// effects such as clarity match the editor), not the linear RAW decode,
/// which takes seconds per frame. RAW thumbnails therefore show the camera's
/// tone rendering under the edit rather than our base curve, the same
/// approximation the editor shows while the full decode loads.
pub fn get_or_create_edited(
    rec: &ImageRecord,
    edit_json: &str,
    mut render: impl FnMut(&FullImage) -> Result<image::RgbaImage>,
) -> Result<image::RgbaImage> {
    let cached = edited_cache_path(rec.id, edit_json);
    if let Ok(img) = image::open(&cached) {
        return Ok(img.to_rgba8());
    }

    let source = crate::decode::full::decode_preview(rec)?;
    let rendered = match render(&source) {
        Ok(img) => img,
        Err(e) => {
            tracing::debug!("{}: full-size edited thumb failed ({e:#}); retrying smaller", rec.file_name);
            render(&shrink(source, FALLBACK_SOURCE_EDGE)?)?
        }
    };
    let thumb = DynamicImage::ImageRgba8(rendered).thumbnail(THUMB_LONG_EDGE, THUMB_LONG_EDGE);
    match thumb.to_rgb8().save(&cached) {
        Ok(()) => remove_edited_except(rec.id, Some(&cached)),
        Err(e) => tracing::warn!("failed to write thumb cache {}: {e}", cached.display()),
    }
    Ok(thumb.to_rgba8())
}

/// Downscale an 8-bit decode to `edge` on its long side.
fn shrink(mut image: FullImage, edge: u32) -> Result<FullImage> {
    let FullPixels::Srgb8(rgba) = image.pixels else {
        anyhow::bail!("expected an 8-bit preview decode");
    };
    let img = image::RgbaImage::from_raw(image.width, image.height, rgba)
        .context("bad preview buffer")?;
    let small = DynamicImage::ImageRgba8(img).thumbnail(edge, edge).to_rgba8();
    image.width = small.width();
    image.height = small.height();
    image.pixels = FullPixels::Srgb8(small.into_raw());
    Ok(image)
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

/// Ensure the disk-cache entry exists without returning pixels; used by the
/// background prewarm. Returns true when a thumbnail was generated.
pub fn warm(rec: &ImageRecord) -> Result<bool> {
    let cached = cache_path(rec.id);
    if cached.exists() {
        return Ok(false);
    }
    let thumb = generate(rec)?;
    thumb
        .to_rgb8()
        .save(&cached)
        .with_context(|| format!("write thumb cache {}", cached.display()))?;
    Ok(true)
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
