//! Unix domain socket server. Accepts newline-delimited JSON requests
//! (see docs/protocol.md and protocol.rs), dispatches them to a `Store`
//! and `Config`, and writes back a newline-delimited JSON response per
//! connection.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::backend::Backend;
use crate::config::Config;
use crate::protocol::{Request, Response, ResponseData};
use crate::store::{Store, StoreError};

/// `$XDG_RUNTIME_DIR/pulse.sock`, falling back to a temp-dir path if
/// `XDG_RUNTIME_DIR` isn't set (unusual, but shouldn't crash startup).
pub fn default_socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    base.join("pulse.sock")
}

/// Bind the socket, removing any stale socket file left behind by a
/// previous run that didn't shut down cleanly. If another daemon
/// instance is actually still running and holding the socket, this
/// will fail loudly (bind error) rather than silently stealing it —
/// see the "two daemons at once" hardening checklist item.
pub fn listen(path: &Path) -> std::io::Result<UnixListener> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    UnixListener::bind(path)
}

/// Shared daemon state a connection handler needs access to.
pub struct DaemonState<B: Backend> {
    pub store: Mutex<Store<B>>,
    pub config: Mutex<Config>,
    pub config_path: PathBuf,
}

/// Run the accept loop until `shutdown` is set (by a signal handler —
/// see main.rs). The listener is set non-blocking with a short poll
/// timeout so a pending `accept()` doesn't block the shutdown flag
/// from ever being checked.
///
/// Every store mutation already persists synchronously before its
/// response is sent (see store.rs), so there's no write buffer to
/// flush here — shutdown just needs to stop accepting and remove the
/// socket file so a restart doesn't need the stale-socket workaround.
pub fn run<B: Backend>(listener: UnixListener, state: Arc<DaemonState<B>>, shutdown: Arc<AtomicBool>) {
    listener
        .set_nonblocking(true)
        .expect("failed to set listener non-blocking");

    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _addr)) => handle_connection(stream, &state),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => eprintln!("pulse: failed to accept connection: {e}"),
        }
    }
}

fn handle_connection<B: Backend>(stream: UnixStream, state: &Arc<DaemonState<B>>) {
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("pulse: failed to clone stream for writing: {e}");
            return;
        }
    };
    let reader = BufReader::new(stream);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("pulse: error reading from client: {e}");
                break;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let response = dispatch(&line, state);
        let response_json = match serde_json::to_string(&response) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("pulse: failed to serialize response: {e}");
                continue;
            }
        };

        if writeln!(writer, "{response_json}").is_err() {
            // Client disconnected; stop serving this connection.
            break;
        }
    }
}

fn dispatch<B: Backend>(line: &str, state: &Arc<DaemonState<B>>) -> Response {
    let request: Request = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => return Response::err(format!("malformed request: {e}")),
    };

    match request {
        Request::ListNotes { filter } => {
            let store = match state.store.lock() {
                Ok(s) => s,
                Err(_) => return Response::err("internal error: store lock poisoned"),
            };
            let filter_pair = filter
                .as_ref()
                .and_then(|f| f.iter().next())
                .map(|(k, v)| (k.as_str(), v.as_str()));
            let notes = store.list_notes(filter_pair);
            Response::ok(ResponseData::Notes(notes))
        }

        Request::AddNote { text, properties } => {
            let mut store = match state.store.lock() {
                Ok(s) => s,
                Err(_) => return Response::err("internal error: store lock poisoned"),
            };
            match store.add_note(text, properties) {
                Ok(note) => Response::ok(ResponseData::Note(note)),
                Err(e) => Response::err(store_error_message(&e)),
            }
        }

        Request::UpdateNote {
            id,
            text,
            properties,
        } => {
            let mut store = match state.store.lock() {
                Ok(s) => s,
                Err(_) => return Response::err("internal error: store lock poisoned"),
            };
            match store.update_note(id, text, properties) {
                Ok(note) => Response::ok(ResponseData::Note(note)),
                Err(e) => Response::err(store_error_message(&e)),
            }
        }

        Request::DeleteNote { id } => {
            let mut store = match state.store.lock() {
                Ok(s) => s,
                Err(_) => return Response::err("internal error: store lock poisoned"),
            };
            match store.delete_note(id) {
                Ok(()) => Response::ok(ResponseData::Null),
                Err(e) => Response::err(store_error_message(&e)),
            }
        }

        // No store interaction: this just signals a UI to render.
        // The daemon itself doesn't push anything — see the "push vs
        // pull" decision recorded in docs/protocol.md.
        Request::ShowNow => Response::ok(ResponseData::Null),

        Request::GetInterval => {
            let config = match state.config.lock() {
                Ok(c) => c,
                Err(_) => return Response::err("internal error: config lock poisoned"),
            };
            Response::ok(ResponseData::Interval {
                seconds: config.interval_seconds,
            })
        }

        Request::SetInterval { seconds } => {
            let mut config = match state.config.lock() {
                Ok(c) => c,
                Err(_) => return Response::err("internal error: config lock poisoned"),
            };
            config.interval_seconds = seconds;

            if let Err(e) = config.save(&state.config_path) {
                return Response::err(format!("failed to persist config: {e}"));
            }

            Response::ok(ResponseData::Interval { seconds })
        }
    }
}

fn store_error_message(e: &StoreError) -> String {
    e.to_string()
}
