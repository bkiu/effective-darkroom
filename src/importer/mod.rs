//! Background import: scan sources, read capture dates, copy/move files into
//! `dest_root/YYYY/MM-DD/`, and register them in the catalog.
//!
//! Runs on its own thread with its own catalog connection (WAL mode makes
//! concurrent access safe); progress is streamed back over the results channel.

pub mod metadata;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{Local, TimeZone};

use crate::catalog::{Catalog, FileKind, ImageRecord};
use crate::jobs::JobResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    Copy,
    Move,
}

#[derive(Debug, Clone)]
pub struct ImportSpec {
    /// Files and/or directories chosen by the user.
    pub sources: Vec<PathBuf>,
    pub dest_root: PathBuf,
    pub mode: ImportMode,
}

#[derive(Debug, Clone)]
pub struct ImportOutcome {
    pub imported: usize,
    pub skipped_duplicates: usize,
    pub errors: Vec<String>,
}

/// Expand directories and filter to importable extensions, sorted for a
/// stable order.
pub fn scan_sources(sources: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for src in sources {
        if src.is_dir() {
            for entry in walkdir::WalkDir::new(src)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let p = entry.path();
                if p.is_file() && FileKind::from_path(p).is_some() {
                    files.push(p.to_path_buf());
                }
            }
        } else if FileKind::from_path(src).is_some() {
            files.push(src.clone());
        }
    }
    files.sort();
    files.dedup();
    files
}

pub fn run(
    spec: ImportSpec,
    results: crossbeam_channel::Sender<JobResult>,
    ctx: egui::Context,
) {
    std::thread::Builder::new()
        .name("import".into())
        .spawn(move || {
            let outcome = run_blocking(&spec, |cur, total, name| {
                let _ = results.send(JobResult::ImportProgress {
                    current: cur,
                    total,
                    file_name: name.to_string(),
                });
                ctx.request_repaint();
            });
            let outcome = outcome.unwrap_or_else(|e| ImportOutcome {
                imported: 0,
                skipped_duplicates: 0,
                errors: vec![format!("import failed: {e:#}")],
            });
            let _ = results.send(JobResult::ImportDone(outcome));
            ctx.request_repaint();
        })
        .expect("spawn import thread");
}

pub fn run_blocking(
    spec: &ImportSpec,
    mut progress: impl FnMut(usize, usize, &str),
) -> Result<ImportOutcome> {
    let catalog = Catalog::open_default().context("open catalog")?;
    let files = scan_sources(&spec.sources);
    let total = files.len();
    let mut outcome = ImportOutcome {
        imported: 0,
        skipped_duplicates: 0,
        errors: Vec::new(),
    };

    for (i, src) in files.iter().enumerate() {
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        progress(i + 1, total, &name);
        match import_one(&catalog, spec, src) {
            Ok(ImportedFile::Added) => outcome.imported += 1,
            Ok(ImportedFile::Duplicate) => outcome.skipped_duplicates += 1,
            Err(e) => outcome.errors.push(format!("{name}: {e:#}")),
        }
    }
    Ok(outcome)
}

enum ImportedFile {
    Added,
    Duplicate,
}

fn import_one(catalog: &Catalog, spec: &ImportSpec, src: &Path) -> Result<ImportedFile> {
    let kind = FileKind::from_path(src).context("unsupported file type")?;
    let file_size = std::fs::metadata(src).context("stat source")?.len();
    let file_name = src
        .file_name()
        .context("source has no file name")?
        .to_string_lossy()
        .into_owned();

    if catalog.contains_path(src)? || catalog.contains_similar(&file_name, file_size)? {
        return Ok(ImportedFile::Duplicate);
    }

    let info = metadata::read(src, kind);
    let capture_ts = info.capture_ts.or_else(|| metadata::mtime_ts(src));

    let dest_dir = spec.dest_root.join(date_folder(capture_ts));
    std::fs::create_dir_all(&dest_dir).context("create destination folder")?;
    let dest = unique_dest(&dest_dir, &file_name);

    // Already-in-place (re-importing the destination itself) would be caught
    // by contains_path above; a same-named different file gets a suffix.
    match spec.mode {
        ImportMode::Copy => {
            std::fs::copy(src, &dest).context("copy file")?;
        }
        ImportMode::Move => {
            if std::fs::rename(src, &dest).is_err() {
                // Cross-device: copy then remove.
                std::fs::copy(src, &dest).context("copy file")?;
                std::fs::remove_file(src).context("remove source after copy")?;
            }
        }
    }

    let rec = ImageRecord {
        id: 0,
        file_name: dest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(file_name),
        abs_path: dest,
        kind,
        capture_ts,
        imported_ts: chrono::Utc::now().timestamp(),
        width: info.width,
        height: info.height,
        orientation: info.orientation,
        camera_make: info.make,
        camera_model: info.model,
        file_size,
    };
    catalog.insert_image(&rec).context("insert into catalog")?;
    Ok(ImportedFile::Added)
}

/// `YYYY/MM-DD` from a unix timestamp in local time; unknown dates group
/// under "unknown-date".
fn date_folder(capture_ts: Option<i64>) -> PathBuf {
    match capture_ts.and_then(|ts| Local.timestamp_opt(ts, 0).earliest()) {
        Some(dt) => PathBuf::from(dt.format("%Y/%m-%d").to_string()),
        None => PathBuf::from("unknown-date"),
    }
}

/// Append ` -1`, ` -2`, ... before the extension until the name is free.
fn unique_dest(dir: &Path, file_name: &str) -> PathBuf {
    let candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (file_name.to_string(), String::new()),
    };
    for i in 1.. {
        let candidate = dir.join(format!("{stem}-{i}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_folder_formats() {
        // 2026-07-18 12:00:00 local — just check the shape YYYY/MM-DD.
        let p = date_folder(Some(1_774_000_000));
        let s = p.to_string_lossy();
        assert_eq!(s.len(), "YYYY/MM-DD".len(), "{s}");
        assert_eq!(&s[4..5], "/");
        assert_eq!(date_folder(None), PathBuf::from("unknown-date"));
    }

    #[test]
    fn unique_dest_suffixes() {
        let dir = std::env::temp_dir().join(format!("ed-uniq-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jpg"), b"x").unwrap();
        assert_eq!(unique_dest(&dir, "a.jpg"), dir.join("a-1.jpg"));
        std::fs::write(dir.join("a-1.jpg"), b"x").unwrap();
        assert_eq!(unique_dest(&dir, "a.jpg"), dir.join("a-2.jpg"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
