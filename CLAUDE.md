# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`local_image_db` is a Rust CLI tool that tracks photos across an SD card and an external drive. It scans the drive and builds a SQLite DB of all images (with EXIF dates, sizes, blake3 hashes). When an SD card is mounted it can diff it against the DB to show — or actually copy — only the new images, placing them at `EXTERNAL_DRIVE_PATH/yyyy/mm/dd/`.

## Architecture

```
src/
  main.rs     — CLI dispatch (clap), opens DB, routes to each module
  cli.rs      — All clap structs: Cli, Commands, DrivesAction, OutputFormat
  config.rs   — Config (TOML, serde), resolved via `directories` crate (XDG/macOS)
  models.rs   — ImageRecord, DriveRecord, CopyCandidate, DuplicateStatus
  db.rs       — Database::open(), all SQL (rusqlite bundled, WAL mode)
  scanner.rs  — walkdir + rayon parallel EXIF scan, build_sd_records()
  hasher.rs   — blake3 chunked file hashing, run_hash_command()
  copy.rs     — Two-pass duplicate detection + file copy, RunOptions struct
```

**Key design decisions:**
- Duplicate detection is two-pass: filename+size (fast, always) → blake3 hash (only with `--hash` flag). Hashes are stored lazily in the DB.
- Scanner collects all `DirEntry` items first (single-threaded walkdir), then processes metadata + EXIF in parallel with rayon. DB inserts are batched in a single transaction.
- `copy::run` takes a `RunOptions` struct (avoids too-many-arguments). `dry_run=true` = `check` behaviour.
- Config priority: CLI flag > env var (`LOCAL_IMAGE_DB_SD_PATH`, `LOCAL_IMAGE_DB_EXTERNAL_PATH`) > `config.toml` > compiled default.
- DB stored at `~/Library/Application Support/local_image_db/images.db` (macOS) or `~/.local/share/local_image_db/images.db` (Linux).

## Commands

```bash
cargo build --release              # production binary
cargo check                        # fast compile check
cargo test                         # run all tests (DB tests use in-memory SQLite)
cargo test <test_name>             # single test
cargo clippy -- -D warnings        # lint (must be clean)
cargo fmt                          # format

# Usage examples
./target/release/local_image_db scan /Volumes/SanDisk4TB --drive-label "SanDisk4TB"
./target/release/local_image_db check --sd /Volumes/SD_CARD
./target/release/local_image_db check --sd /Volumes/SD_CARD --hash   # definitive dedup
./target/release/local_image_db copy  --sd /Volumes/SD_CARD --drive /Volumes/SanDisk4TB
./target/release/local_image_db copy  --sd /Volumes/SD_CARD --dry-run
./target/release/local_image_db drives list
./target/release/local_image_db hash --missing-only
./target/release/local_image_db config
```

## Supported image extensions

`jpg`, `jpeg`, `png`, `arw`, `srf`, `sr2` (Sony), `cr2`, `cr3`, `crw` (Canon), `raf` (Fuji), `nef`, `nrw` (Nikon). Configurable in `config.toml`.
