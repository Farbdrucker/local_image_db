use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;
use walkdir::WalkDir;

use crate::config::ScanConfig;
use crate::db::Database;
use crate::models::ImageRecord;
use crate::progress::{ProgressMsg, ProgressTx};

/// Scan `root` for images, index them into the DB under `drive_id`.
/// Returns the count of indexed images.
///
/// When `progress` is `Some`, sends [`ProgressMsg`] instead of drawing
/// indicatif bars (TUI path). When `None`, uses indicatif directly (CLI path).
pub fn run_scan(
    db: &Database,
    root: &Path,
    drive_id: i64,
    config: &ScanConfig,
    force_rehash: bool,
    verbose: bool,
    progress: Option<ProgressTx>,
) -> Result<usize> {
    let ext_set: HashSet<String> = config.extensions.iter().cloned().collect();

    if verbose && progress.is_none() {
        println!("Scanning {}...", root.display());
    }

    // Phase 1: collect matching entries (single-threaded readdir)
    let entries: Vec<PathBuf> = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| ext_set.contains(&x.to_lowercase()))
                .unwrap_or(false)
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    if entries.is_empty() {
        if progress.is_none() {
            println!("No image files found in {}.", root.display());
        } else if let Some(tx) = &progress {
            let _ = tx.send(ProgressMsg::Finished {
                processed: 0,
                errors: 0,
            });
        }
        return Ok(0);
    }

    let total = entries.len() as u64;

    // Phase 2: parallel metadata + EXIF extraction
    let min_size = config.min_file_size;
    let records: Vec<ImageRecord> = if let Some(ref tx) = progress {
        // TUI path: send ProgressMsg, no indicatif
        let _ = tx.send(ProgressMsg::Started {
            total,
            label: root.display().to_string(),
        });
        let counter = Arc::new(AtomicU64::new(0));
        entries
            .par_iter()
            .filter_map(|path| {
                let result = build_record(path, root, drive_id, min_size, force_rehash);
                let current = counter.fetch_add(1, Ordering::Relaxed) + 1;
                let detail = path.file_name().map(|n| n.to_string_lossy().to_string());
                let _ = tx.send(ProgressMsg::Tick {
                    current,
                    total,
                    detail,
                });
                match result {
                    Ok(Some(r)) => Some(r),
                    Ok(None) => None,
                    Err(e) => {
                        let _ = tx.send(ProgressMsg::Warning {
                            message: format!("{}: {e}", path.display()),
                        });
                        None
                    }
                }
            })
            .collect()
    } else {
        // CLI path: use indicatif as before
        let pb = ProgressBar::new(total);
        pb.set_style(
            ProgressStyle::with_template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
            )
            .unwrap()
            .progress_chars("=>-"),
        );
        let records = entries
            .par_iter()
            .progress_with(pb.clone())
            .filter_map(
                |path| match build_record(path, root, drive_id, min_size, force_rehash) {
                    Ok(Some(r)) => Some(r),
                    Ok(None) => None,
                    Err(e) => {
                        pb.println(format!("  WARN: {}: {e}", path.display()));
                        None
                    }
                },
            )
            .collect();
        pb.finish_and_clear();
        records
    };

    let count = records.len();

    // Phase 3: batch DB insert in a single transaction
    db.upsert_images_batch(&records)
        .context("Batch inserting images into DB")?;

    db.update_drive_scanned_at(drive_id)?;

    if let Some(tx) = progress {
        let _ = tx.send(ProgressMsg::Finished {
            processed: count as u64,
            errors: 0,
        });
    } else {
        println!("Indexed {count} images from {}.", root.display());
    }
    Ok(count)
}

fn build_record(
    path: &Path,
    root: &Path,
    drive_id: i64,
    min_size: u64,
    compute_hash: bool,
) -> Result<Option<ImageRecord>> {
    let meta = std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;

    let file_size = meta.len();
    if file_size < min_size {
        return Ok(None);
    }

    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let relative_path = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();

    let absolute_path = path.to_string_lossy().to_string();

    let file_mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let file_mtime: DateTime<Utc> = file_mtime.into();

    let capture_date = read_exif_date(path);

    let blake3_hash = if compute_hash {
        crate::hasher::hash_file(path).ok()
    } else {
        None
    };

    Ok(Some(ImageRecord {
        id: None,
        drive_id,
        filename,
        relative_path,
        absolute_path,
        file_size,
        capture_date,
        file_mtime,
        blake3_hash,
    }))
}

/// Attempt to read DateTimeOriginal (or DateTime) from EXIF. Returns None on any failure.
fn read_exif_date(path: &Path) -> Option<NaiveDateTime> {
    use std::fs::File;
    use std::io::BufReader;

    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let exif_reader = exif::Reader::new();
    let exif = exif_reader.read_from_container(&mut reader).ok()?;

    // Try DateTimeOriginal first, then DateTime
    for tag in [exif::Tag::DateTimeOriginal, exif::Tag::DateTime] {
        if let Some(field) = exif.get_field(tag, exif::In::PRIMARY) {
            let raw = field.display_value().to_string();
            // EXIF format: "YYYY:MM:DD HH:MM:SS"
            if let Ok(dt) = NaiveDateTime::parse_from_str(&raw, "%Y:%m:%d %H:%M:%S") {
                return Some(dt);
            }
        }
    }
    None
}

/// Scan the SD card for images without inserting into the DB.
/// Returns ImageRecord list with drive_id = 0 (placeholder, not persisted).
pub fn build_sd_records(
    sd_path: &Path,
    config: &ScanConfig,
    verbose: bool,
) -> Result<Vec<ImageRecord>> {
    let ext_set: HashSet<String> = config.extensions.iter().cloned().collect();

    if verbose {
        println!("Reading SD card at {}...", sd_path.display());
    }

    let entries: Vec<PathBuf> = WalkDir::new(sd_path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| ext_set.contains(&x.to_lowercase()))
                .unwrap_or(false)
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    let records: Vec<ImageRecord> = entries
        .par_iter()
        .filter_map(|path| {
            build_record(path, sd_path, 0, config.min_file_size, false)
                .ok()
                .flatten()
        })
        .collect();

    Ok(records)
}

/// Build the destination path for a file: `drive_root/yyyy/mm/dd/filename`
pub fn destination_path(drive_root: &Path, image: &ImageRecord, path_template: &str) -> PathBuf {
    let date = image.capture_date.map(|d| d.date()).unwrap_or_else(|| {
        Utc.from_utc_datetime(&image.file_mtime.naive_utc())
            .date_naive()
    });

    let subdir = path_template
        .replace("{year}", &format!("{:04}", date.format("%Y")))
        .replace("{month}", &format!("{:02}", date.format("%m")))
        .replace("{day}", &format!("{:02}", date.format("%d")));

    drive_root.join(subdir).join(&image.filename)
}
