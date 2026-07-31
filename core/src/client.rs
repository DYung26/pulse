//! A minimal client for talking to the pulse-daemon over its socket.
//! Used by the CLI; a future Windows client would implement the same
//! request/response shapes over a named pipe instead.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use crate::protocol::Request;
use crate::socket::default_socket_path;

#[derive(Debug)]
pub enum ClientError {
    Connect(std::io::Error),
    Io(std::io::Error),
    Parse(serde_json::Error),
    NoResponse,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Connect(e) => write!(
                f,
                "could not connect to pulse-daemon: {e}. Is the daemon running?"
            ),
            ClientError::Io(e) => write!(f, "communication error: {e}"),
            ClientError::Parse(e) => write!(f, "malformed response from daemon: {e}"),
            ClientError::NoResponse => write!(f, "daemon closed the connection without responding"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Send one request, read back one response line, return the raw JSON
/// so callers can decide how to present it (the CLI mostly just wants
/// to pretty-print it).
pub fn send(request: &Request) -> Result<serde_json::Value, ClientError> {
    let socket_path = default_socket_path();
    let stream = UnixStream::connect(&socket_path).map_err(ClientError::Connect)?;

    let mut writer = stream.try_clone().map_err(ClientError::Io)?;
    let request_json = serde_json::to_string(request).expect("request always serializes");
    writeln!(writer, "{request_json}").map_err(ClientError::Io)?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let bytes_read = reader.read_line(&mut line).map_err(ClientError::Io)?;

    if bytes_read == 0 {
        return Err(ClientError::NoResponse);
    }

    serde_json::from_str(&line).map_err(ClientError::Parse)
}
