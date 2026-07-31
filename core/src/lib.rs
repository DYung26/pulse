//! Shared library code for pulse-core: the data model, storage,
//! socket protocol, and server. Both `pulse-daemon` (the server) and
//! `pulse` (the CLI client) depend on this crate rather than
//! duplicating any of it.

pub mod backend;
pub mod client;
pub mod config;
pub mod note;
pub mod protocol;
pub mod socket;
pub mod store;
