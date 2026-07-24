//! The central catalog: a SQLite database in the XDG data dir holding every
//! imported image and its edit parameters. Originals on disk are never
//! modified; all edits live here.

pub mod models;
pub mod schema;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub use models::{FileKind, Flag, ImageRecord};

/// A named grouping of images.
#[derive(Debug, Clone)]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub image_count: usize,
}

/// Directory holding catalog.db and the thumbnail cache. Defaults to the
/// XDG data dir; a `catalog_dir` entry in the app config redirects it (set
/// via Settings → Move catalog). Read directly here so background workers
/// that open their own catalog connection resolve the same location.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = configured_catalog_dir() {
        return dir;
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("effective-darkroom")
}

fn configured_catalog_dir() -> Option<PathBuf> {
    let cfg = dirs::config_dir()?.join("effective-darkroom/config.json");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(cfg).ok()?).ok()?;
    json.get("catalog_dir")?.as_str().map(PathBuf::from)
}

pub fn thumbs_dir() -> PathBuf {
    data_dir().join("thumbs")
}

pub struct Catalog {
    conn: Connection,
}

impl Catalog {
    /// Open (creating if needed) the catalog at the default location.
    pub fn open_default() -> Result<Self> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir).context("create data dir")?;
        std::fs::create_dir_all(thumbs_dir()).context("create thumbs dir")?;
        Self::open_at(&dir.join("catalog.db"))
    }

    /// Open a catalog at an explicit path (used by tests).
    pub fn open_at(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path).context("open catalog db")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut cat = Self { conn };
        cat.migrate()?;
        Ok(cat)
    }

    fn migrate(&mut self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version >= schema::SCHEMA_VERSION {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        for step in &schema::MIGRATIONS[version as usize..] {
            tx.execute_batch(step)?;
        }
        tx.pragma_update(None, "user_version", schema::SCHEMA_VERSION)?;
        tx.commit()?;
        Ok(())
    }

    pub fn insert_image(&self, rec: &ImageRecord) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO images (abs_path, file_name, kind, capture_ts, imported_ts,
                                 width, height, orientation, camera_make, camera_model, file_size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                rec.abs_path.to_string_lossy(),
                rec.file_name,
                rec.kind.as_str(),
                rec.capture_ts,
                rec.imported_ts,
                rec.width,
                rec.height,
                rec.orientation,
                rec.camera_make,
                rec.camera_model,
                rec.file_size,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn contains_path(&self, abs_path: &Path) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM images WHERE abs_path = ?1",
            params![abs_path.to_string_lossy()],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Any image with this original file name.
    pub fn contains_name(&self, file_name: &str) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM images WHERE file_name = ?1",
            params![file_name],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// A file already imported from elsewhere: same name and size.
    pub fn contains_similar(&self, file_name: &str, file_size: u64) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM images WHERE file_name = ?1 AND file_size = ?2",
            params![file_name, file_size],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn all_images(&self) -> Result<Vec<ImageRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, abs_path, file_name, kind, capture_ts, imported_ts,
                    width, height, orientation, camera_make, camera_model, file_size, flag
             FROM images ORDER BY capture_ts ASC, id ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ImageRecord {
                id: r.get(0)?,
                abs_path: PathBuf::from(r.get::<_, String>(1)?),
                file_name: r.get(2)?,
                kind: FileKind::parse(&r.get::<_, String>(3)?),
                capture_ts: r.get(4)?,
                imported_ts: r.get(5)?,
                width: r.get(6)?,
                height: r.get(7)?,
                orientation: r.get(8)?,
                camera_make: r.get(9)?,
                camera_model: r.get(10)?,
                file_size: r.get::<_, i64>(11)? as u64,
                flag: Flag::from_i64(r.get(12)?),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn set_flag(&self, ids: &[i64], flag: Flag) -> Result<()> {
        let mut stmt = self
            .conn
            .prepare("UPDATE images SET flag = ?1 WHERE id = ?2")?;
        for id in ids {
            stmt.execute(params![flag.as_i64(), id])?;
        }
        Ok(())
    }

    // --- collections ---

    pub fn create_collection(&self, name: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO collections (name, created_ts) VALUES (?1, unixepoch())",
            params![name.trim()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn rename_collection(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE collections SET name = ?1 WHERE id = ?2",
            params![name.trim(), id],
        )?;
        Ok(())
    }

    pub fn delete_collection(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM collections WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_collections(&self) -> Result<Vec<Collection>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.name,
                    (SELECT COUNT(*) FROM collection_images ci WHERE ci.collection_id = c.id)
             FROM collections c ORDER BY c.name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Collection {
                id: r.get(0)?,
                name: r.get(1)?,
                image_count: r.get::<_, i64>(2)? as usize,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn add_to_collection(&self, collection_id: i64, image_ids: &[i64]) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "INSERT OR IGNORE INTO collection_images (collection_id, image_id) VALUES (?1, ?2)",
        )?;
        for id in image_ids {
            stmt.execute(params![collection_id, id])?;
        }
        Ok(())
    }

    pub fn remove_from_collection(&self, collection_id: i64, image_ids: &[i64]) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "DELETE FROM collection_images WHERE collection_id = ?1 AND image_id = ?2",
        )?;
        for id in image_ids {
            stmt.execute(params![collection_id, id])?;
        }
        Ok(())
    }

    pub fn collection_image_ids(&self, collection_id: i64) -> Result<std::collections::HashSet<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT image_id FROM collection_images WHERE collection_id = ?1")?;
        let rows = stmt.query_map(params![collection_id], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn load_edit(&self, image_id: i64) -> Result<Option<(i64, String)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT params_version, params_json FROM edits WHERE image_id = ?1",
                params![image_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// Ids of images that have stored develop edits.
    pub fn edited_image_ids(&self) -> Result<std::collections::HashSet<i64>> {
        let mut stmt = self.conn.prepare("SELECT image_id FROM edits")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Flush the WAL into the main database file (before moving it).
    pub fn checkpoint(&self) -> Result<()> {
        self.conn
            .pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
        Ok(())
    }

    /// Re-point an image at a new absolute path (library move).
    pub fn update_abs_path(&self, id: i64, new_path: &Path) -> Result<()> {
        self.conn.execute(
            "UPDATE images SET abs_path = ?1 WHERE id = ?2",
            params![new_path.to_string_lossy(), id],
        )?;
        Ok(())
    }

    /// Remove images (and their edits, via cascade) from the catalog. Files
    /// on disk are untouched; the caller decides what happens to them.
    pub fn delete_images(&self, ids: &[i64]) -> Result<()> {
        let mut stmt = self.conn.prepare("DELETE FROM images WHERE id = ?1")?;
        for id in ids {
            stmt.execute(params![id])?;
        }
        Ok(())
    }

    pub fn save_edit(&self, image_id: i64, version: i64, json: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO edits (image_id, params_version, params_json, updated_ts)
             VALUES (?1, ?2, ?3, unixepoch())
             ON CONFLICT(image_id) DO UPDATE SET
                 params_version = excluded.params_version,
                 params_json = excluded.params_json,
                 updated_ts = excluded.updated_ts",
            params![image_id, version, json],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_catalog() -> Catalog {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        let mut cat = Catalog { conn };
        cat.migrate().unwrap();
        cat
    }

    fn sample(path: &str) -> ImageRecord {
        ImageRecord {
            id: 0,
            abs_path: PathBuf::from(path),
            file_name: Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            kind: FileKind::Jpeg,
            capture_ts: Some(1_775_364_006),
            imported_ts: 1_775_364_100,
            width: 6264,
            height: 4180,
            orientation: 1,
            camera_make: "Canon".into(),
            camera_model: "EOS RP".into(),
            file_size: 123,
            flag: Flag::Unflagged,
        }
    }

    #[test]
    fn insert_and_query() {
        let cat = mem_catalog();
        let id = cat.insert_image(&sample("/lib/2026/07-18/a.jpg")).unwrap();
        assert!(cat.contains_path(Path::new("/lib/2026/07-18/a.jpg")).unwrap());
        assert!(cat.contains_similar("a.jpg", 123).unwrap());
        assert!(!cat.contains_similar("a.jpg", 456).unwrap());
        let all = cat.all_images().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, id);
        assert_eq!(all[0].camera_model, "EOS RP");
    }

    #[test]
    fn duplicate_path_rejected() {
        let cat = mem_catalog();
        cat.insert_image(&sample("/lib/a.jpg")).unwrap();
        assert!(cat.insert_image(&sample("/lib/a.jpg")).is_err());
    }

    #[test]
    fn delete_cascades_edits() {
        let cat = mem_catalog();
        let id = cat.insert_image(&sample("/lib/a.jpg")).unwrap();
        cat.save_edit(id, 1, "{}").unwrap();
        cat.delete_images(&[id]).unwrap();
        assert!(cat.all_images().unwrap().is_empty());
        assert!(cat.load_edit(id).unwrap().is_none());
    }

    #[test]
    fn edits_roundtrip_and_upsert() {
        let cat = mem_catalog();
        let id = cat.insert_image(&sample("/lib/a.jpg")).unwrap();
        assert!(cat.load_edit(id).unwrap().is_none());
        cat.save_edit(id, 1, r#"{"exposure":0.5}"#).unwrap();
        cat.save_edit(id, 1, r#"{"exposure":1.0}"#).unwrap();
        let (ver, json) = cat.load_edit(id).unwrap().unwrap();
        assert_eq!(ver, 1);
        assert!(json.contains("1.0"));
    }

    #[test]
    fn flags_roundtrip() {
        let cat = mem_catalog();
        let id = cat.insert_image(&sample("/lib/a.jpg")).unwrap();
        cat.set_flag(&[id], Flag::Pick).unwrap();
        assert_eq!(cat.all_images().unwrap()[0].flag, Flag::Pick);
        cat.set_flag(&[id], Flag::Reject).unwrap();
        assert_eq!(cat.all_images().unwrap()[0].flag, Flag::Reject);
        cat.set_flag(&[id], Flag::Unflagged).unwrap();
        assert_eq!(cat.all_images().unwrap()[0].flag, Flag::Unflagged);
    }

    #[test]
    fn collections_crud() {
        let cat = mem_catalog();
        let a = cat.insert_image(&sample("/lib/a.jpg")).unwrap();
        let b = cat.insert_image(&sample("/lib/b.jpg")).unwrap();
        let cid = cat.create_collection("Trip").unwrap();
        cat.add_to_collection(cid, &[a, b]).unwrap();
        cat.add_to_collection(cid, &[a]).unwrap(); // idempotent
        let cols = cat.list_collections().unwrap();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].image_count, 2);
        assert!(cat.collection_image_ids(cid).unwrap().contains(&a));

        cat.remove_from_collection(cid, &[a]).unwrap();
        assert_eq!(cat.collection_image_ids(cid).unwrap().len(), 1);

        cat.rename_collection(cid, "  Road Trip ").unwrap();
        assert_eq!(cat.list_collections().unwrap()[0].name, "Road Trip");

        // Deleting an image cascades out of collections.
        cat.delete_images(&[b]).unwrap();
        assert!(cat.collection_image_ids(cid).unwrap().is_empty());

        cat.delete_collection(cid).unwrap();
        assert!(cat.list_collections().unwrap().is_empty());
        // Duplicate collection names are rejected.
        cat.create_collection("X").unwrap();
        assert!(cat.create_collection("X").is_err());
    }

    #[test]
    fn migration_idempotent() {
        let dir = std::env::temp_dir().join(format!("ed-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("catalog.db");
        drop(Catalog::open_at(&db).unwrap());
        drop(Catalog::open_at(&db).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
