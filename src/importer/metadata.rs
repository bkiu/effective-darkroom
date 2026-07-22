//! Capture metadata extraction at import time.
//!
//! JPEGs are read with kamadak-exif; RAWs (CR3/RAF) with a cheap LibRaw
//! header-only parse, since kamadak-exif does not understand CR3's ISO-BMFF
//! container.

use std::path::Path;

use chrono::NaiveDateTime;

use crate::catalog::FileKind;

#[derive(Debug, Default, Clone)]
pub struct CaptureInfo {
    /// Camera-local wall clock of the exposure, as unix seconds.
    pub capture_ts: Option<i64>,
    pub width: u32,
    pub height: u32,
    pub orientation: u32,
    pub make: String,
    pub model: String,
    /// Exposure settings; 0.0 when unknown.
    pub iso: f32,
    /// Shutter time in seconds.
    pub shutter: f32,
    pub aperture: f32,
    pub focal_len: f32,
}

pub fn read(path: &Path, kind: FileKind) -> CaptureInfo {
    match kind {
        FileKind::Jpeg => read_jpeg(path),
        FileKind::Raw => read_raw(path),
    }
}

fn read_raw(path: &Path) -> CaptureInfo {
    match libraw_ffi::RawFile::open(path) {
        Ok(raw) => {
            let md = raw.metadata();
            CaptureInfo {
                // LibRaw runs mktime() on the EXIF local time, so this is
                // already unix seconds for the local zone.
                capture_ts: (md.timestamp > 0).then_some(md.timestamp),
                width: md.width,
                height: md.height,
                orientation: 0, // LibRaw output is already rotated
                make: md.make,
                model: md.model,
                iso: md.iso,
                shutter: md.shutter,
                aperture: md.aperture,
                focal_len: md.focal_len,
            }
        }
        Err(err) => {
            tracing::warn!("libraw header parse failed for {}: {err}", path.display());
            CaptureInfo::default()
        }
    }
}

fn read_jpeg(path: &Path) -> CaptureInfo {
    let mut info = CaptureInfo::default();

    if let Ok((w, h)) = image::image_dimensions(path) {
        info.width = w;
        info.height = h;
    }

    let Ok(file) = std::fs::File::open(path) else {
        return info;
    };
    let mut reader = std::io::BufReader::new(file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) else {
        return info;
    };

    use exif::{In, Tag, Value};
    let field_str = |tag| {
        exif.get_field(tag, In::PRIMARY)
            .map(|f| f.display_value().to_string().trim_matches('"').to_string())
            .unwrap_or_default()
    };
    info.make = field_str(Tag::Make);
    info.model = field_str(Tag::Model);

    if let Some(f) = exif.get_field(Tag::Orientation, In::PRIMARY) {
        if let Some(v) = f.value.get_uint(0) {
            info.orientation = v;
        }
    }

    let rational = |tag| -> f32 {
        exif.get_field(tag, In::PRIMARY)
            .and_then(|f| match &f.value {
                Value::Rational(v) => v.first().map(|r| r.to_f64() as f32),
                _ => None,
            })
            .unwrap_or(0.0)
    };
    info.shutter = rational(Tag::ExposureTime);
    info.aperture = rational(Tag::FNumber);
    info.focal_len = rational(Tag::FocalLength);
    if let Some(f) = exif.get_field(Tag::PhotographicSensitivity, In::PRIMARY) {
        if let Some(v) = f.value.get_uint(0) {
            info.iso = v as f32;
        }
    }

    for tag in [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime] {
        if let Some(f) = exif.get_field(tag, In::PRIMARY) {
            if let Value::Ascii(ref v) = f.value {
                if let Some(bytes) = v.first() {
                    let s = String::from_utf8_lossy(bytes);
                    if let Ok(dt) = NaiveDateTime::parse_from_str(&s, "%Y:%m:%d %H:%M:%S") {
                        info.capture_ts = Some(local_ts(dt));
                        break;
                    }
                }
            }
        }
    }
    info
}

/// Interpret a naive camera-local datetime in the machine's local zone,
/// matching what LibRaw does for RAW timestamps.
fn local_ts(dt: NaiveDateTime) -> i64 {
    use chrono::{Local, TimeZone};
    Local
        .from_local_datetime(&dt)
        .earliest()
        .map(|d| d.timestamp())
        .unwrap_or_else(|| dt.and_utc().timestamp())
}

/// Fallback capture time from filesystem mtime.
pub fn mtime_ts(path: &Path) -> Option<i64> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    Some(
        mtime
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64,
    )
}
