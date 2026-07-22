//! Batch export: render each image at full resolution through the same GPU
//! pipeline as the preview, then resize/encode to JPEG.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{Local, TimeZone};

use crate::catalog::{Catalog, FileKind, ImageRecord};
use crate::decode::full::{self, FullPixels};
use crate::engine::curve::BaseCurve;
use crate::engine::params::EditParams;
use crate::engine::{RenderEngine, SourcePixels};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Naming {
    /// Keep the original file stem.
    Original,
    /// YYYYMMDD_HHMMSS from the capture time.
    DateTime,
}

#[derive(Debug, Clone)]
pub struct ExportSpec {
    pub image_ids: Vec<i64>,
    pub dest: PathBuf,
    /// JPEG quality 1-100.
    pub quality: u8,
    /// Resize so the long edge is at most this many pixels.
    pub long_edge: Option<u32>,
    /// Optional filename prefix.
    pub prefix: String,
    pub naming: Naming,
}

#[derive(Debug, Clone, Default)]
pub struct ExportOutcome {
    pub exported: usize,
    pub errors: Vec<String>,
}

/// Output filename (without collision handling) for an image record.
pub fn export_file_name(spec: &ExportSpec, rec: &ImageRecord) -> String {
    let stem = match spec.naming {
        Naming::Original => Path::new(&rec.file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| rec.file_name.clone()),
        Naming::DateTime => rec
            .capture_ts
            .and_then(|ts| Local.timestamp_opt(ts, 0).earliest())
            .map(|dt| dt.format("%Y%m%d_%H%M%S").to_string())
            .unwrap_or_else(|| {
                Path::new(&rec.file_name)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| rec.file_name.clone())
            }),
    };
    format!("{}{}.jpg", spec.prefix, stem)
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, "jpg"));
    for i in 1.. {
        let candidate = dir.join(format!("{stem}-{i}.{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

/// Run a full export synchronously (call from a worker thread). `progress`
/// receives (index, total, file name) before each image.
pub fn run_blocking(
    engine: &mut RenderEngine,
    catalog: &Catalog,
    spec: &ExportSpec,
    mut progress: impl FnMut(usize, usize, &str),
) -> Result<ExportOutcome> {
    std::fs::create_dir_all(&spec.dest).context("create export directory")?;
    let all = catalog.all_images()?;
    let total = spec.image_ids.len();
    let mut outcome = ExportOutcome::default();

    for (i, id) in spec.image_ids.iter().enumerate() {
        let Some(rec) = all.iter().find(|r| r.id == *id) else {
            outcome.errors.push(format!("image #{id} not in catalog"));
            continue;
        };
        progress(i + 1, total, &rec.file_name);
        match export_one(engine, catalog, spec, rec) {
            Ok(()) => outcome.exported += 1,
            Err(e) => outcome.errors.push(format!("{}: {e:#}", rec.file_name)),
        }
    }
    Ok(outcome)
}

fn export_one(
    engine: &mut RenderEngine,
    catalog: &Catalog,
    spec: &ExportSpec,
    rec: &ImageRecord,
) -> Result<()> {
    // Decode at full quality.
    let image = match rec.kind {
        FileKind::Jpeg => full::decode_preview(rec)?,
        FileKind::Raw => full::decode_linear_raw(rec)?,
    };

    // Edit params: saved edit or defaults (+ as-shot WB adoption for RAW).
    let mut params = match catalog.load_edit(rec.id)? {
        Some((_ver, json)) => EditParams::from_json(&json),
        None => {
            let mut p = EditParams::default();
            if rec.kind == FileKind::Jpeg {
                p.color_nr = 0.0;
            }
            p
        }
    };
    let (ref_temp, ref_tint) = match image.as_shot {
        Some(w) => (w.temp, w.tint),
        None => (crate::engine::params::TEMP_DEFAULT, 0.0),
    };
    if rec.kind == FileKind::Raw
        && params.wb_temp == crate::engine::params::TEMP_DEFAULT
        && params.wb_tint == 0.0
    {
        params.wb_temp = ref_temp;
        params.wb_tint = ref_tint;
    }
    let base = (rec.kind == FileKind::Raw).then(|| BaseCurve::for_camera(&rec.camera_model));

    match &image.pixels {
        FullPixels::Srgb8(rgba) => {
            engine.set_source(image.width, image.height, SourcePixels::Srgb8(rgba))
        }
        FullPixels::LinearF16(rgba) => {
            engine.set_source(image.width, image.height, SourcePixels::LinearF16(rgba))
        }
    }
    let _ = engine.render(&params, ref_temp, ref_tint, base.as_ref());
    let rendered = engine.read_output().context("GPU readback")?;

    let final_img = match spec.long_edge {
        Some(edge) if edge > 0 && edge < rendered.width().max(rendered.height()) => {
            resize_long_edge(&rendered, edge)?
        }
        _ => rendered,
    };

    let out_path = unique_path(&spec.dest, &export_file_name(spec, rec));
    let file = std::fs::File::create(&out_path).context("create output file")?;
    let mut writer = std::io::BufWriter::new(file);
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
        &mut writer,
        spec.quality.clamp(1, 100),
    );
    let rgb = image::DynamicImage::ImageRgba8(final_img).to_rgb8();
    encoder
        .encode(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        .context("encode jpeg")?;
    Ok(())
}

fn resize_long_edge(img: &image::RgbaImage, edge: u32) -> Result<image::RgbaImage> {
    use fast_image_resize as fir;
    let (w, h) = (img.width(), img.height());
    let scale = edge as f32 / w.max(h) as f32;
    let (nw, nh) = (
        ((w as f32 * scale).round() as u32).max(1),
        ((h as f32 * scale).round() as u32).max(1),
    );
    let src = fir::images::Image::from_vec_u8(
        w,
        h,
        img.as_raw().clone(),
        fir::PixelType::U8x4,
    )
    .context("wrap source for resize")?;
    let mut dst = fir::images::Image::new(nw, nh, fir::PixelType::U8x4);
    let opts = fir::ResizeOptions::new()
        .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Lanczos3));
    fir::Resizer::new()
        .resize(&src, &mut dst, &opts)
        .context("resize")?;
    image::RgbaImage::from_raw(nw, nh, dst.into_vec()).context("rewrap resized image")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(name: &str, ts: Option<i64>) -> ImageRecord {
        ImageRecord {
            id: 1,
            abs_path: PathBuf::from(format!("/lib/{name}")),
            file_name: name.into(),
            kind: FileKind::Raw,
            capture_ts: ts,
            imported_ts: 0,
            width: 0,
            height: 0,
            orientation: 0,
            camera_make: String::new(),
            camera_model: String::new(),
            file_size: 0,
        }
    }

    #[test]
    fn naming_schemes() {
        let mut spec = ExportSpec {
            image_ids: vec![],
            dest: PathBuf::new(),
            quality: 85,
            long_edge: None,
            prefix: String::new(),
            naming: Naming::Original,
        };
        assert_eq!(export_file_name(&spec, &rec("IMG_4186.CR3", None)), "IMG_4186.jpg");
        spec.prefix = "trip-".into();
        assert_eq!(
            export_file_name(&spec, &rec("IMG_4186.CR3", None)),
            "trip-IMG_4186.jpg"
        );
        spec.naming = Naming::DateTime;
        let name = export_file_name(&spec, &rec("IMG_4186.CR3", Some(1_775_364_006)));
        assert!(name.starts_with("trip-2026"), "{name}");
        assert!(name.ends_with(".jpg"));
        // Missing capture date falls back to the stem.
        assert_eq!(
            export_file_name(&spec, &rec("IMG_4186.CR3", None)),
            "trip-IMG_4186.jpg"
        );
    }
}
