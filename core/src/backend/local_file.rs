//! JSON-file-backed storage. The first, simplest `Backend` implementation —
//! reads/writes a single file under `~/.local/share/pulse/notes.json`.

use std::fs;
use std::path::PathBuf;

use crate::backend::{Backend, BackendError};
use crate::note::Note;

pub struct LocalFileBackend {
    path: PathBuf,
}

impl LocalFileBackend {
    /// Use an explicit path (mainly for tests). Production code should
    /// prefer `default_path()`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `~/.local/share/pulse/notes.json`, following the XDG data-home
    /// convention. Falls back to `./pulse-notes.json` if `$HOME` isn't
    /// set, which should only happen in unusual environments.
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."));

        base.join("pulse").join("notes.json")
    }

    fn ensure_parent_dir(&self) -> Result<(), BackendError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(())
    }
}

impl Backend for LocalFileBackend {
    fn read(&self) -> Result<Vec<Note>, BackendError> {
        if !self.path.exists() {
            // First run: nothing saved yet. Not an error.
            return Ok(Vec::new());
        }

        let contents = fs::read_to_string(&self.path)?;

        if contents.trim().is_empty() {
            return Ok(Vec::new());
        }

        match serde_json::from_str(&contents) {
            Ok(notes) => Ok(notes),
            Err(e) => {
                // Corrupt file: don't crash the daemon and don't silently
                // wipe the user's data by writing an empty list over it
                // later. Surface the error so the caller can decide
                // (log and start empty, refuse to start, etc.) rather
                // than guessing here.
                eprintln!(
                    "pulse: {} appears corrupt and could not be parsed: {e}",
                    self.path.display()
                );
                Err(BackendError::Serde(e))
            }
        }
    }

    fn write(&self, notes: &[Note]) -> Result<(), BackendError> {
        self.ensure_parent_dir()?;
        let json = serde_json::to_string_pretty(notes)?;

        // Write to a temp file then rename, so a crash mid-write can't
        // leave notes.json truncated or half-written.
        let tmp_path = self.path.with_extension("json.tmp");
        fs::write(&tmp_path, json)?;
        fs::rename(&tmp_path, &self.path)?;

        Ok(())
    }

    fn name(&self) -> &str {
        "local_file"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn temp_backend(test_name: &str) -> LocalFileBackend {
        let path = std::env::temp_dir().join(format!("pulse-test-{test_name}.json"));
        let _ = fs::remove_file(&path); // start clean if a prior run left it
        LocalFileBackend::new(path)
    }

    #[test]
    fn first_run_returns_empty_vec() {
        let backend = temp_backend("first_run");
        let notes = backend.read().expect("read should succeed on missing file");
        assert!(notes.is_empty());
    }

    #[test]
    fn write_then_read_round_trips() {
        let backend = temp_backend("round_trip");
        let note = Note::new("test note", HashMap::new());

        backend.write(&[note.clone()]).expect("write");
        let read_back = backend.read().expect("read");

        assert_eq!(read_back, vec![note]);
    }

    #[test]
    fn corrupt_file_returns_error_not_panic() {
        let backend = temp_backend("corrupt");
        fs::write(&backend.path, "{ not valid json").expect("seed corrupt file");

        let result = backend.read();
        assert!(result.is_err(), "corrupt file must surface as an error, not panic or silently empty");
    }
}
