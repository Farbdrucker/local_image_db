use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::mpsc;

use crate::models::{CopyCandidate, DriveRecord};
use crate::progress::ProgressMsg;

#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    Tick,
    Progress(ProgressMsg),
    CandidatesReady {
        to_copy: Vec<CopyCandidate>,
        already: Vec<CopyCandidate>,
    },
    DrivesLoaded(Vec<DriveRecord>),
    StatsLoaded {
        image_count: u64,
        drive_count: u64,
        unhashed_count: u64,
    },
    TaskDone {
        task: TaskKind,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskKind {
    Scan,
    Copy,
    LoadCandidates,
    #[allow(dead_code)]
    Hash,
    LoadDrives,
    LoadStats,
    RemoveDrive,
    AddDrive,
}

/// Spawn a thread that polls crossterm for input events and forwards them
/// to the app event channel. Sends `AppEvent::Tick` on timeout (≈16 ms).
pub fn spawn_input_thread(tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        loop {
            match crossterm::event::poll(std::time::Duration::from_millis(16)) {
                Ok(true) => {
                    if let Ok(crossterm::event::Event::Key(k)) = crossterm::event::read()
                        && tx.send(AppEvent::Key(k)).is_err()
                    {
                        break;
                    }
                }
                Ok(false) => {
                    if tx.send(AppEvent::Tick).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
}

/// Returns true if the key event represents a quit intent (q or Ctrl+C).
pub fn is_quit(key: &KeyEvent) -> bool {
    matches!(
        key,
        KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            ..
        } | KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        }
    )
}
