use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Raw,
    Jpeg,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::Raw => "raw",
            FileKind::Jpeg => "jpeg",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "raw" => FileKind::Raw,
            _ => FileKind::Jpeg,
        }
    }

    /// Classify by file extension; `None` for files we don't import.
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "cr3" | "raf" => Some(FileKind::Raw),
            "jpg" | "jpeg" => Some(FileKind::Jpeg),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImageRecord {
    pub id: i64,
    pub abs_path: PathBuf,
    pub file_name: String,
    pub kind: FileKind,
    /// Capture time as unix seconds; camera-local wall time.
    pub capture_ts: Option<i64>,
    pub imported_ts: i64,
    pub width: u32,
    pub height: u32,
    /// EXIF orientation value (1-8) for JPEGs, 0 for already-rotated sources.
    pub orientation: u32,
    pub camera_make: String,
    pub camera_model: String,
    pub file_size: u64,
}
