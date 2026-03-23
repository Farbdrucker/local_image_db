use std::sync::mpsc;

/// Progress messages emitted by long-running backend operations.
/// Used by the TUI to drive progress bars and log displays.
/// When a backend receives `None` for its progress parameter it uses
/// indicatif directly (CLI path); when it receives `Some(tx)` it sends
/// these messages instead.
#[derive(Debug, Clone)]
pub enum ProgressMsg {
    /// The operation has started and the total work unit count is known.
    Started {
        total: u64,
        #[allow(dead_code)]
        label: String,
    },
    /// One work unit completed.
    Tick {
        current: u64,
        total: u64,
        detail: Option<String>,
    },
    /// A non-fatal warning (e.g. unreadable file).
    Warning { message: String },
    /// The operation completed successfully.
    Finished { processed: u64, errors: u64 },
    /// The operation failed with a fatal error.
    #[allow(dead_code)]
    Failed { error: String },
}

/// Sender half passed to backend functions.
pub type ProgressTx = mpsc::Sender<ProgressMsg>;
