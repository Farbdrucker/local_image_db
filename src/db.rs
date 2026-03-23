use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use std::path::Path;

use crate::models::{DriveRecord, ImageRecord};

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Creating DB directory {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("Opening database at {}", path.display()))?;
        let db = Self { conn };
        db.apply_pragmas()?;
        db.create_schema()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.apply_pragmas()?;
        db.create_schema()?;
        Ok(db)
    }

    fn apply_pragmas(&self) -> Result<()> {
        self.conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA cache_size = -32000;
             PRAGMA temp_store = MEMORY;",
        )?;
        Ok(())
    }

    fn create_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS drives (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                label           TEXT NOT NULL,
                root_path       TEXT NOT NULL UNIQUE,
                last_scanned_at TEXT,
                created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
            );

            CREATE TABLE IF NOT EXISTS images (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                drive_id        INTEGER NOT NULL REFERENCES drives(id) ON DELETE CASCADE,
                filename        TEXT NOT NULL,
                relative_path   TEXT NOT NULL,
                absolute_path   TEXT NOT NULL UNIQUE,
                file_size       INTEGER NOT NULL,
                capture_date    TEXT,
                file_mtime      TEXT NOT NULL,
                blake3_hash     TEXT,
                created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
            );

            CREATE INDEX IF NOT EXISTS idx_images_filename_size
                ON images(filename, file_size);

            CREATE INDEX IF NOT EXISTS idx_images_blake3
                ON images(blake3_hash)
                WHERE blake3_hash IS NOT NULL;

            CREATE INDEX IF NOT EXISTS idx_images_drive_id
                ON images(drive_id);

            CREATE INDEX IF NOT EXISTS idx_images_capture_date
                ON images(capture_date);",
        )?;
        Ok(())
    }

    // ── Drives ────────────────────────────────────────────────────────────────

    /// Insert or update a drive record, returning its id.
    pub fn upsert_drive(&self, label: &str, root_path: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO drives (label, root_path)
             VALUES (?1, ?2)
             ON CONFLICT(root_path) DO UPDATE SET label = excluded.label",
            params![label, root_path],
        )?;
        let id: i64 = self.conn.query_row(
            "SELECT id FROM drives WHERE root_path = ?1",
            params![root_path],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    pub fn update_drive_scanned_at(&self, drive_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE drives SET last_scanned_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')
             WHERE id = ?1",
            params![drive_id],
        )?;
        Ok(())
    }

    pub fn list_drives(&self) -> Result<Vec<DriveRecord>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, label, root_path, last_scanned_at FROM drives ORDER BY id")?;
        let records = stmt
            .query_map([], |row| {
                Ok(DriveRecord {
                    id: row.get(0)?,
                    label: row.get(1)?,
                    root_path: row.get(2)?,
                    last_scanned_at: row
                        .get::<_, Option<String>>(3)?
                        .and_then(|s| s.parse::<DateTime<Utc>>().ok()),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    pub fn find_drive_by_path(&self, root_path: &str) -> Result<Option<DriveRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, root_path, last_scanned_at FROM drives WHERE root_path = ?1",
        )?;
        let mut rows = stmt.query(params![root_path])?;
        if let Some(row) = rows.next()? {
            Ok(Some(DriveRecord {
                id: row.get(0)?,
                label: row.get(1)?,
                root_path: row.get(2)?,
                last_scanned_at: row
                    .get::<_, Option<String>>(3)?
                    .and_then(|s| s.parse::<DateTime<Utc>>().ok()),
            }))
        } else {
            Ok(None)
        }
    }

    pub fn remove_drive_by_path(&self, root_path: &str) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM drives WHERE root_path = ?1",
            params![root_path],
        )?;
        Ok(n)
    }

    // ── Images ────────────────────────────────────────────────────────────────

    /// Insert or replace an image record (by absolute_path).
    /// Returns the row id.
    pub fn upsert_image(&self, img: &ImageRecord) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO images
                (drive_id, filename, relative_path, absolute_path, file_size,
                 capture_date, file_mtime, blake3_hash)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(absolute_path) DO UPDATE SET
                drive_id      = excluded.drive_id,
                filename      = excluded.filename,
                relative_path = excluded.relative_path,
                file_size     = excluded.file_size,
                capture_date  = excluded.capture_date,
                file_mtime    = excluded.file_mtime,
                blake3_hash   = COALESCE(excluded.blake3_hash, images.blake3_hash)",
            params![
                img.drive_id,
                img.filename,
                img.relative_path,
                img.absolute_path,
                img.file_size as i64,
                img.capture_date
                    .map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string()),
                img.file_mtime.to_rfc3339(),
                img.blake3_hash,
            ],
        )?;
        let id: i64 = self.conn.last_insert_rowid();
        Ok(id)
    }

    /// Batch-insert a slice of ImageRecords in a single transaction.
    pub fn upsert_images_batch(&self, images: &[ImageRecord]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO images
                    (drive_id, filename, relative_path, absolute_path, file_size,
                     capture_date, file_mtime, blake3_hash)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(absolute_path) DO UPDATE SET
                    drive_id      = excluded.drive_id,
                    filename      = excluded.filename,
                    relative_path = excluded.relative_path,
                    file_size     = excluded.file_size,
                    capture_date  = excluded.capture_date,
                    file_mtime    = excluded.file_mtime,
                    blake3_hash   = COALESCE(excluded.blake3_hash, images.blake3_hash)",
            )?;
            for img in images {
                stmt.execute(params![
                    img.drive_id,
                    img.filename,
                    img.relative_path,
                    img.absolute_path,
                    img.file_size as i64,
                    img.capture_date
                        .map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string()),
                    img.file_mtime.to_rfc3339(),
                    img.blake3_hash,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Find all images with matching filename, returning (absolute_path, file_size, blake3_hash).
    pub fn find_by_filename(&self, filename: &str) -> Result<Vec<(String, u64, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT absolute_path, file_size, blake3_hash FROM images WHERE filename = ?1",
        )?;
        let rows = stmt
            .query_map(params![filename], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Check if any image matches the given blake3 hash.
    pub fn find_by_hash(&self, hash: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT absolute_path FROM images WHERE blake3_hash = ?1 LIMIT 1")?;
        let mut rows = stmt.query(params![hash])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// Update the blake3 hash for a specific image by absolute path.
    pub fn update_hash(&self, absolute_path: &str, hash: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE images SET blake3_hash = ?1 WHERE absolute_path = ?2",
            params![hash, absolute_path],
        )?;
        Ok(())
    }

    /// Return all images on a given drive that are missing a hash.
    pub fn images_missing_hash(&self, drive_id: Option<i64>) -> Result<Vec<(i64, String)>> {
        let (sql, drive_id_val) = if let Some(id) = drive_id {
            (
                "SELECT id, absolute_path FROM images
                 WHERE blake3_hash IS NULL AND drive_id = ?1",
                id,
            )
        } else {
            (
                "SELECT id, absolute_path FROM images
                 WHERE blake3_hash IS NULL AND 1 = ?1",
                1_i64,
            )
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt
            .query_map(params![drive_id_val], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Total number of indexed images. Available in all builds (used by TUI stats screen).
    pub fn image_count_pub(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM images", [], |r| r.get(0))?)
    }

    #[cfg(test)]
    pub fn image_count(&self) -> Result<i64> {
        self.image_count_pub()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDateTime, Utc};

    fn sample_image(drive_id: i64, filename: &str, path: &str, size: u64) -> ImageRecord {
        ImageRecord {
            id: None,
            drive_id,
            filename: filename.to_string(),
            relative_path: filename.to_string(),
            absolute_path: path.to_string(),
            file_size: size,
            capture_date: None,
            file_mtime: Utc::now(),
            blake3_hash: None,
        }
    }

    #[test]
    fn test_upsert_drive_and_find() {
        let db = Database::open_in_memory().unwrap();
        let id = db.upsert_drive("Test Drive", "/mnt/test").unwrap();
        assert!(id > 0);
        let drive = db.find_drive_by_path("/mnt/test").unwrap().unwrap();
        assert_eq!(drive.label, "Test Drive");
        assert_eq!(drive.id, id);
    }

    #[test]
    fn test_upsert_drive_updates_label() {
        let db = Database::open_in_memory().unwrap();
        let id1 = db.upsert_drive("Old Label", "/mnt/test").unwrap();
        let id2 = db.upsert_drive("New Label", "/mnt/test").unwrap();
        assert_eq!(id1, id2);
        let drive = db.find_drive_by_path("/mnt/test").unwrap().unwrap();
        assert_eq!(drive.label, "New Label");
    }

    #[test]
    fn test_upsert_and_find_image() {
        let db = Database::open_in_memory().unwrap();
        let drive_id = db.upsert_drive("Drive", "/mnt/d").unwrap();
        let img = sample_image(
            drive_id,
            "DSC_0001.NEF",
            "/mnt/d/2024/DSC_0001.NEF",
            15_000_000,
        );
        db.upsert_image(&img).unwrap();

        let results = db.find_by_filename("DSC_0001.NEF").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, 15_000_000);
    }

    #[test]
    fn test_batch_insert() {
        let db = Database::open_in_memory().unwrap();
        let drive_id = db.upsert_drive("Drive", "/mnt/d").unwrap();
        let images: Vec<_> = (0..100)
            .map(|i| {
                sample_image(
                    drive_id,
                    &format!("IMG_{i:04}.jpg"),
                    &format!("/mnt/d/IMG_{i:04}.jpg"),
                    1_000_000,
                )
            })
            .collect();
        db.upsert_images_batch(&images).unwrap();
        assert_eq!(db.image_count().unwrap(), 100);
    }

    #[test]
    fn test_hash_update_and_find() {
        let db = Database::open_in_memory().unwrap();
        let drive_id = db.upsert_drive("Drive", "/mnt/d").unwrap();
        let img = sample_image(drive_id, "photo.arw", "/mnt/d/photo.arw", 25_000_000);
        db.upsert_image(&img).unwrap();

        db.update_hash("/mnt/d/photo.arw", "abc123").unwrap();
        let found = db.find_by_hash("abc123").unwrap();
        assert_eq!(found, Some("/mnt/d/photo.arw".to_string()));
    }

    #[test]
    fn test_cascade_delete() {
        let db = Database::open_in_memory().unwrap();
        let drive_id = db.upsert_drive("Drive", "/mnt/d").unwrap();
        let img = sample_image(drive_id, "photo.jpg", "/mnt/d/photo.jpg", 5_000_000);
        db.upsert_image(&img).unwrap();
        assert_eq!(db.image_count().unwrap(), 1);

        db.remove_drive_by_path("/mnt/d").unwrap();
        assert_eq!(db.image_count().unwrap(), 0);
    }

    #[test]
    fn test_capture_date_roundtrip() {
        let db = Database::open_in_memory().unwrap();
        let drive_id = db.upsert_drive("Drive", "/mnt/d").unwrap();
        let capture =
            NaiveDateTime::parse_from_str("2024:07:15 14:30:00", "%Y:%m:%d %H:%M:%S").unwrap();
        let mut img = sample_image(drive_id, "photo.cr2", "/mnt/d/photo.cr2", 10_000_000);
        img.capture_date = Some(capture);
        db.upsert_image(&img).unwrap();

        // Verify via find_by_filename (we just check no error and row exists)
        let results = db.find_by_filename("photo.cr2").unwrap();
        assert_eq!(results.len(), 1);
    }
}
