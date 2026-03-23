use anyhow::{Context, Result};
use chrono::Utc;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};

use crate::cli::OutputFormat;
use crate::config::Config;
use crate::db::Database;
use crate::hasher::hash_file;
use crate::models::{CopyCandidate, DuplicateStatus, ImageRecord};
use crate::scanner::{build_sd_records, destination_path};

pub struct RunOptions<'a> {
    pub sd_path: &'a Path,
    pub drive_root: &'a Path,
    pub drive_id: i64,
    pub use_hash: bool,
    pub format: &'a OutputFormat,
    pub dry_run: bool,
    pub verbose: bool,
}

/// Run the `check` subcommand (dry_run = true) or `copy` subcommand (dry_run = false).
pub fn run(db: &Database, opts: RunOptions<'_>, config: &Config) -> Result<()> {
    let RunOptions {
        sd_path,
        drive_root,
        drive_id,
        use_hash,
        format,
        dry_run,
        verbose,
    } = opts;
    // Scan SD card in-memory (not inserted into DB)
    let sd_images = build_sd_records(sd_path, &config.scan, verbose)?;

    if sd_images.is_empty() {
        println!("No image files found on SD card at {}.", sd_path.display());
        return Ok(());
    }

    // Classify each SD image against the DB
    let mut candidates: Vec<CopyCandidate> = Vec::new();
    for mut img in sd_images {
        let dest = destination_path(drive_root, &img, &config.copy.path_template);
        let status = classify(db, &mut img, use_hash)?;
        candidates.push(CopyCandidate {
            source: img,
            destination_path: dest,
            status,
        });
    }

    let to_copy: Vec<&CopyCandidate> = candidates
        .iter()
        .filter(|c| !matches!(c.status, DuplicateStatus::AlreadyExists { .. }))
        .collect();

    let already: Vec<&CopyCandidate> = candidates
        .iter()
        .filter(|c| matches!(c.status, DuplicateStatus::AlreadyExists { .. }))
        .collect();

    if dry_run {
        print_report(&to_copy, &already, format);
        return Ok(());
    }

    // Actual copy
    print_report(&to_copy, &already, format);

    if to_copy.is_empty() {
        println!("Nothing to copy.");
        return Ok(());
    }

    let pb = ProgressBar::new(to_copy.len() as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
        )
        .unwrap()
        .progress_chars("=>-"),
    );

    let mut copied = 0usize;
    let mut errors = 0usize;

    for candidate in &to_copy {
        if verbose {
            pb.set_message(candidate.source.filename.clone());
        }

        match copy_file(candidate, drive_id, db) {
            Ok(()) => copied += 1,
            Err(e) => {
                pb.println(format!("  ERROR: {}: {e}", candidate.source.filename));
                errors += 1;
            }
        }
        pb.inc(1);
    }

    pb.finish_and_clear();
    println!("Copied {copied} files. Errors: {errors}.");
    Ok(())
}

/// Two-pass duplicate classification.
fn classify(db: &Database, img: &mut ImageRecord, use_hash: bool) -> Result<DuplicateStatus> {
    let matches = db.find_by_filename(&img.filename)?;

    if matches.is_empty() {
        return Ok(DuplicateStatus::New);
    }

    // Pass 1: filename + size
    let size_matches: Vec<_> = matches
        .iter()
        .filter(|(_, size, _)| *size == img.file_size)
        .collect();

    if size_matches.is_empty() {
        return Ok(DuplicateStatus::SizeConflict);
    }

    if !use_hash {
        // Fast-path: same filename + size → treat as already present
        return Ok(DuplicateStatus::AlreadyExists {
            existing_path: size_matches[0].0.clone(),
        });
    }

    // Pass 2: compute hash of SD file and compare
    let sd_hash = hash_file(Path::new(&img.absolute_path))?;
    img.blake3_hash = Some(sd_hash.clone());

    // Check against hashes in the size-matching set
    for (path, _, stored_hash) in &size_matches {
        if let Some(h) = stored_hash
            && *h == sd_hash
        {
            return Ok(DuplicateStatus::AlreadyExists {
                existing_path: path.clone(),
            });
        }
    }

    // Also do a direct hash query in case hash was indexed from a different filename
    if let Some(existing_path) = db.find_by_hash(&sd_hash)? {
        return Ok(DuplicateStatus::AlreadyExists { existing_path });
    }

    Ok(DuplicateStatus::HashConflict)
}

