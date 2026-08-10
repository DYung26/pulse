# Pulse

A small overlay for tracking ambient state while multitasking: what you're
doing right now, rate-limited accounts, subscriptions, codespaces, browser
profiles — anything you want visible without keeping it in your head.

Not a task manager (see [Chronix](../chronix) for scheduled/deadline-driven
work). Pulse stores manually-curated **notes**: free text plus arbitrary
key/value properties you define yourself. No auto-detection, no lifecycle,
no "done" state — you add, update, and delete notes by hand.

## Pieces

- **`core/`** — Rust daemon + CLI. Owns the note store, persistence, and a
  local Unix-socket API (`pulse-daemon`), plus a thin CLI client (`pulse`)
  for adding/listing/updating/deleting notes from a terminal.
  Platform-agnostic; the same daemon is meant to be reused if a Windows
  client is added later.
- **`ui-gnome/`** — GNOME Shell extension. Connects to the daemon over its
  local socket, renders the overlay (top-right, draggable, persisted
  position, black/white theme with a minimal accent color), handles a
  global keybind (stays open, dismissed by clicking it, pressing the
  keybind again, or Escape) and a periodic timer (auto-fades).
- **`ui-windows/`** — not started. Deferred until the GNOME side is solid.
- **`docs/protocol.md`** — the socket request/response contract between
  `core` and any UI. Read this before changing either side.
- **`docs/_local/checklist.md`** — running build checklist, including notes
  on GNOME/Wayland development quirks encountered along the way.

## Status

Core (daemon + CLI) fully functional. GNOME extension has a working
skeleton, daemon connection, overlay rendering, and trigger behavior
(keybind + timer + dismiss). Settings/preferences UI not yet built.

## Running

```bash
# Build and run the daemon (blocks; leave running in its own terminal)
cd core
cargo run

# CLI, from another terminal
./target/debug/pulse add "some note" --prop group="AI accounts"
./target/debug/pulse list
./target/debug/pulse list --filter group="AI accounts"
./target/debug/pulse update <id> --text "..." --prop key=value
./target/debug/pulse delete <id>
./target/debug/pulse show
```

## GNOME extension setup

The extension lives in this repo (`ui-gnome/`) but GNOME only looks for
extensions in a specific directory, keyed by UUID. Symlink it in rather
than copying, so edits here are picked up directly:

```bash
ln -s ~/Projects/pulse/ui-gnome ~/.local/share/gnome-shell/extensions/pulse@example.com
```

The extension uses a GSettings schema (keybinding, overlay position),
which must be compiled once (and again any time `schemas/*.gschema.xml`
changes):

```bash
glib-compile-schemas ~/Projects/pulse/ui-gnome/schemas/
```

Then enable it:

```bash
gnome-extensions enable pulse@example.com
```

**Important GNOME/Wayland quirk**: `gnome-extensions disable`/`enable`
re-runs the extension's `disable()`/`enable()` methods on the
already-loaded code, but does **not** re-read the JS files from disk. A
full logout/login is required to pick up any edit to `extension.js`,
`overlay.js`, `pulseClient.js`, or a newly-added file. See
`docs/_local/checklist.md` for the full account of this (including why
`gnome-shell --devkit --wayland`, the faster nested-session dev tool,
didn't work on this setup).

A systemd `--user` unit for auto-starting the daemon on login will be
added once daemon lifecycle hardening is finished (see checklist).

## Socket path configuration

By default, every piece talks over a Unix socket at
`$XDG_RUNTIME_DIR/pulse.sock` (falling back to a temp-dir path if that
var isn't set). This is fine as long as daemon and client are on the
same machine, in the same login session.

That assumption breaks if a client runs somewhere that doesn't share
`$XDG_RUNTIME_DIR` with the daemon's session — the motivating case is
`pulse-mcp` running inside a Docker container (e.g. proxied through
metamcp over Streamable HTTP), where `/run/user/1000` on the host isn't
visible at all inside the container.

For that case, set `PULSE_SOCKET_PATH` to an absolute path that *is*
visible to every process that needs it — typically somewhere under
`$HOME`, since that's easier to bind-mount into a container than `/run`.
When set, every piece below uses it verbatim instead of computing a path
from `$XDG_RUNTIME_DIR`.

**This has to be set independently in every place a process reads it,
since there's no OS mechanism that broadcasts one process's environment
to another.** As of this setup, that's three places:

1. **The daemon** — `pulse.service`'s `[Service]` block:
   ```ini
   Environment=PULSE_SOCKET_PATH=%h/.local/run/pulse.sock
   ```
2. **GNOME Shell** (`ui-gnome/pulseClient.js`) — GNOME Shell is a
   separate process from the daemon even though both run in the same
   login session, so it needs its own copy of the var. Session-wide env
   vars go in `~/.config/environment.d/*.conf` (read by `systemd --user`
   and graphical sessions; NOT the same expansion rules as unit files —
   `%h` does not work here, use a literal path):
   ```
   PULSE_SOCKET_PATH=/home/dyung/.local/run/pulse.sock
   ```
   Requires a full logout/login to take effect, same as any other
   `pulseClient.js` edit (see the Wayland quirk above).
3. **`pulse-mcp`, when proxied through metamcp in Docker** — metamcp
   spawns stdio MCP servers with a filtered environment (a small fixed
   allowlist, not a full inherit) merged with that server's own
   per-server `env` config. Setting `PULSE_SOCKET_PATH` on the
   `docker-compose.yml` `environment:` block does **not** reach the
   child process — it has to be set in metamcp's own config for the
   `pulse-mcp` server entry (its web UI/API, equivalent to
   `mcpServers.<name>.env` in a Claude Desktop-style config).

All three must point at the exact same path, and that path must be
readable/writable by whichever user each process runs as (the daemon's
user on the host; for the container, this works because `/home/dyung` is
already bind-mounted in — see `metamcp`'s `docker-compose.yml`).
