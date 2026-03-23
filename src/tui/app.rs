use std::sync::mpsc;

use super::events::{AppEvent, TaskKind};
use crate::models::{CopyCandidate, DriveRecord};
use crate::progress::ProgressMsg;

// ── Screen identifier ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Default)]
pub enum AppScreen {
    #[default]
    Home,
    Scan,
    CheckCopy,
    Drives,
}

// ── Per-screen state ─────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct HomeState {
    pub image_count: u64,
    pub drive_count: u64,
    pub unhashed_count: u64,
    pub loading: bool,
}

#[derive(Debug, Default)]
pub struct ScanState {
    pub path_input: String,
    pub input_cursor: usize,
    pub label_input: String,
    pub label_cursor: usize,
    /// 0 = path field, 1 = label field
    pub focused_field: usize,
    pub force_rehash: bool,
    pub running: bool,
    pub progress_current: u64,
    pub progress_total: u64,
    pub progress_detail: String,
    /// Capped at 200 lines
    pub log: Vec<String>,
    pub result: Option<String>,
}

#[derive(Debug, Default, PartialEq)]
pub enum CheckCopyMode {
    #[default]
    Check,
    Copy,
}

#[derive(Debug, Default)]
pub struct CheckCopyState {
    pub sd_input: String,
    pub sd_cursor: usize,
    pub drive_input: String,
    pub drive_cursor: usize,
    /// 0 = sd field, 1 = drive field
    pub focused_field: usize,
    pub use_hash: bool,
    pub mode: CheckCopyMode,
    // Candidate list
    pub to_copy: Vec<CopyCandidate>,
    pub already: Vec<CopyCandidate>,
    pub table_offset: usize,
    pub selected_row: usize,
    pub loading: bool,
    pub running: bool,
    pub progress_current: u64,
    pub progress_total: u64,
    pub progress_detail: String,
    pub log: Vec<String>,
    pub result: Option<String>,
}

#[derive(Debug, Default)]
pub struct DrivesState {
    pub drives: Vec<DriveRecord>,
    pub selected_row: usize,
    pub loading: bool,
    pub add_open: bool,
    pub add_path: String,
    pub add_path_cursor: usize,
    pub add_label: String,
    pub add_label_cursor: usize,
    pub add_focused: usize,
}

// ── Root App ─────────────────────────────────────────────────────────────────

pub struct App {
    pub screen: AppScreen,
    pub should_quit: bool,
    pub status_message: Option<String>,
    pub active_task: Option<TaskKind>,
    #[allow(dead_code)]
    pub event_tx: mpsc::Sender<AppEvent>,

    pub home: HomeState,
    pub scan: ScanState,
    pub check_copy: CheckCopyState,
    pub drives: DrivesState,
}

impl App {
    pub fn new(event_tx: mpsc::Sender<AppEvent>) -> Self {
        Self {
            screen: AppScreen::default(),
            should_quit: false,
            status_message: None,
            active_task: None,
            event_tx,
            home: HomeState::default(),
            scan: ScanState::default(),
            check_copy: CheckCopyState::default(),
            drives: DrivesState::default(),
        }
    }

    /// Handle a progress message from a background task.
    pub fn handle_progress(&mut self, msg: ProgressMsg) {
        match &self.active_task {
            Some(TaskKind::Scan) => match msg {
                ProgressMsg::Started { total, .. } => {
                    self.scan.progress_total = total;
                    self.scan.progress_current = 0;
                }
                ProgressMsg::Tick {
                    current,
                    total,
                    detail,
                } => {
                    self.scan.progress_current = current;
                    self.scan.progress_total = total;
                    if let Some(d) = detail {
                        self.scan.progress_detail = d;
                    }
                }
                ProgressMsg::Warning { message } => push_log(&mut self.scan.log, message),
                ProgressMsg::Finished { processed, .. } => {
                    self.scan.result = Some(format!("Indexed {processed} images."));
                }
                ProgressMsg::Failed { error } => {
                    self.scan.result = Some(format!("Error: {error}"));
                }
            },
            Some(TaskKind::Copy) | Some(TaskKind::LoadCandidates) => match msg {
                ProgressMsg::Started { total, .. } => {
                    self.check_copy.progress_total = total;
                    self.check_copy.progress_current = 0;
                }
                ProgressMsg::Tick {
                    current,
                    total,
                    detail,
                } => {
                    self.check_copy.progress_current = current;
                    if total > 0 {
                        self.check_copy.progress_total = total;
                    }
                    if let Some(d) = detail {
                        self.check_copy.progress_detail = d;
                    }
                }
                ProgressMsg::Warning { message } => {
                    push_log(&mut self.check_copy.log, message);
                }
                ProgressMsg::Finished { processed, .. } => {
                    self.check_copy.result = Some(format!("Done. {processed} files processed."));
                }
                ProgressMsg::Failed { error } => {
                    self.check_copy.result = Some(format!("Error: {error}"));
                }
            },
            Some(TaskKind::Hash) => match msg {
                ProgressMsg::Started { total, .. } => {
                    self.scan.progress_total = total;
                    self.scan.progress_current = 0;
                }
                ProgressMsg::Tick {
                    current,
                    total,
                    detail,
                } => {
                    self.scan.progress_current = current;
                    self.scan.progress_total = total;
                    if let Some(d) = detail {
                        self.scan.progress_detail = d;
                    }
                }
                ProgressMsg::Warning { message } => push_log(&mut self.scan.log, message),
                ProgressMsg::Finished { processed, errors } => {
                    self.scan.result = Some(format!("Hashed {processed} files. Errors: {errors}."));
                }
                ProgressMsg::Failed { error } => {
                    self.scan.result = Some(format!("Error: {error}"));
                }
            },
            _ => {} // progress for an unknown/finished task — ignore
        }
    }

    pub fn task_done(&mut self, task: TaskKind, error: Option<String>) {
        self.active_task = None;
        match task {
            TaskKind::Scan => self.scan.running = false,
            TaskKind::Copy => self.check_copy.running = false,
            TaskKind::LoadCandidates => self.check_copy.loading = false,
            TaskKind::LoadDrives => self.drives.loading = false,
            TaskKind::LoadStats => self.home.loading = false,
            TaskKind::Hash => self.scan.running = false,
            TaskKind::RemoveDrive | TaskKind::AddDrive => self.drives.loading = false,
        }
        if let Some(e) = error {
            self.status_message = Some(format!("Error: {e}"));
        }
    }
}

fn push_log(log: &mut Vec<String>, msg: String) {
    log.push(msg);
    if log.len() > 200 {
        log.remove(0);
    }
}
