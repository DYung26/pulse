//! Small daemon-side config. Currently just the periodic-overlay
//! interval, but this is the natural home for any future
//! daemon-wide (not per-note) setting.
//!
//! Lives in the daemon rather than GNOME's GSettings so a future
//! Windows UI can share the same value instead of reinventing its own
//! copy — see docs/protocol.md.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const DEFAULT_INTERVAL_SECONDS: u64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub interval_seconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_seconds: DEFAULT_INTERVAL_SECONDS,
        }
    }
}

impl Config {
    /// `~/.local/share/pulse/config.json`, alongside notes.json.
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."));

        base.join("pulse").join("config.json")
    }

    /// Load from disk, falling back to defaults on first run or if the
    /// file is missing/corrupt. Config is low-stakes enough that a
    /// corrupt file resetting to defaults is acceptable (unlike notes,
    /// where we refuse to guess and surface an error instead).
    pub fn load(path: &std::path::Path) -> Self {
        match fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|e| {
                eprintln!(
                    "pulse: {} is unreadable ({e}), using default config",
                    path.display()
                );
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).expect("config always serializes");
        fs::write(path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let path = std::env::temp_dir().join("pulse-config-test-missing.json");
        let _ = fs::remove_file(&path);

        let config = Config::load(&path);
        assert_eq!(config.interval_seconds, DEFAULT_INTERVAL_SECONDS);
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = std::env::temp_dir().join("pulse-config-test-roundtrip.json");
        let _ = fs::remove_file(&path);

        let config = Config {
            interval_seconds: 120,
        };
        config.save(&path).expect("save");

        let loaded = Config::load(&path);
        assert_eq!(loaded.interval_seconds, 120);
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let path = std::env::temp_dir().join("pulse-config-test-corrupt.json");
        fs::write(&path, "{ not valid json").expect("seed corrupt file");

        let config = Config::load(&path);
        assert_eq!(config.interval_seconds, DEFAULT_INTERVAL_SECONDS);
    }
}
