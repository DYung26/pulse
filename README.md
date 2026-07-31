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
