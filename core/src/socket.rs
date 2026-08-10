//! Unix domain socket server. Accepts newline-delimited JSON requests
//! (see docs/protocol.md and protocol.rs), dispatches them to a `Store`
//! and `Config`, and writes back a newline-delimited JSON response per
//! connection.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::backend::Backend;
use crate::config::Config;
use crate::protocol::{Request, Response, ResponseData};
use crate::store::{Store, StoreError};

/// `$PULSE_SOCKET_PATH` if set (e.g. for daemon/client pairs split
/// across a container boundary, where `$XDG_RUNTIME_DIR` isn't a
/// shared filesystem location); otherwise `$XDG_RUNTIME_DIR/pulse.sock`,
/// falling back to a temp-dir path if `XDG_RUNTIME_DIR` isn't set
/// either (unusual, but shouldn't crash startup).
///
/// Both the daemon (main.rs) and every client (client.rs, and so
/// pulse-mcp) call this same function, so setting the override once
/// keeps them pointed at the same socket without needing to thread a
/// path through both binaries separately.
pub fn default_socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("PULSE_SOCKET_PATH") {
        return PathBuf::from(path);
    }

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
///
/// Creates the socket's parent directory if it doesn't exist yet.
/// `$XDG_RUNTIME_DIR` is guaranteed to already exist (it's a
/// login-session tmpfs managed by the OS/systemd), but a
/// `$PULSE_SOCKET_PATH` override can point anywhere — e.g. a fresh
/// `~/.local/run/` that's never been created — and `UnixListener::bind`
/// does not create missing parent directories on its own.
pub fn listen(path: &Path) -> std::io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    // `bind()` creates the socket file honoring the process umask like
    // any other file creation syscall, so under a default umask
    // (typically 022) the socket ends up mode 644 — no write bit for
    // group/other. A Unix socket's `connect()` requires write
    // permission on the socket node, so any peer outside the owning
    // UID/GID (e.g. a different UID inside a container that has the
    // parent directory bind-mounted in) gets a permission-denied error
    // at connect time even though the path is fully reachable. Relying
    // on the caller's umask being permissive enough is fragile and
    // non-obvious from the failure mode, so set it explicitly here.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o777))?;
    Ok(listener)
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
