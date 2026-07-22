//! Headless import for testing: import sources into a destination root.
//!
//!   cargo run --example import_cli -- <dest_root> <source>...
//!
//! Respects XDG_DATA_HOME, so point that somewhere disposable to avoid
//! touching your real catalog.

use effective_darkroom::catalog::Catalog;
use effective_darkroom::importer::{run_blocking, ImportMode, ImportSource, ImportSpec};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dest_root = std::path::PathBuf::from(args.next().expect("usage: import_cli <dest> <src>..."));
    let sources: Vec<_> = args.map(std::path::PathBuf::from).collect();
    assert!(!sources.is_empty(), "no sources given");

    let outcome = run_blocking(
        &ImportSpec {
            source: ImportSource::Paths(sources),
            dest_root,
            mode: ImportMode::Copy,
        },
        |cur, total, name| println!("[{cur}/{total}] {name}"),
    )?;
    println!(
        "imported={} duplicates={} errors={:?}",
        outcome.imported, outcome.skipped_duplicates, outcome.errors
    );

    let make_thumbs = std::env::var("MAKE_THUMBS").is_ok();
    let catalog = Catalog::open_default()?;
    for img in catalog.all_images()? {
        println!(
            "#{} {} [{}] {}x{} {} {}",
            img.id,
            img.abs_path.display(),
            img.kind.as_str(),
            img.width,
            img.height,
            img.camera_make,
            img.camera_model
        );
        if make_thumbs {
            let t = std::time::Instant::now();
            let thumb = effective_darkroom::decode::thumbs::get_or_create(&img)?;
            println!(
                "   thumb {}x{} in {:?}",
                thumb.width(),
                thumb.height(),
                t.elapsed()
            );
        }
    }
    Ok(())
}
