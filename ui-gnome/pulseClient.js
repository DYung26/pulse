// Client for talking to pulse-daemon over its Unix socket.
//
// All GIO calls here are async — GNOME Shell's main loop must never be
// blocked by a synchronous socket call, or the whole desktop would
// freeze. Gio._promisify() turns GIO's async/finish method pairs into
// real Promises so this can be written with async/await instead of
// nested callbacks.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

// Promisify once at module load, not per-call.
Gio._promisify(Gio.SocketClient.prototype, 'connect_async', 'connect_finish');
Gio._promisify(Gio.OutputStream.prototype, 'write_bytes_async', 'write_bytes_finish');
Gio._promisify(Gio.DataInputStream.prototype, 'read_line_async', 'read_line_finish_utf8');

function socketPath() {
    const runtimeDir = GLib.getenv('XDG_RUNTIME_DIR') ?? GLib.get_tmp_dir();
    return GLib.build_filenamev([runtimeDir, 'pulse.sock']);
}

/**
 * Send one request object, read back one parsed JSON response.
 * Mirrors core/src/client.rs (the CLI's equivalent) and
 * docs/protocol.md — every request is one line of JSON out, one line
 * of JSON back.
 *
 * Throws if the daemon isn't running or the connection otherwise
 * fails; callers must handle that rather than let it propagate into
 * GNOME Shell as an unhandled error.
 */
export async function send(request) {
    const address = Gio.UnixSocketAddress.new(socketPath());
    const client = new Gio.SocketClient();

    const connection = await client.connect_async(address, null);

    const requestLine = JSON.stringify(request) + '\n';
    const outputStream = connection.get_output_stream();
    await outputStream.write_bytes_async(
        new GLib.Bytes(new TextEncoder().encode(requestLine)),
        GLib.PRIORITY_DEFAULT,
        null,
    );

    const inputStream = new Gio.DataInputStream({
        base_stream: connection.get_input_stream(),
    });
    const [line] = await inputStream.read_line_async(GLib.PRIORITY_DEFAULT, null);

    if (line === null) {
        throw new Error('pulse-daemon closed the connection without responding');
    }

    return JSON.parse(line);
}
