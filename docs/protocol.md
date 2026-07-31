# Pulse — Core/UI Socket Protocol

The daemon (`core/`) exposes a local socket. Any UI (GNOME extension, CLI,
future Windows client) talks to it over this protocol instead of touching
the data file directly. This is what keeps `core/` reusable across UIs.

## Transport

- Linux: Unix domain socket at `$XDG_RUNTIME_DIR/pulse.sock`
- Windows (future): named pipe, same message shapes

## Message format

Newline-delimited JSON. One request per line, one response per line.

### Request shape

```json
{ "action": "<action_name>", ...action-specific fields }
```

### Response shape

```json
{ "ok": true, "data": { ... } }
```
or
```json
{ "ok": false, "error": "<message>" }
```

## Actions

### `list_notes`
Request:
```json
{ "action": "list_notes", "filter": { "key": "value" } }
```
`filter` is optional — omit to list all notes.

Response `data`: array of Note objects.

### `add_note`
Request:
```json
{ "action": "add_note", "text": "...", "properties": { "key": "value" } }
```
`properties` is optional (defaults to empty).

Response `data`: the created Note.

### `update_note`
Request:
```json
{ "action": "update_note", "id": "...", "text": "...", "properties": { "key": "value" } }
```
`text` and `properties` are both optional — include only what's changing.
`properties` replaces the full map, it does not merge (keep this simple;
revisit if partial-merge turns out to matter in practice).

Response `data`: the updated Note.

### `delete_note`
Request:
```json
{ "action": "delete_note", "id": "..." }
```
Response `data`: `null` on success.

### `show_now`
Request:
```json
{ "action": "show_now" }
```
Signals a UI-side "render immediately, stay open until dismissed" event.
The daemon does not push this proactively — see note below.

### `get_interval`
Request:
```json
{ "action": "get_interval" }
```
Response `data`: `{ "seconds": 300 }`

### `set_interval`
Request:
```json
{ "action": "set_interval", "seconds": 300 }
```
Response `data`: `{ "seconds": 300 }` (the value now in effect).

## Note object shape

```json
{
  "id": "uuid-v4-string",
  "text": "cryptoleinad@gmail.com rate-limited",
  "properties": { "resets_at": "15:00", "group": "AI accounts" },
  "created_at": "2026-07-30T12:00:00Z",
  "updated_at": "2026-07-30T12:00:00Z"
}
```

`properties` is a flat string-to-string map. No key is special-cased by
the daemon — grouping, status, etc. are just properties a UI may choose
to filter or display differently.

## Decided: push vs. pull for the periodic timer

The GNOME extension owns its own timer and calls `list_notes` on its own
schedule, rather than the daemon proactively pushing a "show" event over
the socket. Simpler, and avoids needing a persistent push channel.

## Decided: where the interval value lives

The interval (how often the overlay auto-shows) lives in the **daemon**,
not in GNOME's GSettings. It's OS-agnostic — about the person's rhythm,
not about GNOME specifically — so a future Windows UI reads/writes the
same value via `get_interval`/`set_interval` instead of maintaining its
own separate setting. `prefs.js` edits it through the socket.
