//! Moving develop work between catalogs on different machines.
//!
//! A bundle is a single JSON file holding, for every photo that carries
//! work, its flag, its develop edit and the collections it belongs to.
//! Photos are matched back by original file name + byte size — the pair the
//! importer already dedupes on — because `abs_path` means nothing on the
//! machine receiving the file.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, Flag, ImageRecord};

pub const FORMAT: &str = "effective-darkroom-metadata";
pub const FORMAT_VERSION: i64 = 1;
/// Extension for bundle files (`my-library.edmeta`).
pub const EXTENSION: &str = "edmeta";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub format: String,
    pub format_version: i64,
    pub exported_ts: i64,
    /// Where this came from, for whoever opens the file. Free text.
    #[serde(default)]
    pub source: String,
    /// How many photos the source catalog held in total, so the receiving
    /// side can tell "3 unmatched" from "3 of 4000 unmatched".
    #[serde(default)]
    pub source_photo_count: usize,
    pub photos: Vec<PhotoMeta>,
}

/// One photo's work. Everything but the match key is optional, so a photo
/// with only a flag costs one short line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhotoMeta {
    pub file_name: String,
    pub file_size: u64,
    /// Not used for matching; carried so a human can identify the row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_ts: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub camera_model: String,
    /// 1 pick, -1 reject, 0 unflagged.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub flag: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<EditMeta>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collections: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditMeta {
    pub params_version: i64,
    /// The develop params, inline rather than as an escaped string so the
    /// file stays readable and diffable.
    pub params: serde_json::Value,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

impl PhotoMeta {
    /// Whether this row carries anything worth transferring.
    fn has_work(&self) -> bool {
        self.flag != 0 || self.edit.is_some() || !self.collections.is_empty()
    }
}

/// Match key: file name (case-folded, since a trip through exFAT or a
/// case-insensitive share can change it) plus byte size.
fn match_key(file_name: &str, file_size: u64) -> (String, u64) {
    (file_name.to_ascii_lowercase(), file_size)
}

/// Local images grouped by match key. A key can hold several images when the
/// same file was imported twice from different folders.
fn index_by_key(images: &[ImageRecord]) -> HashMap<(String, u64), Vec<i64>> {
    let mut map: HashMap<(String, u64), Vec<i64>> = HashMap::new();
    for rec in images {
        map.entry(match_key(&rec.file_name, rec.file_size))
            .or_default()
            .push(rec.id);
    }
    map
}

// --- export ---

/// Collect every photo that carries work into a bundle. `source` is free
/// text shown to whoever opens the file (the library path is a good choice).
pub fn export(cat: &Catalog, source: &str) -> Result<Bundle> {
    let images = cat.all_images().context("read images")?;
    let edits = cat.all_edits().context("read edits")?;

    // id -> collection names it belongs to.
    let mut memberships: HashMap<i64, Vec<String>> = HashMap::new();
    for col in cat.list_collections().context("read collections")? {
        for id in cat.collection_image_ids(col.id)? {
            memberships.entry(id).or_default().push(col.name.clone());
        }
    }
    for names in memberships.values_mut() {
        names.sort();
    }

    let mut photos = Vec::new();
    for rec in &images {
        let edit = edits.get(&rec.id).map(|(version, json)| EditMeta {
            params_version: *version,
            // Inline the stored blob; fall back to a string if it somehow
            // isn't valid JSON, so one bad row can't fail the whole export.
            params: serde_json::from_str(json)
                .unwrap_or_else(|_| serde_json::Value::String(json.clone())),
        });
        let meta = PhotoMeta {
            file_name: rec.file_name.clone(),
            file_size: rec.file_size,
            capture_ts: rec.capture_ts,
            camera_model: rec.camera_model.clone(),
            flag: rec.flag.as_i64(),
            edit,
            collections: memberships.get(&rec.id).cloned().unwrap_or_default(),
        };
        if meta.has_work() {
            photos.push(meta);
        }
    }
    photos.sort_by(|a, b| {
        a.capture_ts
            .cmp(&b.capture_ts)
            .then_with(|| a.file_name.cmp(&b.file_name))
    });

    Ok(Bundle {
        format: FORMAT.to_string(),
        format_version: FORMAT_VERSION,
        exported_ts: chrono::Utc::now().timestamp(),
        source: source.to_string(),
        source_photo_count: images.len(),
        photos,
    })
}

