mod app;
mod events;
mod ui;

use std::io;
use std::path::PathBuf;
use std::sync::mpsc;

use anyhow::Result;
use crossterm::{
    event::{KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::config::Config;
use crate::copy::{RunOptions, build_candidates, run_copy_phase};
use crate::db::Database;
use crate::models::CopyCandidate;
use crate::progress::ProgressMsg;

use app::{App, AppScreen, CheckCopyMode};
use events::{AppEvent, TaskKind, is_quit, spawn_input_thread};

pub fn run(db: Database, config: Config) -> Result<()> {
    // Install panic hook to restore terminal on crash
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stderr(), LeaveAlternateScreen);
        original_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, db, config);

    // Restore terminal regardless of result
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    _db: Database,
    config: Config,
) -> Result<()> {
    let (event_tx, event_rx) = mpsc::channel::<AppEvent>();

    spawn_input_thread(event_tx.clone());

    let mut app = App::new(event_tx.clone());

    // Pre-populate inputs from config
    if let Some(p) = &config.sd_path {
        app.check_copy.sd_input = p.to_string_lossy().to_string();
    }
    if let Some(p) = &config.external_drive_path {
        app.scan.path_input = p.to_string_lossy().to_string();
        app.check_copy.drive_input = p.to_string_lossy().to_string();
    }

    // Load initial stats
    spawn_load_stats(event_tx.clone(), config.db.path.clone());
    app.home.loading = true;

    loop {
        terminal.draw(|frame| ui::render(frame, &app))?;

        let event = event_rx.recv()?;

        match event {
            AppEvent::Tick => {}

            AppEvent::Key(key) => {
                // Consume status message on any key
                app.status_message = None;

                // Global quit
                if is_quit(&key) && !is_in_text_input(&app) {
                    app.should_quit = true;
                }

                // Global tab switching (1-4, F1-F4)
                if app.active_task.is_none() {
                    match key.code {
                        KeyCode::Char('1') | KeyCode::F(1) => {
                            app.screen = AppScreen::Home;
                            if !app.home.loading {
                                spawn_load_stats(event_tx.clone(), config.db.path.clone());
                                app.home.loading = true;
                            }
                        }
                        KeyCode::Char('2') | KeyCode::F(2) => {
                            app.screen = AppScreen::Scan;
                        }
                        KeyCode::Char('3') | KeyCode::F(3) => {
                            app.screen = AppScreen::CheckCopy;
                        }
                        KeyCode::Char('4') | KeyCode::F(4) => {
                            app.screen = AppScreen::Drives;
                            spawn_load_drives(event_tx.clone(), config.db.path.clone());
                            app.drives.loading = true;
                        }
                        _ => {}
                    }
                }

                handle_screen_key(&mut app, key, &config, &event_tx);
            }

            AppEvent::Progress(msg) => app.handle_progress(msg),

            AppEvent::CandidatesReady { to_copy, already } => {
                app.check_copy.to_copy = to_copy;
                app.check_copy.already = already;
                app.check_copy.loading = false;
                app.check_copy.selected_row = 0;
                app.check_copy.table_offset = 0;
            }

            AppEvent::DrivesLoaded(drives) => {
                app.drives.drives = drives;
                app.drives.loading = false;
            }

            AppEvent::StatsLoaded {
                image_count,
                drive_count,
                unhashed_count,
            } => {
                app.home.image_count = image_count;
                app.home.drive_count = drive_count;
                app.home.unhashed_count = unhashed_count;
                app.home.loading = false;
            }

            AppEvent::TaskDone { task, error } => {
                let should_reload_drives =
                    matches!(task, TaskKind::AddDrive | TaskKind::RemoveDrive);
                app.task_done(task, error);
                if should_reload_drives {
                    spawn_load_drives(event_tx.clone(), config.db.path.clone());
                    app.drives.loading = true;
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

/// Dispatch keyboard events to the active screen.
fn handle_screen_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    config: &Config,
    event_tx: &mpsc::Sender<AppEvent>,
) {
    match app.screen {
        AppScreen::Home => handle_home_key(app, key, event_tx, config),
        AppScreen::Scan => handle_scan_key(app, key, event_tx, config),
        AppScreen::CheckCopy => handle_check_copy_key(app, key, event_tx, config),
        AppScreen::Drives => handle_drives_key(app, key, event_tx, config),
    }
}

fn handle_home_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    event_tx: &mpsc::Sender<AppEvent>,
    config: &Config,
) {
    if key.code == KeyCode::Char('r') {
        spawn_load_stats(event_tx.clone(), config.db.path.clone());
        app.home.loading = true;
    }
}

fn handle_scan_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    event_tx: &mpsc::Sender<AppEvent>,
    config: &Config,
) {
    if app.scan.running {
        return;
    }

    match key.code {
        KeyCode::Tab => {
            app.scan.focused_field = (app.scan.focused_field + 1) % 2;
        }
        KeyCode::Char(' ') => {
            app.scan.force_rehash = !app.scan.force_rehash;
        }
        KeyCode::Enter => {
            if app.scan.path_input.is_empty() {
                app.status_message = Some("Path is required.".to_string());
            } else if app.active_task.is_none() {
                app.scan.log.clear();
                app.scan.result = None;
                app.scan.progress_current = 0;
                app.scan.progress_total = 0;
                app.scan.running = true;
                app.active_task = Some(TaskKind::Scan);
                spawn_scan(
                    event_tx.clone(),
                    config.db.path.clone(),
                    PathBuf::from(&app.scan.path_input),
                    if app.scan.label_input.is_empty() {
                        None
                    } else {
                        Some(app.scan.label_input.clone())
                    },
                    app.scan.force_rehash,
                    config.scan.clone(),
                );
            }
        }
        KeyCode::Backspace => {
            if app.scan.focused_field == 0 {
                backspace_input(&mut app.scan.path_input, &mut app.scan.input_cursor);
            } else {
                backspace_input(&mut app.scan.label_input, &mut app.scan.label_cursor);
            }
        }
        KeyCode::Char(c)
            if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
        {
            if app.scan.focused_field == 0 {
                insert_char(&mut app.scan.path_input, &mut app.scan.input_cursor, c);
            } else {
                insert_char(&mut app.scan.label_input, &mut app.scan.label_cursor, c);
            }
        }
        _ => {}
    }
}

fn handle_check_copy_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    event_tx: &mpsc::Sender<AppEvent>,
    config: &Config,
) {
    if app.check_copy.running {
        return;
    }

    // Navigate candidate table
    if !app.check_copy.to_copy.is_empty() {
        match key.code {
            KeyCode::Down => {
                let max = app.check_copy.to_copy.len().saturating_sub(1);
                if app.check_copy.selected_row < max {
                    app.check_copy.selected_row += 1;
                }
                return;
            }
            KeyCode::Up => {
                if app.check_copy.selected_row > 0 {
                    app.check_copy.selected_row -= 1;
                }
                return;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Tab => {
            app.check_copy.focused_field = (app.check_copy.focused_field + 1) % 2;
        }
        KeyCode::Char('h') => {
            app.check_copy.use_hash = !app.check_copy.use_hash;
        }
        KeyCode::Char('c') => {
            if app.active_task.is_none()
                && !app.check_copy.sd_input.is_empty()
                && !app.check_copy.drive_input.is_empty()
            {
                app.check_copy.mode = CheckCopyMode::Check;
                app.check_copy.loading = true;
                app.check_copy.log.clear();
                app.check_copy.result = None;
                app.active_task = Some(TaskKind::LoadCandidates);
                spawn_load_candidates(
                    event_tx.clone(),
                    config.db.path.clone(),
                    PathBuf::from(&app.check_copy.sd_input),
                    PathBuf::from(&app.check_copy.drive_input),
                    app.check_copy.use_hash,
                    config.clone(),
                );
            }
        }
        KeyCode::Char('x') => {
            if app.active_task.is_none() && !app.check_copy.to_copy.is_empty() {
                app.check_copy.mode = CheckCopyMode::Copy;
                app.check_copy.running = true;
                app.check_copy.result = None;
                app.active_task = Some(TaskKind::Copy);
                let drive_root = PathBuf::from(&app.check_copy.drive_input);
                spawn_copy_phase(
                    event_tx.clone(),
                    config.db.path.clone(),
                    app.check_copy.to_copy.clone(),
                    drive_root,
                );
            }
        }
        KeyCode::Backspace => {
            if app.check_copy.focused_field == 0 {
                backspace_input(&mut app.check_copy.sd_input, &mut app.check_copy.sd_cursor);
            } else {
                backspace_input(
                    &mut app.check_copy.drive_input,
                    &mut app.check_copy.drive_cursor,
                );
            }
        }
        KeyCode::Char(c)
            if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
        {
            if app.check_copy.focused_field == 0 {
                insert_char(
                    &mut app.check_copy.sd_input,
                    &mut app.check_copy.sd_cursor,
                    c,
                );
            } else {
                insert_char(
                    &mut app.check_copy.drive_input,
                    &mut app.check_copy.drive_cursor,
                    c,
                );
            }
        }
        _ => {}
    }
}

fn handle_drives_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    event_tx: &mpsc::Sender<AppEvent>,
    config: &Config,
) {
    if app.drives.add_open {
        match key.code {
            KeyCode::Esc => {
                app.drives.add_open = false;
                app.drives.add_path.clear();
                app.drives.add_label.clear();
            }
            KeyCode::Tab => {
                app.drives.add_focused = (app.drives.add_focused + 1) % 2;
            }
            KeyCode::Enter => {
                if app.drives.add_path.is_empty() {
                    app.status_message = Some("Path is required.".to_string());
                } else if app.active_task.is_none() {
                    let path = app.drives.add_path.clone();
                    let label = if app.drives.add_label.is_empty() {
                        PathBuf::from(&path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string()
                    } else {
                        app.drives.add_label.clone()
                    };
                    app.drives.add_open = false;
                    app.drives.add_path.clear();
                    app.drives.add_label.clear();
                    app.active_task = Some(TaskKind::AddDrive);
                    spawn_add_drive(event_tx.clone(), config.db.path.clone(), path, label);
                }
            }
            KeyCode::Backspace => {
                if app.drives.add_focused == 0 {
                    backspace_input(&mut app.drives.add_path, &mut app.drives.add_path_cursor);
                } else {
                    backspace_input(&mut app.drives.add_label, &mut app.drives.add_label_cursor);
                }
            }
            KeyCode::Char(c)
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                if app.drives.add_focused == 0 {
                    insert_char(&mut app.drives.add_path, &mut app.drives.add_path_cursor, c);
                } else {
                    insert_char(
                        &mut app.drives.add_label,
                        &mut app.drives.add_label_cursor,
                        c,
                    );
                }
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Down => {
            if app.drives.selected_row + 1 < app.drives.drives.len() {
                app.drives.selected_row += 1;
            }
        }
        KeyCode::Up => {
            if app.drives.selected_row > 0 {
                app.drives.selected_row -= 1;
            }
        }
        KeyCode::Char('a') => {
            app.drives.add_open = true;
            app.drives.add_focused = 0;
        }
        KeyCode::Char('d') | KeyCode::Delete => {
            if let Some(drive) = app.drives.drives.get(app.drives.selected_row)
                && app.active_task.is_none()
            {
                let path = drive.root_path.clone();
                app.active_task = Some(TaskKind::RemoveDrive);
                spawn_remove_drive(event_tx.clone(), config.db.path.clone(), path);
            }
        }
        KeyCode::Char('r') => {
            spawn_load_drives(event_tx.clone(), config.db.path.clone());
            app.drives.loading = true;
        }
        _ => {}
    }
}

// ── Background task spawners ──────────────────────────────────────────────────

fn spawn_load_stats(tx: mpsc::Sender<AppEvent>, db_path: PathBuf) {
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<(u64, u64, u64)> {
            let db = Database::open(&db_path)?;
            let drives = db.list_drives()?;
            let image_count = db.image_count_pub()?;
            let unhashed = db.images_missing_hash(None)?.len() as u64;
            Ok((image_count as u64, drives.len() as u64, unhashed))
        })();
        let event = match result {
            Ok((images, drives, unhashed)) => AppEvent::StatsLoaded {
                image_count: images,
                drive_count: drives,
                unhashed_count: unhashed,
            },
            Err(e) => AppEvent::TaskDone {
                task: TaskKind::LoadStats,
                error: Some(e.to_string()),
            },
        };
        let _ = tx.send(event);
    });
}

fn spawn_load_drives(tx: mpsc::Sender<AppEvent>, db_path: PathBuf) {
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<_> {
            let db = Database::open(&db_path)?;
            db.list_drives()
        })();
        let event = match result {
            Ok(drives) => AppEvent::DrivesLoaded(drives),
            Err(e) => AppEvent::TaskDone {
                task: TaskKind::LoadDrives,
                error: Some(e.to_string()),
            },
        };
        let _ = tx.send(event);
    });
}

