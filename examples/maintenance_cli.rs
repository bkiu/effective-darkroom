//! Headless test for library/catalog moves.
//!
//!   cargo run --example maintenance_cli -- move-library <old_root> <new_root>
//!   cargo run --example maintenance_cli -- move-catalog <new_dir>

use effective_darkroom::catalog::{self, Catalog};
use effective_darkroom::maintenance;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args[0].as_str() {
        "move-library" => {
            let old = std::path::PathBuf::from(&args[1]);
            let new = std::path::PathBuf::from(&args[2]);
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let outcome = maintenance::move_library_blocking(
                &old,
                &new,
                &cancel,
                |cur, total, name| println!("[{cur}/{total}] {name}"),
            )?;
            println!(
                "moved={} outside={} errors={:?} cancelled={}",
                outcome.moved, outcome.skipped_outside, outcome.errors, outcome.cancelled
            );
            for img in Catalog::open_default()?.all_images()? {
                println!("  {} exists={}", img.abs_path.display(), img.abs_path.exists());
            }
        }
        "move-catalog" => {
            let new_dir = std::path::PathBuf::from(&args[1]);
            let current = catalog::data_dir();
            println!("current catalog dir: {}", current.display());
            maintenance::move_catalog_files(&current, &new_dir)?;
            println!("moved to {}", new_dir.display());
        }
        other => anyhow::bail!("unknown command {other}"),
    }
    Ok(())
}
