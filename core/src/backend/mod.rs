//! Storage backend abstraction. `Backend` is the seam that lets the note
//! store persist to different places (local file today; Notion, a DB,
//! etc. later) without the rest of the daemon caring which one is active.

pub mod local_file;

use crate::note::Note;

/// Something that can load and save the full set of notes.
///
/// Deliberately whole-collection read/write rather than per-note CRUD —
/// keeps every backend implementation simple (just serialize/deserialize
/// a list) and pushes merge/diff logic to a future sync layer instead of
/// duplicating it into every backend.
pub trait Backend {
    /// Load all notes currently persisted. Returns an empty vec if
    /// nothing has been saved yet (first run).
    fn read(&self) -> Result<Vec<Note>, BackendError>;

    /// Overwrite persisted state with the given notes.
    fn write(&self, notes: &[Note]) -> Result<(), BackendError>;

    /// Human-readable identifier for logs/diagnostics (e.g. "local_file").
    fn name(&self) -> &str;
}

#[derive(Debug)]
pub enum BackendError {
    Io(std::io::Error),
    Serde(serde_json::Error),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendError::Io(e) => write!(f, "storage io error: {e}"),
            BackendError::Serde(e) => write!(f, "storage serialization error: {e}"),
        }
    }
}

impl std::error::Error for BackendError {}

impl From<std::io::Error> for BackendError {
    fn from(e: std::io::Error) -> Self {
        BackendError::Io(e)
    }
}

impl From<serde_json::Error> for BackendError {
    fn from(e: serde_json::Error) -> Self {
        BackendError::Serde(e)
    }
}
