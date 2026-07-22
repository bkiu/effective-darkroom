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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExportFormat {
    /// Rendered JPEG through the develop pipeline.
    Jpeg,
    /// Lossless DNG conversion of the original raw data (RAW sources only;
    /// develop edits are not baked in).
    Dng,
}

#[derive(Debug, Clone)]
pub struct ExportSpec {
    pub image_ids: Vec<i64>,
    pub dest: PathBuf,
    pub format: ExportFormat,
    /// For DNG: bake the develop edits into the pixels (linear DNG) instead
    /// of converting the untouched raw data.
    pub dng_bake: bool,
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
    let ext = match spec.format {
        ExportFormat::Jpeg => "jpg",
        ExportFormat::Dng => "dng",
    };
    format!("{}{}.{}", spec.prefix, stem, ext)
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
    if spec.format == ExportFormat::Dng {
        return if spec.dng_bake {
            export_one_dng_baked(engine, catalog, spec, rec)
        } else {
            export_one_dng(spec, rec)
        };
    }
    let rendered = render_full(engine, catalog, rec)?;

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

/// Render an image at full resolution through the develop pipeline with its
/// saved edits (the same path the preview uses).
fn render_full(
    engine: &mut RenderEngine,
    catalog: &Catalog,
    rec: &ImageRecord,
) -> Result<image::RgbaImage> {
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
    engine.read_output().context("GPU readback")
}

/// Lossless DNG conversion of the original raw file (rawler / DNGLab).
/// Develop edits are not baked in; the DNG holds the untouched raw data.
fn export_one_dng(spec: &ExportSpec, rec: &ImageRecord) -> Result<()> {
    anyhow::ensure!(
        rec.kind == FileKind::Raw,
        "DNG export needs a RAW source (this is a JPEG)"
    );
    let out_path = unique_path(&spec.dest, &export_file_name(spec, rec));
    let file = std::fs::File::create(&out_path).context("create output file")?;
    let mut writer = std::io::BufWriter::new(file);
    let params = rawler::dng::convert::ConvertParams {
        software: "Effective Darkroom".into(),
        // Matches Adobe's default: don't embed a full copy of the original
        // (the library keeps the original anyway; embedding doubles the size).
        embedded: false,
        ..Default::default()
    };
    rawler::dng::convert::convert_raw_file(&rec.abs_path, &mut writer, &params)
        .map_err(|e| anyhow::anyhow!("DNG conversion: {e}"))?;
    Ok(())
}

/// Linear DNG with the develop edits baked into the pixels: the rendered
/// output is converted back to linear light and stored as 16-bit RGB
/// (LinearRaw), tagged as sRGB-primaries D65 so other apps reproduce the
/// look. EXIF is carried over from the original raw file.
fn export_one_dng_baked(
    engine: &mut RenderEngine,
    catalog: &Catalog,
    spec: &ExportSpec,
    rec: &ImageRecord,
) -> Result<()> {
    use rawler::dng::writer::DngWriter;
    use rawler::dng::{DngCompression, DNG_VERSION_V1_4};
    use rawler::formats::tiff::{Rational, SRational};
    use rawler::imgop::xyz::Illuminant;
    use rawler::tags::DngTag;

    anyhow::ensure!(
        rec.kind == FileKind::Raw,
        "DNG export needs a RAW source (this is a JPEG)"
    );
    let rendered = render_full(engine, catalog, rec)?;
    let (w, h) = (rendered.width() as usize, rendered.height() as usize);

    // sRGB u8 → linear u16 lookup.
    let lut: Vec<u16> = (0..256u32)
        .map(|v| {
            let x = v as f64 / 255.0;
            let lin = if x <= 0.04045 {
                x / 12.92
            } else {
                ((x + 0.055) / 1.055).powf(2.4)
            };
            (lin * 65535.0).round() as u16
        })
        .collect();
    let mut data = Vec::with_capacity(w * h * 3);
    for px in rendered.pixels() {
        data.push(lut[px[0] as usize]);
        data.push(lut[px[1] as usize]);
        data.push(lut[px[2] as usize]);
    }

    let out_path = unique_path(&spec.dest, &export_file_name(spec, rec));
    let file = std::fs::File::create(&out_path).context("create output file")?;
    let mut writer = std::io::BufWriter::new(file);
    let mut dng = DngWriter::new(&mut writer, DNG_VERSION_V1_4)
        .map_err(|e| anyhow::anyhow!("DNG writer: {e}"))?;

    let mut frame = dng.subframe(0);
    frame
        .rgb_image_u16(&data, w, h, DngCompression::Lossless, 1)
        .map_err(|e| anyhow::anyhow!("write linear DNG frame: {e}"))?;
    frame.finalize().map_err(|e| anyhow::anyhow!("finalize frame: {e}"))?;

    // Thumbnail + preview from the rendered image.
    let dynimg = image::DynamicImage::ImageRgba8(rendered);
    let mut preview = dng.subframe(1);
    preview
        .preview(&dynimg, 0.8)
        .map_err(|e| anyhow::anyhow!("write preview: {e}"))?;
    preview.finalize().map_err(|e| anyhow::anyhow!("finalize preview: {e}"))?;
    dng.thumbnail(&dynimg)
        .map_err(|e| anyhow::anyhow!("write thumbnail: {e}"))?;

    // Color tags: the stored data is linear with sRGB primaries, so
    // ColorMatrix1 (XYZ → "camera") is the standard XYZ→sRGB matrix.
    const XYZ_TO_SRGB: [[f64; 3]; 3] = [
        [3.2404542, -1.5371385, -0.4985314],
        [-0.9692660, 1.8760108, 0.0415560],
        [0.0556434, -0.2040259, 1.0572252],
    ];
    let matrix: Vec<SRational> = XYZ_TO_SRGB
        .iter()
        .flatten()
        .map(|v| SRational::new((v * 10000.0).round() as i32, 10000))
        .collect();
    dng.color_matrix(1, Illuminant::D65, &matrix);
    dng.as_shot_neutral([Rational::new(1, 1), Rational::new(1, 1), Rational::new(1, 1)]);

    // Carry EXIF over from the original raw file (best effort).
    let unique_model = format!("{} {}", rec.camera_make, rec.camera_model);
    if let Ok(src) = rawler::rawsource::RawSource::new(&rec.abs_path) {
        if let Ok(decoder) = rawler::get_decoder(&src) {
            if let Ok(md) = decoder.raw_metadata(&src, &Default::default()) {
                if let Err(e) = dng.load_metadata(&md) {
                    tracing::warn!("DNG metadata carry-over failed: {e}");
                }
            }
        }
    }
    dng.root_ifd_mut()
        .add_tag(DngTag::UniqueCameraModel, unique_model.trim());

    dng.close().map_err(|e| anyhow::anyhow!("finish DNG: {e}"))?;
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
            flag: crate::catalog::Flag::Unflagged,
        }
    }

    #[test]
    fn naming_schemes() {
        let mut spec = ExportSpec {
            image_ids: vec![],
            dest: PathBuf::new(),
            format: ExportFormat::Jpeg,
            dng_bake: false,
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