fn spawn_scan(
    tx: mpsc::Sender<AppEvent>,
    db_path: PathBuf,
    path: PathBuf,
    label: Option<String>,
    force_rehash: bool,
    scan_config: crate::config::ScanConfig,
) {
    std::thread::spawn(move || {
        let (prog_tx, prog_rx) = mpsc::channel::<ProgressMsg>();
        let relay = tx.clone();
        std::thread::spawn(move || {
            for msg in prog_rx {
                let _ = relay.send(AppEvent::Progress(msg));
            }
        });

        let result = (|| -> anyhow::Result<()> {
            let db = Database::open(&db_path)?;
            let lbl = label.unwrap_or_else(|| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });
            let drive_id = db.upsert_drive(&lbl, &path.to_string_lossy())?;
            crate::scanner::run_scan(
                &db,
                &path,
                drive_id,
                &scan_config,
                force_rehash,
                false,
                Some(prog_tx),
            )?;
            Ok(())
        })();

        let _ = tx.send(AppEvent::TaskDone {
            task: TaskKind::Scan,
            error: result.err().map(|e| e.to_string()),
        });
    });
}

fn spawn_load_candidates(
    tx: mpsc::Sender<AppEvent>,
    db_path: PathBuf,
    sd_path: PathBuf,
    drive_path: PathBuf,
    use_hash: bool,
    config: Config,
) {
    std::thread::spawn(move || {
        let (prog_tx, prog_rx) = mpsc::channel::<ProgressMsg>();
        let relay = tx.clone();
        std::thread::spawn(move || {
            for msg in prog_rx {
                let _ = relay.send(AppEvent::Progress(msg));
            }
        });

        let result = (|| -> anyhow::Result<(Vec<CopyCandidate>, Vec<CopyCandidate>)> {
            let db = Database::open(&db_path)?;
            let drive_id = ensure_drive(&db, &drive_path)?;
            let opts = RunOptions {
                sd_path: &sd_path,
                drive_root: &drive_path,
                drive_id,
                use_hash,
                format: &crate::cli::OutputFormat::Table,
                dry_run: true,
                verbose: false,
            };
            build_candidates(&db, &opts, &config, Some(prog_tx))
        })();

        let event = match result {
            Ok((to_copy, already)) => AppEvent::CandidatesReady { to_copy, already },
            Err(e) => AppEvent::TaskDone {
                task: TaskKind::LoadCandidates,
                error: Some(e.to_string()),
            },
        };
        let _ = tx.send(event);
    });
}

