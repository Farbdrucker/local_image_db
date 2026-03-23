use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub sd_path: Option<PathBuf>,
    pub external_drive_path: Option<PathBuf>,
    pub scan: ScanConfig,
    pub db: DbConfig,
    pub copy: CopyConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    pub extensions: Vec<String>,
    /// 0 = use all logical CPUs
    pub threads: usize,
    /// Minimum file size in bytes (skip tiny thumbnails)
    pub min_file_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DbConfig {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CopyConfig {
    pub path_template: String,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            extensions: vec![
                "jpg".into(),
                "jpeg".into(),
                "png".into(),
                "arw".into(),
                "srf".into(),
                "sr2".into(),
                "cr2".into(),
                "cr3".into(),
                "crw".into(),
                "raf".into(),
                "nef".into(),
                "nrw".into(),
            ],
            threads: 0,
            min_file_size: 102_400, // 100 KB
        }
    }
}

impl Default for DbConfig {
    fn default() -> Self {
        let path = if let Some(dirs) = ProjectDirs::from("", "", "local_image_db") {
            dirs.data_dir().join("images.db")
        } else {
            PathBuf::from("images.db")
        };
        Self { path }
    }
}

impl Default for CopyConfig {
    fn default() -> Self {
        Self {
            path_template: "{year}/{month}/{day}".into(),
        }
    }
}

impl Config {
    /// Load config from the given path, or the default XDG location.
    /// Missing file is treated as all-defaults (not an error).
    pub fn load(override_path: Option<&Path>) -> Result<Self> {
        let config_path = match override_path {
            Some(p) => p.to_path_buf(),
            None => default_config_path(),
        };

        if !config_path.exists() {
            return Ok(Config::default());
        }

        let contents = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Reading config from {}", config_path.display()))?;
        let config: Config = toml::from_str(&contents)
            .with_context(|| format!("Parsing config at {}", config_path.display()))?;
        Ok(config)
    }

    /// Print the resolved config and where it was loaded from.
    pub fn print_resolved(&self, path: Option<&Path>) {
        let path_str = path
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| default_config_path().display().to_string());
        println!("Config file: {path_str}");
        println!("sd_path:              {:?}", self.sd_path);
        println!("external_drive_path:  {:?}", self.external_drive_path);
        println!("db.path:              {}", self.db.path.display());
        println!("scan.extensions:      {:?}", self.scan.extensions);
        println!("scan.threads:         {} (0 = all CPUs)", self.scan.threads);
        println!("scan.min_file_size:   {} bytes", self.scan.min_file_size);
        println!("copy.path_template:   {}", self.copy.path_template);
    }

    /// Resolve the effective SD path: CLI flag > env > config
    pub fn effective_sd_path(&self, cli_override: Option<PathBuf>) -> Option<PathBuf> {
        cli_override.or_else(|| self.sd_path.clone())
    }

    /// Resolve the effective external drive path: CLI flag > env > config
    pub fn effective_drive_path(&self, cli_override: Option<PathBuf>) -> Option<PathBuf> {
        cli_override.or_else(|| self.external_drive_path.clone())
    }
}

fn default_config_path() -> PathBuf {
    ProjectDirs::from("", "", "local_image_db")
        .map(|d| d.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("config.toml"))
}
