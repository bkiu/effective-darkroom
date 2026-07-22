//! Catalog schema and migrations. Bump `SCHEMA_VERSION` and append a step to
//! `MIGRATIONS` for any change; steps run in order inside one transaction.

pub const SCHEMA_VERSION: i64 = 2;

pub const MIGRATIONS: &[&str] = &[
    // v1
    "
    CREATE TABLE images (
        id INTEGER PRIMARY KEY,
        abs_path TEXT NOT NULL UNIQUE,
        file_name TEXT NOT NULL,
        kind TEXT NOT NULL CHECK (kind IN ('raw','jpeg')),
        capture_ts INTEGER,
        imported_ts INTEGER NOT NULL,
        width INTEGER NOT NULL DEFAULT 0,
        height INTEGER NOT NULL DEFAULT 0,
        orientation INTEGER NOT NULL DEFAULT 0,
        camera_make TEXT NOT NULL DEFAULT '',
        camera_model TEXT NOT NULL DEFAULT '',
        file_size INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX idx_images_capture ON images(capture_ts);

    CREATE TABLE edits (
        image_id INTEGER PRIMARY KEY REFERENCES images(id) ON DELETE CASCADE,
        params_version INTEGER NOT NULL,
        params_json TEXT NOT NULL,
        updated_ts INTEGER NOT NULL
    );
    ",
    // v2: pick/reject flags and collections.
    "
    ALTER TABLE images ADD COLUMN flag INTEGER NOT NULL DEFAULT 0;

    CREATE TABLE collections (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL UNIQUE,
        created_ts INTEGER NOT NULL
    );
    CREATE TABLE collection_images (
        collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
        image_id INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
        PRIMARY KEY (collection_id, image_id)
    );
    ",
];
