//! Run the auto-straighten detector on a photo file.
//!
//!   cargo run --example straighten_cli -- <file> [more files...]

use effective_darkroom::catalog::{FileKind, Flag, ImageRecord};
use effective_darkroom::decode::full::{self, FullPixels};
use effective_darkroom::engine::straighten;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "effective_darkroom=info".into()),
        )
        .init();
    for path in std::env::args().skip(1) {
        let input = std::path::PathBuf::from(&path);
        let kind = FileKind::from_path(&input).expect("unsupported file");
        let rec = ImageRecord {
            id: 0,
            abs_path: input.clone(),
            file_name: path.clone(),
            kind,
            capture_ts: None,
            imported_ts: 0,
            width: 0,
            height: 0,
            orientation: 1,
            camera_make: String::new(),
            camera_model: String::new(),
            file_size: 0,
            flag: Flag::Unflagged,
        };
        let img = full::decode_preview(&rec)?;
        let rgba = match img.pixels {
            FullPixels::Srgb8(data) => {
                image::RgbaImage::from_raw(img.width, img.height, data).unwrap()
            }
            _ => anyhow::bail!("unexpected pixel format"),
        };
        let t = std::time::Instant::now();
        match straighten::detect_angle(&rgba) {
            Some(a) => println!("{path}: suggested angle {a:+.2}° ({:?})", t.elapsed()),
            None => println!("{path}: no confident line direction ({:?})", t.elapsed()),
        }
    }
    Ok(())
}