fn spawn_copy_phase(
    tx: mpsc::Sender<AppEvent>,
    db_path: PathBuf,
    to_copy: Vec<CopyCandidate>,
    drive_root: PathBuf,
) {
    std::thread::spawn(move || {
        let (prog_tx, prog_rx) = mpsc::channel::<ProgressMsg>();
        let relay = tx.clone();
        std::thread::spawn(move || {
            for msg in prog_rx {
                let _ = relay.send(AppEvent::Progress(msg));
            }
        });

        let result = (|| -> anyhow::Result<()> {
            let db = Database::open(&db_path)?;
            let drive_id = ensure_drive(&db, &drive_root)?;
            let refs: Vec<&CopyCandidate> = to_copy.iter().collect();
            run_copy_phase(&db, &refs, drive_id, Some(prog_tx))?;
            Ok(())
        })();

        let _ = tx.send(AppEvent::TaskDone {
            task: TaskKind::Copy,
            error: result.err().map(|e| e.to_string()),
        });

        let _ = tx.send(AppEvent::TaskDone {
            task: TaskKind::LoadStats,
            error: None,
        });
    });
}

fn spawn_remove_drive(tx: mpsc::Sender<AppEvent>, db_path: PathBuf, root_path: String) {
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<()> {
            let db = Database::open(&db_path)?;
            db.remove_drive_by_path(&root_path)?;
            Ok(())
        })();
        let _ = tx.send(AppEvent::TaskDone {
            task: TaskKind::RemoveDrive,
            error: result.err().map(|e| e.to_string()),
        });
    });
}

