# local_image_db

[![CI](https://github.com/YOUR_USERNAME/local_image_db/actions/workflows/ci.yml/badge.svg)](https://github.com/YOUR_USERNAME/local_image_db/actions/workflows/ci.yml)
![Rust](https://img.shields.io/badge/rust-2024-orange?logo=rust)
![SQLite](https://img.shields.io/badge/database-SQLite-blue?logo=sqlite)
![License: MIT](https://img.shields.io/badge/license-MIT-green)

Track which photos from your SD card have already been copied to an external drive — and copy the new ones into a `yyyy/mm/dd` folder structure.

## Install

```bash
cargo build --release
# binary at ./target/release/local_image_db
```

## Usage

```bash
# Index your external drive (run once, then after new imports)
local_image_db scan /Volumes/MyDrive --drive-label "MyDrive"

# See what's new on the SD card
local_image_db check --sd /Volumes/SD_CARD

# Copy new images to /Volumes/MyDrive/yyyy/mm/dd/
local_image_db copy --sd /Volumes/SD_CARD --drive /Volumes/MyDrive

# Use blake3 hashes for definitive duplicate detection
local_image_db check --sd /Volumes/SD_CARD --hash
```

## Configuration

Create `~/Library/Application Support/local_image_db/config.toml` (macOS) or `~/.config/local_image_db/config.toml` (Linux):

```toml
sd_path = "/Volumes/SD_CARD"
external_drive_path = "/Volumes/MyDrive"

[scan]
extensions = ["jpg", "jpeg", "png", "arw", "cr2", "cr3", "nef", "raf"]
min_file_size = 102400  # skip files < 100 KB
```

Or use env vars: `LOCAL_IMAGE_DB_SD_PATH`, `LOCAL_IMAGE_DB_EXTERNAL_PATH`.

## Supported formats

JPG, PNG · Sony ARW/SRF/SR2 · Canon CR2/CR3/CRW · Fuji RAF · Nikon NEF/NRW

## Commands

| Command | Description |
|---|---|
| `scan [PATH]` | Index all images on a drive into the DB |
| `check` | Show what would be copied (no writes) |
| `copy` | Copy new images; `--dry-run` to preview |
| `hash` | Backfill blake3 hashes for indexed images |
| `drives list\|add\|remove` | Manage known drives |
| `config` | Show resolved configuration |
