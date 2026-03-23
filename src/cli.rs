use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "local_image_db",
    about = "Track and copy images between SD card and external drive"
)]
pub struct Cli {
    /// Override config file path
    #[arg(short, long, env = "LOCAL_IMAGE_DB_CONFIG")]
    pub config: Option<PathBuf>,

    /// Enable verbose output
    #[arg(short, long)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Scan a drive and index all images into the DB
    Scan {
        /// Path to scan (defaults to external_drive_path in config)
        path: Option<PathBuf>,

        /// Human-readable label for this drive
        #[arg(long)]
        drive_label: Option<String>,

        /// Recompute hashes even if already present
        #[arg(long)]
        force_rehash: bool,
    },

    /// Show which SD card images are not yet on the drive
    Check {
        /// SD card path (overrides SD_PATH in config)
        #[arg(long, env = "LOCAL_IMAGE_DB_SD_PATH")]
        sd: Option<PathBuf>,

        /// External drive path (overrides external_drive_path in config)
        #[arg(long, env = "LOCAL_IMAGE_DB_EXTERNAL_PATH")]
        drive: Option<PathBuf>,

        /// Use blake3 hashes for definitive deduplication (slower)
        #[arg(long)]
        hash: bool,

        /// Output format
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },

    /// Copy new images from SD card to external drive
    Copy {
        /// SD card path (overrides SD_PATH in config)
        #[arg(long, env = "LOCAL_IMAGE_DB_SD_PATH")]
        sd: Option<PathBuf>,

        /// External drive path (overrides external_drive_path in config)
        #[arg(long, env = "LOCAL_IMAGE_DB_EXTERNAL_PATH")]
        drive: Option<PathBuf>,

        /// Use blake3 hashes for definitive deduplication (slower)
        #[arg(long)]
        hash: bool,

        /// Show what would be copied without actually copying
        #[arg(long)]
        dry_run: bool,
    },

    /// Compute and store blake3 hashes for indexed images
    Hash {
        /// Only process images on this drive path
        #[arg(long, env = "LOCAL_IMAGE_DB_EXTERNAL_PATH")]
        drive: Option<PathBuf>,

        /// Only hash images that don't have a hash yet (default: true)
        #[arg(long, default_value = "true")]
        missing_only: bool,
    },

    /// Manage known drives
    Drives {
        #[command(subcommand)]
        action: DrivesAction,
    },

    /// Show resolved configuration
    Config,
}

#[derive(Subcommand, Debug)]
pub enum DrivesAction {
    /// List all known drives
    List,
    /// Add a drive to the DB
    Add {
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        label: String,
    },
    /// Remove a drive from the DB
    Remove {
        #[arg(long)]
        path: PathBuf,
    },
}

#[derive(ValueEnum, Clone, Debug, Default)]
pub enum OutputFormat {
    #[default]
    Table,
    Json,
    Paths,
}