fn spawn_add_drive(tx: mpsc::Sender<AppEvent>, db_path: PathBuf, path: String, label: String) {
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<()> {
            let db = Database::open(&db_path)?;
            db.upsert_drive(&label, &path)?;
            Ok(())
        })();
        let _ = tx.send(AppEvent::TaskDone {
            task: TaskKind::AddDrive,
            error: result.err().map(|e| e.to_string()),
        });
    });
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn ensure_drive(db: &Database, path: &std::path::Path) -> anyhow::Result<i64> {
    let path_str = path.to_string_lossy();
    if let Some(d) = db.find_drive_by_path(&path_str)? {
        return Ok(d.id);
    }
    let label = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    db.upsert_drive(&label, &path_str)
}

fn is_in_text_input(app: &App) -> bool {
    match app.screen {
        AppScreen::Scan => !app.scan.running,
        AppScreen::CheckCopy => !app.check_copy.running,
        AppScreen::Drives => app.drives.add_open,
        AppScreen::Home => false,
    }
}

fn insert_char(buf: &mut String, cursor: &mut usize, c: char) {
    buf.insert(*cursor, c);
    *cursor += c.len_utf8();
}

fn backspace_input(buf: &mut String, cursor: &mut usize) {
    if *cursor > 0 {
        let prev = buf[..*cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        buf.drain(prev..*cursor);
        *cursor = prev;
    }
}
