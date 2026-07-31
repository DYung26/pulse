//! In-memory note store, backed by a `Backend` for persistence.
//!
//! This is the layer daemon/socket code actually talks to. It holds the
//! current notes in memory for fast reads, and write-through persists
//! to the backend after every mutation (see docs/_local/checklist.md —
//! deliberately simple, no batching).

use std::collections::HashMap;

use uuid::Uuid;

use crate::backend::{Backend, BackendError};
use crate::note::Note;

#[derive(Debug)]
pub enum StoreError {
    NotFound(Uuid),
    Backend(BackendError),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::NotFound(id) => write!(f, "no note found with id {id}"),
            StoreError::Backend(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<BackendError> for StoreError {
    fn from(e: BackendError) -> Self {
        StoreError::Backend(e)
    }
}

pub struct Store<B: Backend> {
    backend: B,
    notes: Vec<Note>,
}

impl<B: Backend> Store<B> {
    /// Load existing notes from the backend to seed the in-memory store.
    pub fn load(backend: B) -> Result<Self, StoreError> {
        let notes = backend.read()?;
        Ok(Self { backend, notes })
    }

    fn persist(&self) -> Result<(), StoreError> {
        self.backend.write(&self.notes)?;
        Ok(())
    }

    /// Which backend this store is persisting through, for startup
    /// logging/diagnostics.
    pub fn backend_name(&self) -> &str {
        self.backend.name()
    }

    pub fn add_note(
        &mut self,
        text: impl Into<String>,
        properties: HashMap<String, String>,
    ) -> Result<Note, StoreError> {
        let note = Note::new(text, properties);
        self.notes.push(note.clone());
        self.persist()?;
        Ok(note)
    }

    pub fn update_note(
        &mut self,
        id: Uuid,
        text: Option<String>,
        properties: Option<HashMap<String, String>>,
    ) -> Result<Note, StoreError> {
        let note = self
            .notes
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or(StoreError::NotFound(id))?;

        note.apply_update(text, properties);
        let updated = note.clone();

        self.persist()?;
        Ok(updated)
    }

    pub fn delete_note(&mut self, id: Uuid) -> Result<(), StoreError> {
        let starting_len = self.notes.len();
        self.notes.retain(|n| n.id != id);

        if self.notes.len() == starting_len {
            return Err(StoreError::NotFound(id));
        }

        self.persist()?;
        Ok(())
    }

    /// List notes, optionally filtered to those with a given
    /// property key/value pair. `None` returns every note.
    pub fn list_notes(&self, filter: Option<(&str, &str)>) -> Vec<Note> {
        match filter {
            None => self.notes.clone(),
            Some((key, value)) => self
                .notes
                .iter()
                .filter(|n| n.properties.get(key).map(|v| v.as_str()) == Some(value))
                .cloned()
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::local_file::LocalFileBackend;

    fn temp_store(test_name: &str) -> Store<LocalFileBackend> {
        let path = std::env::temp_dir().join(format!("pulse-store-test-{test_name}.json"));
        let _ = std::fs::remove_file(&path);
        Store::load(LocalFileBackend::new(path)).expect("load fresh store")
    }

    #[test]
    fn add_note_persists_and_is_listed() {
        let mut store = temp_store("add");
        let added = store.add_note("test note", HashMap::new()).expect("add");

        let all = store.list_notes(None);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, added.id);
    }

    #[test]
    fn update_note_changes_text_and_bumps_timestamp() {
        let mut store = temp_store("update");
        let added = store.add_note("original", HashMap::new()).expect("add");
        let original_updated_at = added.updated_at;

        let updated = store
            .update_note(added.id, Some("changed".into()), None)
            .expect("update");

        assert_eq!(updated.text, "changed");
        assert!(updated.updated_at >= original_updated_at);
    }

    #[test]
    fn update_missing_note_returns_not_found() {
        let mut store = temp_store("update_missing");
        let result = store.update_note(Uuid::new_v4(), Some("x".into()), None);
        assert!(matches!(result, Err(StoreError::NotFound(_))));
    }

    #[test]
    fn delete_note_removes_it() {
        let mut store = temp_store("delete");
        let added = store.add_note("to delete", HashMap::new()).expect("add");

        store.delete_note(added.id).expect("delete");

        assert!(store.list_notes(None).is_empty());
    }

    #[test]
    fn delete_missing_note_returns_not_found() {
        let mut store = temp_store("delete_missing");
        let result = store.delete_note(Uuid::new_v4());
        assert!(matches!(result, Err(StoreError::NotFound(_))));
    }

    #[test]
    fn list_notes_filters_by_property() {
        let mut store = temp_store("filter");

        let mut ai_props = HashMap::new();
        ai_props.insert("group".to_string(), "AI accounts".to_string());
        store.add_note("account A rate-limited", ai_props.clone()).expect("add");
        store.add_note("account B rate-limited", ai_props).expect("add");

        let mut sub_props = HashMap::new();
        sub_props.insert("group".to_string(), "Subscriptions".to_string());
        store.add_note("Netflix renews Friday", sub_props).expect("add");

        let ai_only = store.list_notes(Some(("group", "AI accounts")));
        assert_eq!(ai_only.len(), 2);

        let subs_only = store.list_notes(Some(("group", "Subscriptions")));
        assert_eq!(subs_only.len(), 1);
    }

    #[test]
    fn store_reloads_persisted_notes_across_instances() {
        let path = std::env::temp_dir().join("pulse-store-test-reload.json");
        let _ = std::fs::remove_file(&path);

        {
            let mut store = Store::load(LocalFileBackend::new(path.clone())).expect("load");
            store.add_note("survives reload", HashMap::new()).expect("add");
        }

        let reloaded = Store::load(LocalFileBackend::new(path)).expect("reload");
        assert_eq!(reloaded.list_notes(None).len(), 1);
    }
}
