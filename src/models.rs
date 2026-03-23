use chrono::{DateTime, NaiveDateTime, Utc};

#[derive(Debug, Clone)]
pub struct DriveRecord {
    pub id: i64,
    pub label: String,
    pub root_path: String,
    pub last_scanned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ImageRecord {
    #[allow(dead_code)]
    pub id: Option<i64>,
    pub drive_id: i64,
    pub filename: String,
    pub relative_path: String,
    pub absolute_path: String,
    pub file_size: u64,
    pub capture_date: Option<NaiveDateTime>,
    pub file_mtime: DateTime<Utc>,
    pub blake3_hash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CopyCandidate {
    pub source: ImageRecord,
    pub destination_path: std::path::PathBuf,
    pub status: DuplicateStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DuplicateStatus {
    /// Not found in DB at all — copy
    New,
    /// Same filename, different size — treat as distinct file, copy
    SizeConflict,
    /// Same filename + size, different hash — treat as distinct file, copy
    HashConflict,
    /// Definitive match found — skip
    AlreadyExists { existing_path: String },
}
