mod cli;
mod config;
mod copy;
mod db;
mod hasher;
mod models;
mod scanner;

use anyhow::{Context, Result};
use clap::Parser;

use cli::{Cli, Commands, DrivesAction};
use config::Config;
use db::Database;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref())?;

    // Configure rayon thread pool if requested
    if config.scan.threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(config.scan.threads)
            .build_global()
            .ok(); // ignore error if pool already initialised
    }

    match cli.command {
        Commands::Scan {
            path,
            drive_label,
            force_rehash,
        } => {
            let scan_path = path
                .or_else(|| config.external_drive_path.clone())
                .context(
                    "No drive path provided. Pass a path or set external_drive_path in config.",
                )?;

            let db = open_db(&config)?;
            let label = drive_label.unwrap_or_else(|| {
                scan_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });
            let drive_id = db.upsert_drive(&label, &scan_path.to_string_lossy())?;
            scanner::run_scan(
                &db,
                &scan_path,
                drive_id,
                &config.scan,
                force_rehash,
                cli.verbose,
            )?;
        }

        Commands::Check {
            sd,
            drive,
            hash,
            format,
        } => {
            let sd_path = config
                .effective_sd_path(sd)
                .context("No SD path provided. Pass --sd or set sd_path in config.")?;
            let drive_path = config.effective_drive_path(drive).context(
                "No drive path provided. Pass --drive or set external_drive_path in config.",
            )?;

            let db = open_db(&config)?;
            let drive_id = ensure_drive_indexed(&db, &drive_path)?;

            copy::run(
                &db,
                copy::RunOptions {
                    sd_path: &sd_path,
                    drive_root: &drive_path,
                    drive_id,
                    use_hash: hash,
                    format: &format,
                    dry_run: true,
                    verbose: cli.verbose,
                },
                &config,
            )?;
        }

        Commands::Copy {
            sd,
            drive,
            hash,
            dry_run,
        } => {
            let sd_path = config
                .effective_sd_path(sd)
                .context("No SD path provided. Pass --sd or set sd_path in config.")?;
            let drive_path = config.effective_drive_path(drive).context(
                "No drive path provided. Pass --drive or set external_drive_path in config.",
            )?;

            let db = open_db(&config)?;
            let drive_id = ensure_drive_indexed(&db, &drive_path)?;

            copy::run(
                &db,
                copy::RunOptions {
                    sd_path: &sd_path,
                    drive_root: &drive_path,
                    drive_id,
                    use_hash: hash,
                    format: &cli::OutputFormat::Table,
                    dry_run,
                    verbose: cli.verbose,
                },
                &config,
            )?;
        }

        Commands::Hash {
            drive,
            missing_only,
        } => {
            let db = open_db(&config)?;
            hasher::run_hash_command(&db, drive, missing_only, cli.verbose)?;
        }

        Commands::Drives { action } => {
            let db = open_db(&config)?;
            match action {
                DrivesAction::List => {
                    let drives = db.list_drives()?;
                    if drives.is_empty() {
                        println!("No drives indexed. Run `scan` first.");
                    } else {
                        println!("{:<5} {:<30} {:<50} Last Scanned", "ID", "Label", "Path");
                        println!("{}", "-".repeat(100));
                        for d in drives {
                            println!(
                                "{:<5} {:<30} {:<50} {}",
                                d.id,
                                d.label,
                                d.root_path,
                                d.last_scanned_at
                                    .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
                                    .unwrap_or_else(|| "never".to_string())
                            );
                        }
                    }
                }
                DrivesAction::Add { path, label } => {
                    let db_path_str = path.to_string_lossy();
                    let id = db.upsert_drive(&label, &db_path_str)?;
                    println!(
                        "Drive '{label}' at {} registered with id {id}.",
                        path.display()
                    );
                }
                DrivesAction::Remove { path } => {
                    let path_str = path.to_string_lossy();
                    let n = db.remove_drive_by_path(&path_str)?;
                    if n == 0 {
                        println!("No drive found at {}.", path.display());
                    } else {
                        println!(
                            "Drive at {} removed (all indexed images deleted).",
                            path.display()
                        );
                    }
                }
            }
        }

        Commands::Config => {
            config.print_resolved(cli.config.as_deref());
        }
    }

    Ok(())
}

fn open_db(config: &Config) -> Result<Database> {
    Database::open(&config.db.path)
        .with_context(|| format!("Opening database at {}", config.db.path.display()))
}

/// Ensure a drive record exists for this path. If not, create one with a default label.
fn ensure_drive_indexed(db: &Database, drive_path: &std::path::Path) -> Result<i64> {
    let path_str = drive_path.to_string_lossy();
    let drive_id = if let Some(d) = db.find_drive_by_path(&path_str)? {
        d.id
    } else {
        let label = drive_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        db.upsert_drive(&label, &path_str)?
    };
    Ok(drive_id)
}
