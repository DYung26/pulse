use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use pulse_core::backend::local_file::LocalFileBackend;
use pulse_core::config::Config;
use pulse_core::socket::{self, DaemonState};
use pulse_core::store::Store;

fn main() {
    let backend = LocalFileBackend::new(LocalFileBackend::default_path());
    let store = Store::load(backend).expect("failed to load note store");
    println!("pulse-daemon: backend = {}", store.backend_name());

    let config_path = Config::default_path();
    let config = Config::load(&config_path);
    println!(
        "pulse-daemon: interval = {}s (config at {})",
        config.interval_seconds,
        config_path.display()
    );

    let state = Arc::new(DaemonState {
        store: Mutex::new(store),
        config: Mutex::new(config),
        config_path,
    });

    let socket_path = socket::default_socket_path();
    let listener = socket::listen(&socket_path).unwrap_or_else(|e| {
        panic!(
            "pulse: failed to bind socket at {}: {e}. \
             Is another pulse-daemon instance already running?",
            socket_path.display()
        )
    });

    // Shared flag, flipped by SIGTERM/SIGINT. The socket accept loop
    // polls this instead of blocking forever, so shutdown is prompt.
    let shutdown = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        signal_hook::flag::register(signal, Arc::clone(&shutdown))
            .expect("failed to register signal handler");
    }

    println!("pulse-daemon: listening on {}", socket_path.display());
    socket::run(listener, state, Arc::clone(&shutdown));

    // Every mutation already persisted synchronously before responding
    // (see store.rs), so there's nothing to flush here — just tidy up
    // the socket file so a future start doesn't need the stale-socket
    // cleanup path.
    let _ = std::fs::remove_file(&socket_path);
    println!("pulse-daemon: shut down cleanly");
}