pub fn write_file(path: &Path, bundle: &Bundle) -> Result<()> {
    let json = serde_json::to_string_pretty(bundle).context("serialize bundle")?;
    std::fs::write(path, json)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn read_file(path: &Path) -> Result<Bundle> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    let bundle: Bundle = serde_json::from_str(&text)
        .with_context(|| format!("{} is not a metadata bundle", path.display()))?;
    if bundle.format != FORMAT {
        bail!(
            "{} is not an Effective Darkroom metadata file",
            path.display()
        );
    }
    if bundle.format_version > FORMAT_VERSION {
        bail!(
            "{} was written by a newer version (format {}, this build reads {})",
            path.display(),
            bundle.format_version,
            FORMAT_VERSION
        );
    }
    Ok(bundle)
}

// --- import ---

/// What applying a bundle would do, shown for confirmation before anything
/// is written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportPlan {
    /// Rows in the file.
    pub photos: usize,
    /// Rows found in this library (a row can match more than one image).
    pub matched: usize,
    /// Rows with no photo here — usually not imported yet.
    pub unmatched: usize,
    /// Matched rows whose local photo already has an edit or a flag.
    pub conflicts: usize,
    /// Collections the bundle would create here, in name order.
    pub new_collections: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnConflict {
    /// Leave photos that already have work here untouched.
    Skip,
    /// Let the incoming file win.
    Overwrite,
}

#[derive(Debug, Clone, Copy)]
pub struct ApplyOptions {
    pub edits: bool,
    pub flags: bool,
    pub collections: bool,
    pub on_conflict: OnConflict,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            edits: true,
            flags: true,
            collections: true,
            on_conflict: OnConflict::Skip,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub edits_applied: usize,
    pub flags_applied: usize,
    pub collections_created: usize,
    pub memberships_added: usize,
    /// Rows left alone because the local photo already had work.
    pub skipped_conflicts: usize,
    pub unmatched: usize,
    pub errors: Vec<String>,
}

/// Local state that decides whether a row is a conflict: a stored edit, or a
/// flag other than unflagged.
fn local_work(images: &[ImageRecord], edited: &HashSet<i64>) -> HashSet<i64> {
    images
        .iter()
        .filter(|r| edited.contains(&r.id) || r.flag != Flag::Unflagged)
        .map(|r| r.id)
        .collect()
}

pub fn plan(cat: &Catalog, bundle: &Bundle) -> Result<ImportPlan> {
    let images = cat.all_images()?;
    let edited = cat.edited_image_ids()?;
    let index = index_by_key(&images);
    let has_work = local_work(&images, &edited);

    let mut plan = ImportPlan {
        photos: bundle.photos.len(),
        ..Default::default()
    };
    let mut wanted_collections: HashSet<&str> = HashSet::new();
    for photo in &bundle.photos {
        match index.get(&match_key(&photo.file_name, photo.file_size)) {
            Some(ids) => {
                plan.matched += 1;
                if ids.iter().any(|id| has_work.contains(id)) {
                    plan.conflicts += 1;
                }
                wanted_collections.extend(photo.collections.iter().map(|s| s.as_str()));
            }
            None => plan.unmatched += 1,
        }
    }

    let existing: HashSet<String> = cat
        .list_collections()?
        .into_iter()
        .map(|c| c.name)
        .collect();
    let mut new: Vec<String> = wanted_collections
        .into_iter()
        .filter(|n| !existing.contains(*n))
        .map(|n| n.to_string())
        .collect();
    new.sort();
    plan.new_collections = new;
    Ok(plan)
}

