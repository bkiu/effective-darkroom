//! Camera import smoke test: detect, list card files, download a couple and
//! import them into the (XDG_DATA_HOME-scoped) catalog.
//!
//!   cargo run --example camera_cli -- <dest_root> [count]

use effective_darkroom::importer::camera;
use effective_darkroom::importer::{run_blocking, ImportMode, ImportSource, ImportSpec};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dest = std::path::PathBuf::from(args.next().expect("usage: camera_cli <dest> [count]"));
    let count: usize = args.next().and_then(|c| c.parse().ok()).unwrap_or(2);

    let cams = camera::detect()?;
    println!("cameras: {cams:?}");
    let Some(desc) = cams.first() else {
        anyhow::bail!("no camera detected");
    };

    let cam = camera::connect(desc)?;
    let files = camera::list_card_files(&cam)?;
    println!("{} importable files on card", files.len());
    let catalog = effective_darkroom::catalog::Catalog::open_default()?;
    for f in files.iter().take(3) {
        let size = camera::file_size(&cam, f);
        let dup = catalog.contains_name(&f.name).unwrap_or(false)
            && size
                .map(|s| catalog.contains_similar(&f.name, s).unwrap_or(false))
                .unwrap_or(false);
        println!("  {}/{} size={size:?} already-imported={dup}", f.folder, f.name);
    }

    // Download the first `count` files to a staging dir.
    let staging = dest.join("camera-staging");
    std::fs::create_dir_all(&staging)?;
    let mut staged = Vec::new();
    for f in files.iter().take(count) {
        let path = staging.join(&f.name);
        let t = std::time::Instant::now();
        camera::download(&cam, f, &path)?;
        println!(
            "downloaded {} ({} bytes) in {:?}",
            f.name,
            std::fs::metadata(&path)?.len(),
            t.elapsed()
        );
        staged.push(path);
    }
    drop(cam);

    // Import them through the normal pipeline (move out of staging).
    let outcome = run_blocking(
        &ImportSpec {
            source: ImportSource::Paths(staged),
            dest_root: dest.clone(),
            mode: ImportMode::Move,
        },
        |cur, total, name| println!("[{cur}/{total}] {name}"),
    )?;
    println!(
        "imported={} duplicates={} errors={:?}",
        outcome.imported, outcome.skipped_duplicates, outcome.errors
    );
    let _ = std::fs::remove_dir(&staging);
    Ok(())
}
