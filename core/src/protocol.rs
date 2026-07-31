//! Request/response types for the local socket API. These are the Rust
//! mirror of docs/protocol.md — keep both in sync when either changes.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::note::Note;

/// One line of JSON, tagged by `action`. Deserialized by the daemon,
/// serialized by any client (CLI, GNOME extension via its own JSON
/// encoding, future Windows client).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Request {
    ListNotes {
        #[serde(default)]
        filter: Option<HashMap<String, String>>,
    },
    AddNote {
        text: String,
        #[serde(default)]
        properties: HashMap<String, String>,
    },
    UpdateNote {
        id: Uuid,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        properties: Option<HashMap<String, String>>,
    },
    DeleteNote {
        id: Uuid,
    },
    ShowNow,
    GetInterval,
    SetInterval {
        seconds: u64,
    },
}

/// One line of outgoing JSON. `ok: true` responses carry `data`;
/// `ok: false` responses carry `error`. Modeled as two structs rather
/// than one flexible one so serialization always matches
/// docs/protocol.md exactly regardless of which variant is built.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Response {
    Ok { ok: bool, data: ResponseData },
    Err { ok: bool, error: String },
}

impl Response {
    pub fn ok(data: ResponseData) -> Self {
        Response::Ok { ok: true, data }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Response::Err {
            ok: false,
            error: message.into(),
        }
    }
}

/// The `data` payload shape varies by which request produced it.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ResponseData {
    Notes(Vec<Note>),
    Note(Note),
    Interval { seconds: u64 },
    Null,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_notes_request_parses_without_filter() {
        let json = r#"{"action":"list_notes"}"#;
        let req: Request = serde_json::from_str(json).expect("parse");
        assert!(matches!(req, Request::ListNotes { filter: None }));
    }

    #[test]
    fn add_note_request_parses_with_properties() {
        let json = r#"{"action":"add_note","text":"hello","properties":{"group":"AI accounts"}}"#;
        let req: Request = serde_json::from_str(json).expect("parse");
        match req {
            Request::AddNote { text, properties } => {
                assert_eq!(text, "hello");
                assert_eq!(properties.get("group"), Some(&"AI accounts".to_string()));
            }
            _ => panic!("expected AddNote"),
        }
    }

    #[test]
    fn show_now_request_parses_with_no_fields() {
        let json = r#"{"action":"show_now"}"#;
        let req: Request = serde_json::from_str(json).expect("parse");
        assert!(matches!(req, Request::ShowNow));
    }

    #[test]
    fn get_interval_request_parses() {
        let json = r#"{"action":"get_interval"}"#;
        let req: Request = serde_json::from_str(json).expect("parse");
        assert!(matches!(req, Request::GetInterval));
    }

    #[test]
    fn set_interval_request_parses_with_seconds() {
        let json = r#"{"action":"set_interval","seconds":120}"#;
        let req: Request = serde_json::from_str(json).expect("parse");
        match req {
            Request::SetInterval { seconds } => assert_eq!(seconds, 120),
            _ => panic!("expected SetInterval"),
        }
    }

    #[test]
    fn add_note_request_serializes_for_sending() {
        let req = Request::AddNote {
            text: "hello".to_string(),
            properties: HashMap::new(),
        };
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(json.contains(r#""action":"add_note""#));
    }

    #[test]
    fn ok_response_serializes_with_ok_true() {
        let response = Response::ok(ResponseData::Null);
        let json = serde_json::to_string(&response).expect("serialize");
        assert!(json.contains(r#""ok":true"#));
    }

    #[test]
    fn err_response_serializes_with_ok_false_and_message() {
        let response = Response::err("no note found");
        let json = serde_json::to_string(&response).expect("serialize");
        assert!(json.contains(r#""ok":false"#));
        assert!(json.contains("no note found"));
    }

    #[test]
    fn interval_response_serializes_with_seconds() {
        let response = Response::ok(ResponseData::Interval { seconds: 300 });
        let json = serde_json::to_string(&response).expect("serialize");
        assert!(json.contains(r#""seconds":300"#));
    }
}
