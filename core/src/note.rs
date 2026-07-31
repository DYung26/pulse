//! Core data model: the `Note` type shared by every backend and every UI.
//!
//! A Note is deliberately minimal: free text plus an open property bag.
//! No field here is a task, a status, or a deadline — see docs/protocol.md
//! and docs/_local/checklist.md for why that's an intentional boundary.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single note: some text, an arbitrary set of properties, and
/// timestamps for staleness display.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Note {
    pub id: Uuid,
    pub text: String,

    /// Free-form key/value tags. No key is special-cased by the core —
    /// "status", "group", etc. are just properties a UI may choose to
    /// filter or display differently.
    #[serde(default)]
    pub properties: HashMap<String, String>,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Note {
    /// Create a new note with a fresh id and both timestamps set to now.
    pub fn new(text: impl Into<String>, properties: HashMap<String, String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            text: text.into(),
            properties,
            created_at: now,
            updated_at: now,
        }
    }

    /// Replace text and/or properties, bumping `updated_at`. `None` means
    /// "leave this field unchanged" (matches the update_note protocol,
    /// where properties fully replace rather than merge when provided).
    pub fn apply_update(&mut self, text: Option<String>, properties: Option<HashMap<String, String>>) {
        if let Some(text) = text {
            self.text = text;
        }
        if let Some(properties) = properties {
            self.properties = properties;
        }
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_note_has_matching_timestamps() {
        let note = Note::new("test", HashMap::new());
        assert_eq!(note.created_at, note.updated_at);
        assert_eq!(note.text, "test");
    }

    #[test]
    fn apply_update_changes_only_given_fields() {
        let mut note = Note::new("original", HashMap::new());
        let created = note.created_at;

        note.apply_update(Some("changed".into()), None);

        assert_eq!(note.text, "changed");
        assert_eq!(note.created_at, created, "created_at must never change");
        assert!(note.updated_at >= created);
    }

    #[test]
    fn serializes_to_json_and_back() {
        let mut props = HashMap::new();
        props.insert("group".to_string(), "AI accounts".to_string());

        let note = Note::new("cryptoleinad@gmail.com rate-limited", props);
        let json = serde_json::to_string(&note).expect("serialize");
        let restored: Note = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(note, restored);
    }
}
