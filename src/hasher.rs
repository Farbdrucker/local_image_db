use anyhow::Result;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use crate::progress::{ProgressMsg, ProgressTx};

const CHUNK_SIZE: usize = 65_536; // 64 KB — cache-friendly for large RAW files

/// Compute the blake3 hash of a file, reading in chunks.
/// Returns the hash as a lowercase hex string.
pub fn hash_file(path: &Path) -> Result<String> {
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(CHUNK_SIZE, file);
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; CHUNK_SIZE];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Run the `hash` subcommand: compute and store blake3 hashes for images
/// that are missing them (or all images if `missing_only` is false).
///
/// When `progress` is `Some`, sends [`ProgressMsg`] instead of drawing
/// indicatif bars (TUI path). When `None`, uses indicatif directly (CLI path).
#[allow(dead_code)]
pub fn run_hash_command(
    db: &crate::db::Database,
    drive_path: Option<std::path::PathBuf>,
    _missing_only: bool,
    verbose: bool,
    progress: Option<ProgressTx>,
) -> Result<()> {
    // Resolve drive_id if a drive path was provided
    let drive_id = if let Some(path) = &drive_path {
        let path_str = path.to_string_lossy();
        match db.find_drive_by_path(&path_str)? {
            Some(d) => Some(d.id),
            None => {
                let msg = format!("No drive found at {} — run `scan` first.", path.display());
                if let Some(tx) = &progress {
                    let _ = tx.send(ProgressMsg::Failed { error: msg });
                } else {
                    eprintln!("{msg}");
                }
                return Ok(());
            }
        }
    } else {
        None
    };

    // missing_only=false currently falls through to same list (simplification)
    let targets = db.images_missing_hash(drive_id)?;

    if targets.is_empty() {
        if progress.is_none() {
            println!("No images need hashing.");
        } else if let Some(tx) = &progress {
            let _ = tx.send(ProgressMsg::Finished {
                processed: 0,
                errors: 0,
            });
        }
        return Ok(());
    }

    let total = targets.len() as u64;

    if let Some(tx) = &progress {
        let _ = tx.send(ProgressMsg::Started {
            total,
            label: "hash".to_string(),
        });
    }

    let mut hashed = 0u64;
    let mut errors = 0u64;

    for (current_idx, (_id, abs_path)) in targets.iter().enumerate() {
        let path = Path::new(abs_path);

        if let Some(tx) = &progress {
            let detail = path.file_name().map(|n| n.to_string_lossy().to_string());
            let _ = tx.send(ProgressMsg::Tick {
                current: current_idx as u64 + 1,
                total,
                detail,
            });
        }

        match hash_file(path) {
            Ok(hash) => {
                db.update_hash(abs_path, &hash)?;
                hashed += 1;
            }
            Err(e) => {
                if let Some(tx) = &progress {
                    let _ = tx.send(ProgressMsg::Warning {
                        message: format!("{abs_path}: {e}"),
                    });
                } else if verbose {
                    eprintln!("  WARN: {abs_path}: {e}");
                }
                errors += 1;
            }
        }
    }

    if let Some(tx) = progress {
        let _ = tx.send(ProgressMsg::Finished {
            processed: hashed,
            errors,
        });
    } else {
        println!("Hashed {hashed} files. Errors: {errors}.");
    }
    Ok(())
}

/// CLI-path version that shows an indicatif progress bar.
/// Called from the CLI hash subcommand to preserve the original UX.
pub fn run_hash_command_cli(
    db: &crate::db::Database,
    drive_path: Option<std::path::PathBuf>,
    missing_only: bool,
    verbose: bool,
) -> Result<()> {
    use indicatif::{ProgressBar, ProgressStyle};

    let drive_id = if let Some(path) = &drive_path {
        let path_str = path.to_string_lossy();
        match db.find_drive_by_path(&path_str)? {
            Some(d) => Some(d.id),
            None => {
                eprintln!("No drive found at {} — run `scan` first.", path.display());
                return Ok(());
            }
        }
    } else {
        None
    };

    let _ = missing_only; // currently always hashes missing-only
    let targets = db.images_missing_hash(drive_id)?;

    if targets.is_empty() {
        println!("No images need hashing.");
        return Ok(());
    }

    let pb = ProgressBar::new(targets.len() as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
        )
        .unwrap()
        .progress_chars("=>-"),
    );

    let mut hashed = 0usize;
    let mut errors = 0usize;

    for (_id, abs_path) in &targets {
        let path = Path::new(abs_path);
        if verbose {
            pb.set_message(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            );
        }
        match hash_file(path) {
            Ok(hash) => {
                db.update_hash(abs_path, &hash)?;
                hashed += 1;
            }
            Err(e) => {
                if verbose {
                    pb.println(format!("  WARN: {abs_path}: {e}"));
                }
                errors += 1;
            }
        }
        pb.inc(1);
    }

    pb.finish_and_clear();
    println!("Hashed {hashed} files. Errors: {errors}.");
    Ok(())
}