fn copy_file(candidate: &CopyCandidate, drive_id: i64, db: &Database) -> Result<()> {
    let src = Path::new(&candidate.source.absolute_path);
    let dst = &candidate.destination_path;

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Creating directory {}", parent.display()))?;
    }

    // Handle filename collisions
    let dst = resolve_collision(dst);

    std::fs::copy(src, &dst)
        .with_context(|| format!("Copying {} -> {}", src.display(), dst.display()))?;

    // Verify size
    let copied_size = std::fs::metadata(&dst)?.len();
    if copied_size != candidate.source.file_size {
        std::fs::remove_file(&dst).ok();
        anyhow::bail!(
            "Size mismatch after copy: expected {}, got {}",
            candidate.source.file_size,
            copied_size
        );
    }

    // Insert the new record into DB
    let new_record = ImageRecord {
        id: None,
        drive_id,
        filename: dst
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        relative_path: dst.to_string_lossy().to_string(), // simplified; could strip drive root
        absolute_path: dst.to_string_lossy().to_string(),
        file_size: copied_size,
        capture_date: candidate.source.capture_date,
        file_mtime: Utc::now(),
        blake3_hash: candidate.source.blake3_hash.clone(),
    };
    db.upsert_image(&new_record)?;
    Ok(())
}

/// If the destination file already exists, append _1, _2, etc.
fn resolve_collision(dst: &Path) -> PathBuf {
    if !dst.exists() {
        return dst.to_path_buf();
    }
    let stem = dst
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let ext = dst
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = dst.parent().unwrap_or(Path::new("."));
    let mut i = 1u32;
    loop {
        let candidate = parent.join(format!("{stem}_{i}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
        i += 1;
    }
}

fn print_report(to_copy: &[&CopyCandidate], already: &[&CopyCandidate], format: &OutputFormat) {
    match format {
        OutputFormat::Table => print_table(to_copy, already),
        OutputFormat::Json => print_json(to_copy, already),
        OutputFormat::Paths => print_paths(to_copy),
    }
}

fn print_table(to_copy: &[&CopyCandidate], already: &[&CopyCandidate]) {
    println!("\n{} files to copy:", to_copy.len());
    for c in to_copy {
        let reason = match &c.status {
            DuplicateStatus::New => "new",
            DuplicateStatus::SizeConflict => "size-conflict",
            DuplicateStatus::HashConflict => "hash-conflict",
            DuplicateStatus::AlreadyExists { .. } => unreachable!(),
        };
        println!(
            "  [{reason:14}] {} -> {}",
            c.source.filename,
            c.destination_path.display()
        );
    }
    if !already.is_empty() {
        println!("\n{} files already on drive (skipped).", already.len());
    }
}

fn print_json(to_copy: &[&CopyCandidate], already: &[&CopyCandidate]) {
    // Simple hand-rolled JSON to avoid adding serde_json dependency here
    println!("{{");
    println!("  \"to_copy\": [");
    for (i, c) in to_copy.iter().enumerate() {
        let comma = if i + 1 < to_copy.len() { "," } else { "" };
        println!(
            "    {{\"filename\": \"{}\", \"destination\": \"{}\", \"status\": \"{:?}\"}}{comma}",
            c.source.filename,
            c.destination_path.display(),
            c.status
        );
    }
    println!("  ],");
    println!("  \"already_exists\": {}", already.len());
    println!("}}");
}

fn print_paths(to_copy: &[&CopyCandidate]) {
    for c in to_copy {
        println!("{}", c.source.absolute_path);
    }
}
