//! Library/catalog maintenance: moving the photo library to a new folder
//! (relocating files and re-pointing catalog rows) and moving the catalog
//! (database + thumbnail cache) itself.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::catalog::Catalog;
use crate::jobs::JobResult;

#[derive(Debug, Clone, Default)]
pub struct MoveOutcome {
    pub moved: usize,
    pub skipped_outside: usize,
    pub errors: Vec<String>,
    pub cancelled: bool,
}

/// Move a file, falling back to copy+remove across filesystems.
fn move_file(src: &Path, dest: &Path) -> Result<()> {
    if std::fs::rename(src, dest).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dest).with_context(|| format!("copy to {}", dest.display()))?;
    std::fs::remove_file(src).context("remove source after copy")?;
    Ok(())
}

/// Move every catalog photo under `old_root` to `new_root`, preserving the
/// relative folder structure and updating catalog paths. Photos outside
/// `old_root` are left where they are.
pub fn move_library_blocking(
    old_root: &Path,
    new_root: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize, &str),
) -> Result<MoveOutcome> {
    anyhow::ensure!(old_root != new_root, "destination equals the current location");
    anyhow::ensure!(
        !new_root.starts_with(old_root),
        "destination must not be inside the current library"
    );
    std::fs::create_dir_all(new_root).context("create new library root")?;

    let catalog = Catalog::open_default().context("open catalog")?;
    let images = catalog.all_images()?;
    let movable: Vec<_> = images
        .iter()
        .filter(|r| r.abs_path.starts_with(old_root))
        .collect();
    let total = movable.len();
    let mut outcome = MoveOutcome {
        skipped_outside: images.len() - total,
        ..Default::default()
    };

    for (i, rec) in movable.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            outcome.cancelled = true;
            break;
        }
        progress(i + 1, total, &rec.file_name);
        let rel = rec
            .abs_path
            .strip_prefix(old_root)
            .expect("filtered by starts_with");
        let dest = new_root.join(rel);
        if dest.exists() {
            outcome
                .errors
                .push(format!("{}: already exists at destination", rec.file_name));
            continue;
        }
        let result = (|| -> Result<()> {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).context("create destination folder")?;
            }
            move_file(&rec.abs_path, &dest)?;
            // Path update failure after a successful move must move the file
            // back, or the catalog would point at nothing.
            if let Err(e) = catalog.update_abs_path(rec.id, &dest) {
                let _ = move_file(&dest, &rec.abs_path);
                return Err(e).context("update catalog path");
            }
            Ok(())
        })();
        match result {
            Ok(()) => outcome.moved += 1,
            Err(e) => outcome.errors.push(format!("{}: {e:#}", rec.file_name)),
        }
    }

    // Sweep now-empty date folders left behind.
    remove_empty_dirs(old_root);
    Ok(outcome)
}

/// Spawn the library move on a background thread, streaming progress.
pub fn move_library(
    old_root: PathBuf,
    new_root: PathBuf,
    results: crossbeam_channel::Sender<JobResult>,
    ctx: egui::Context,
    cancel: Arc<AtomicBool>,
) {
    std::thread::Builder::new()
        .name("move-library".into())
        .spawn(move || {
            let outcome =
                move_library_blocking(&old_root, &new_root, &cancel, |cur, total, name| {
                    let _ = results.send(JobResult::MoveLibraryProgress {
                        current: cur,
                        total,
                        file_name: name.to_string(),
                    });
                    ctx.request_repaint();
                })
                .unwrap_or_else(|e| MoveOutcome {
                    errors: vec![format!("move failed: {e:#}")],
                    ..Default::default()
                });
            let _ = results.send(JobResult::MoveLibraryDone(outcome));
            ctx.request_repaint();
        })
        .expect("spawn move-library thread");
}

/// Best-effort removal of empty directories under (and including) `root`.
fn remove_empty_dirs(root: &Path) {
    fn is_empty_after_sweep(dir: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        let mut empty = true;
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() && is_empty_after_sweep(&p) {
                let _ = std::fs::remove_dir(&p);
            }
            if p.exists() {
                empty = false;
            }
        }
        empty
    }
    is_empty_after_sweep(root);
}

/// Move the catalog database and thumbnail cache to `new_dir`. The caller
/// must have dropped/reopen the live catalog connection around this call.
pub fn move_catalog_files(current_dir: &Path, new_dir: &Path) -> Result<()> {
    anyhow::ensure!(current_dir != new_dir, "destination equals the current location");
    std::fs::create_dir_all(new_dir).context("create catalog directory")?;

    for name in ["catalog.db", "catalog.db-wal", "catalog.db-shm"] {
        let src = current_dir.join(name);
        if src.exists() {
            move_file(&src, &new_dir.join(name)).with_context(|| format!("move {name}"))?;
        }
    }
    let old_thumbs = current_dir.join("thumbs");
    let new_thumbs = new_dir.join("thumbs");
    if old_thumbs.is_dir() {
        if std::fs::rename(&old_thumbs, &new_thumbs).is_err() {
            std::fs::create_dir_all(&new_thumbs).context("create thumbs dir")?;
            for entry in std::fs::read_dir(&old_thumbs)?.flatten() {
                let _ = move_file(&entry.path(), &new_thumbs.join(entry.file_name()));
            }
            let _ = std::fs::remove_dir(&old_thumbs);
        }
    } else {
        std::fs::create_dir_all(&new_thumbs).context("create thumbs dir")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_catalog_files_roundtrip() {
        let base = std::env::temp_dir().join(format!("ed-mv-{}", std::process::id()));
        let a = base.join("a");
        let b = base.join("b");
        std::fs::create_dir_all(a.join("thumbs")).unwrap();
        std::fs::write(a.join("catalog.db"), b"db").unwrap();
        std::fs::write(a.join("thumbs/1.jpg"), b"t").unwrap();

        move_catalog_files(&a, &b).unwrap();
        assert!(b.join("catalog.db").exists());
        assert!(b.join("thumbs/1.jpg").exists());
        assert!(!a.join("catalog.db").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn empty_dir_sweep() {
        let base = std::env::temp_dir().join(format!("ed-sweep-{}", std::process::id()));
        std::fs::create_dir_all(base.join("2026/07-01")).unwrap();
        std::fs::create_dir_all(base.join("2026/07-02")).unwrap();
        std::fs::write(base.join("2026/07-02/keep.jpg"), b"x").unwrap();

        remove_empty_dirs(&base);
        assert!(!base.join("2026/07-01").exists());
        assert!(base.join("2026/07-02/keep.jpg").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
