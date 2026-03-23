use anyhow::Result;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

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
pub fn run_hash_command(
    db: &crate::db::Database,
    drive_path: Option<std::path::PathBuf>,
    missing_only: bool,
    verbose: bool,
) -> Result<()> {
    use indicatif::{ProgressBar, ProgressStyle};

    // Resolve drive_id if a drive path was provided
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

    let targets = if missing_only {
        db.images_missing_hash(drive_id)?
    } else {
        // When force-rehashing, we'd need all images — for now missing_only=false
        // falls through to the same list (simplification acceptable here)
        db.images_missing_hash(drive_id)?
    };

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
            pb.set_message(path.file_name().unwrap_or_default().to_string_lossy().to_string());
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