/// Write a bundle's work into this catalog. Photos the bundle doesn't
/// mention are never touched, and originals on disk are never read.
pub fn apply(cat: &Catalog, bundle: &Bundle, opts: ApplyOptions) -> Result<ApplyOutcome> {
    let images = cat.all_images()?;
    let edited = cat.edited_image_ids()?;
    let index = index_by_key(&images);
    let has_work = local_work(&images, &edited);

    // Collection name -> id, creating as needed (lazily, so a run that
    // matches nothing doesn't litter the sidebar with empty collections).
    let mut collection_ids: HashMap<String, i64> = HashMap::new();
    let mut members: HashSet<(i64, i64)> = HashSet::new();
    for col in cat.list_collections()? {
        for image_id in cat.collection_image_ids(col.id)? {
            members.insert((col.id, image_id));
        }
        collection_ids.insert(col.name, col.id);
    }

    let mut out = ApplyOutcome::default();
    for photo in &bundle.photos {
        let Some(ids) = index.get(&match_key(&photo.file_name, photo.file_size)) else {
            out.unmatched += 1;
            continue;
        };
        if opts.on_conflict == OnConflict::Skip
            && ids.iter().any(|id| has_work.contains(id))
        {
            out.skipped_conflicts += 1;
            continue;
        }

        if opts.flags {
            let flag = Flag::from_i64(photo.flag);
            if let Err(e) = cat.set_flag(ids, flag) {
                out.errors.push(format!("{}: flag: {e:#}", photo.file_name));
            } else {
                out.flags_applied += ids.len();
            }
        }

        if opts.edits {
            if let Some(edit) = &photo.edit {
                // Round-trip through EditParams so an unreadable or
                // future-shaped blob lands as defaults for the fields we
                // understand rather than poisoning the row.
                let json = match &edit.params {
                    serde_json::Value::String(s) => s.clone(),
                    v => v.to_string(),
                };
                let params = crate::engine::params::EditParams::from_json(&json);
                if let Err(e) =
                    cat.save_edit(ids[0], edit.params_version, &params.to_json())
                {
                    out.errors.push(format!("{}: edit: {e:#}", photo.file_name));
                } else {
                    out.edits_applied += 1;
                    // Duplicates of the same file get the same edit.
                    for id in &ids[1..] {
                        let _ = cat.save_edit(*id, edit.params_version, &params.to_json());
                        out.edits_applied += 1;
                    }
                }
            }
        }

        if opts.collections {
            for name in &photo.collections {
                let id = match collection_ids.get(name) {
                    Some(id) => *id,
                    None => match cat.create_collection(name) {
                        Ok(id) => {
                            out.collections_created += 1;
                            collection_ids.insert(name.clone(), id);
                            id
                        }
                        Err(e) => {
                            out.errors.push(format!("collection {name}: {e:#}"));
                            continue;
                        }
                    },
                };
                let fresh: Vec<i64> = ids
                    .iter()
                    .copied()
                    .filter(|img| members.insert((id, *img)))
                    .collect();
                if fresh.is_empty() {
                    continue;
                }
                if let Err(e) = cat.add_to_collection(id, &fresh) {
                    out.errors.push(format!("collection {name}: {e:#}"));
                } else {
                    out.memberships_added += fresh.len();
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{FileKind, ImageRecord};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A catalog in its own temp dir; `Catalog`'s in-memory helper is private
    /// to that module, and a file-backed one exercises the same code.
    fn temp_catalog() -> (Catalog, PathBuf) {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("ed-meta-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cat = Catalog::open_at(&dir.join("catalog.db")).unwrap();
        (cat, dir)
    }

    fn insert(cat: &Catalog, path: &str, size: u64) -> i64 {
        let name = Path::new(path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        cat.insert_image(&ImageRecord {
            id: 0,
            abs_path: PathBuf::from(path),
            file_name: name,
            kind: FileKind::Raw,
            capture_ts: Some(1_775_364_006),
            imported_ts: 1_775_364_100,
            width: 6264,
            height: 4180,
            orientation: 1,
            camera_make: "Canon".into(),
            camera_model: "EOS RP".into(),
            file_size: size,
            flag: Flag::Unflagged,
        })
        .unwrap()
    }

    fn params_with_exposure(ev: f32) -> String {
        crate::engine::params::EditParams {
            exposure: ev,
            ..Default::default()
        }
        .to_json()
    }

    #[test]
    fn bundle_carries_work_across_catalogs_with_different_paths() {
        let (src, src_dir) = temp_catalog();
        let a = insert(&src, "/machine-a/2026/07-18/IMG_0042.CR3", 2400);
        let b = insert(&src, "/machine-a/2026/07-18/IMG_0043.CR3", 2500);
        insert(&src, "/machine-a/2026/07-18/IMG_0044.CR3", 2600); // no work
        src.set_flag(&[a], Flag::Pick).unwrap();
        src.set_flag(&[b], Flag::Reject).unwrap();
        src.save_edit(a, 1, &params_with_exposure(1.5)).unwrap();
        let col = src.create_collection("Trip").unwrap();
        src.add_to_collection(col, &[a]).unwrap();

        let bundle = export(&src, "/machine-a").unwrap();
        // Only the two photos carrying work travel; the third is left out.
        assert_eq!(bundle.photos.len(), 2);
        assert_eq!(bundle.source_photo_count, 3);

        // Same files, imported to a different root on the other machine.
        let (dst, dst_dir) = temp_catalog();
        let a2 = insert(&dst, "/machine-b/photos/2026/07-18/IMG_0042.CR3", 2400);
        let b2 = insert(&dst, "/machine-b/photos/2026/07-18/IMG_0043.CR3", 2500);

        let plan = plan(&dst, &bundle).unwrap();
        assert_eq!(plan.matched, 2);
        assert_eq!(plan.unmatched, 0);
        assert_eq!(plan.conflicts, 0);
        assert_eq!(plan.new_collections, vec!["Trip".to_string()]);

        let out = apply(&dst, &bundle, ApplyOptions::default()).unwrap();
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(out.edits_applied, 1);
        assert_eq!(out.collections_created, 1);

        let images = dst.all_images().unwrap();
        let flag_of = |id: i64| images.iter().find(|r| r.id == id).unwrap().flag;
        assert_eq!(flag_of(a2), Flag::Pick);
        assert_eq!(flag_of(b2), Flag::Reject);
        let (ver, json) = dst.load_edit(a2).unwrap().unwrap();
        assert_eq!(ver, 1);
        assert_eq!(
            crate::engine::params::EditParams::from_json(&json).exposure,
            1.5
        );
        let cols = dst.list_collections().unwrap();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, "Trip");
        assert!(dst.collection_image_ids(cols[0].id).unwrap().contains(&a2));

        std::fs::remove_dir_all(src_dir).ok();
        std::fs::remove_dir_all(dst_dir).ok();
    }

    #[test]
    fn file_round_trip_and_format_check() {
        let (src, src_dir) = temp_catalog();
        let a = insert(&src, "/a/IMG_1.CR3", 10);
        src.set_flag(&[a], Flag::Pick).unwrap();
        let bundle = export(&src, "/a").unwrap();

        let path = src_dir.join("work.edmeta");
        write_file(&path, &bundle).unwrap();
        let back = read_file(&path).unwrap();
        assert_eq!(back.photos.len(), 1);
        assert_eq!(back.photos[0].file_name, "IMG_1.CR3");
        assert_eq!(back.photos[0].flag, 1);

        // Anything else is rejected rather than half-applied.
        let junk = src_dir.join("junk.edmeta");
        std::fs::write(&junk, r#"{"hello":"world"}"#).unwrap();
        assert!(read_file(&junk).is_err());
        let future = src_dir.join("future.edmeta");
        std::fs::write(
            &future,
            format!(
                r#"{{"format":"{FORMAT}","format_version":99,"exported_ts":0,"photos":[]}}"#
            ),
        )
        .unwrap();
        assert!(read_file(&future).is_err());

        std::fs::remove_dir_all(src_dir).ok();
    }

    #[test]
    fn existing_work_is_skipped_or_overwritten_on_request() {
        let (src, src_dir) = temp_catalog();
        let a = insert(&src, "/a/IMG_1.CR3", 10);
        let b = insert(&src, "/a/IMG_2.CR3", 20);
        src.save_edit(a, 1, &params_with_exposure(2.0)).unwrap();
        src.set_flag(&[b], Flag::Pick).unwrap();
        let bundle = export(&src, "/a").unwrap();

        let (dst, dst_dir) = temp_catalog();
        let a2 = insert(&dst, "/b/IMG_1.CR3", 10);
        let b2 = insert(&dst, "/b/IMG_2.CR3", 20);
        // Local work on the first photo only.
        dst.save_edit(a2, 1, &params_with_exposure(-1.0)).unwrap();

        let plan = plan(&dst, &bundle).unwrap();
        assert_eq!(plan.matched, 2);
        assert_eq!(plan.conflicts, 1);

        // Skip leaves the local edit alone but still applies the clean photo.
        let out = apply(&dst, &bundle, ApplyOptions::default()).unwrap();
        assert_eq!(out.skipped_conflicts, 1);
        let (_, json) = dst.load_edit(a2).unwrap().unwrap();
        assert_eq!(
            crate::engine::params::EditParams::from_json(&json).exposure,
            -1.0
        );
        let images = dst.all_images().unwrap();
        assert_eq!(images.iter().find(|r| r.id == b2).unwrap().flag, Flag::Pick);

        // Overwrite lets the incoming file win.
        let out = apply(
            &dst,
            &bundle,
            ApplyOptions {
                on_conflict: OnConflict::Overwrite,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(out.skipped_conflicts, 0);
        let (_, json) = dst.load_edit(a2).unwrap().unwrap();
        assert_eq!(
            crate::engine::params::EditParams::from_json(&json).exposure,
            2.0
        );

        std::fs::remove_dir_all(src_dir).ok();
        std::fs::remove_dir_all(dst_dir).ok();
    }

    #[test]
    fn photos_not_in_the_library_are_reported_not_invented() {
        let (src, src_dir) = temp_catalog();
        let a = insert(&src, "/a/IMG_1.CR3", 10);
        src.set_flag(&[a], Flag::Pick).unwrap();
        let bundle = export(&src, "/a").unwrap();

        let (dst, dst_dir) = temp_catalog();
        // Same name, different size: a different file, so not a match.
        insert(&dst, "/b/IMG_1.CR3", 999);

        let plan = plan(&dst, &bundle).unwrap();
        assert_eq!(plan.matched, 0);
        assert_eq!(plan.unmatched, 1);
        let out = apply(&dst, &bundle, ApplyOptions::default()).unwrap();
        assert_eq!(out.unmatched, 1);
        assert_eq!(out.flags_applied, 0);
        assert_eq!(dst.all_images().unwrap()[0].flag, Flag::Unflagged);

        std::fs::remove_dir_all(src_dir).ok();
        std::fs::remove_dir_all(dst_dir).ok();
    }

    #[test]
    fn a_name_that_changed_case_in_transit_still_matches() {
        let (src, src_dir) = temp_catalog();
        let a = insert(&src, "/a/IMG_1.CR3", 10);
        src.set_flag(&[a], Flag::Reject).unwrap();
        let bundle = export(&src, "/a").unwrap();

        let (dst, dst_dir) = temp_catalog();
        let a2 = insert(&dst, "/b/img_1.cr3", 10);
        let out = apply(&dst, &bundle, ApplyOptions::default()).unwrap();
        assert_eq!(out.unmatched, 0);
        assert_eq!(dst.all_images().unwrap()[0].flag, Flag::Reject);
        assert_eq!(a2, dst.all_images().unwrap()[0].id);

        std::fs::remove_dir_all(src_dir).ok();
        std::fs::remove_dir_all(dst_dir).ok();
    }
}
