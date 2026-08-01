// Renders the note-list overlay: a small St.Widget added to GNOME
// Shell's chrome layer (Main.layoutManager.addChrome), positioned in
// the top-right corner (or a persisted dragged position), showing
// each note's text, properties, and last-updated time.
//
// Owns rendering, drag positioning (persisted via GSettings), and the
// show()/hide() fade transitions. Timer/keybind trigger logic and
// dismiss-on-click/Escape live in extension.js, which calls into this
// through onClick()/disconnectClick() rather than reaching into
// internals directly.

import St from 'gi://St';
import Clutter from 'gi://Clutter';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const MARGIN_FROM_EDGE = 12;
const FADE_DURATION_MS = 250;
const POSITION_UNSET = -1;
const MAX_OVERLAY_HEIGHT = 400;

/**
 * Format a Note's `updated_at` (ISO 8601 string from the daemon) as a
 * short relative "Xm ago" / "Xh ago" string, so staleness is visible
 * at a glance without the user doing any math.
 */
function relativeTime(isoString) {
    const then = new Date(isoString).getTime();
    const now = Date.now();
    const diffSeconds = Math.max(0, Math.floor((now - then) / 1000));

    if (diffSeconds < 60)
        return 'just now';
    const diffMinutes = Math.floor(diffSeconds / 60);
    if (diffMinutes < 60)
        return `${diffMinutes}m ago`;
    const diffHours = Math.floor(diffMinutes / 60);
    if (diffHours < 24)
        return `${diffHours}h ago`;
    const diffDays = Math.floor(diffHours / 24);
    return `${diffDays}d ago`;
}

function buildNoteRow(note) {
    const row = new St.BoxLayout({
        style_class: 'pulse-note-row',
        vertical: true,
        x_expand: true,
    });

    const textLabel = new St.Label({
        style_class: 'pulse-note-text',
        text: note.text,
    });
    textLabel.clutter_text.line_wrap = true;
    row.add_child(textLabel);

    const propertyKeys = Object.keys(note.properties);
    if (propertyKeys.length > 0) {
        const propertiesText = propertyKeys
            .map(key => `${key}: ${note.properties[key]}`)
            .join('  ·  ');
        const propertiesLabel = new St.Label({
            style_class: 'pulse-note-properties',
            text: propertiesText,
        });
        propertiesLabel.clutter_text.line_wrap = true;
        row.add_child(propertiesLabel);
    }

    const timestampLabel = new St.Label({
        style_class: 'pulse-note-timestamp',
        text: relativeTime(note.updated_at),
    });
    row.add_child(timestampLabel);

    return row;
}

export class PulseOverlay {
    /**
     * @param {Gio.Settings} settings - from Extension.getSettings(),
     *     used to persist dragged position across sessions.
     */
    constructor(settings) {
        this._settings = settings;

        // _container is the outer frame: fixed max height, chrome-
        // registered, draggable, click-handled. _noteList is the inner
        // actor that actually grows with content; _scrollView clips it
        // to _container's height and adds scrolling once it overflows.
        this._container = new St.BoxLayout({
            style_class: 'pulse-overlay',
            vertical: true,
            reactive: true,
            can_focus: true,
            track_hover: true,
        });
        this._container.hide();

        this._scrollView = new St.ScrollView({
            style_class: 'pulse-scroll-view',
            hscrollbar_policy: St.PolicyType.NEVER,
            vscrollbar_policy: St.PolicyType.AUTOMATIC,
            overlay_scrollbars: true,
        });
        // No fixed height set here — render() decides per-update
        // whether the content fits under MAX_OVERLAY_HEIGHT and only
        // clamps the height when it actually overflows, so a short
        // note list stays compact instead of always reserving the cap.

        this._noteList = new St.BoxLayout({
            style_class: 'pulse-note-list',
            vertical: true,
            x_expand: true,
        });

        this._scrollView.add_child(this._noteList);
        this._container.add_child(this._scrollView);

        // Note: affectsInputRegion is a real Mutter param historically,
        // but this GJS/GNOME version rejects it as unrecognized — drop
        // it and rely on the two widely-used params instead.
        Main.layoutManager.addChrome(this._container, {
            affectsStruts: false,
            trackFullscreen: false,
        });

        this._positionOnFirstAllocation();
        this._setupDragging();
    }

    _positionOnFirstAllocation() {
        // A fresh St.BoxLayout reports 0 width/height until its first
        // layout pass, so position after allocation, not at
        // construction time.
        const handlerId = this._container.connect('notify::allocation', () => {
            if (this._container.width === 0)
                return;
            this._applyPosition();
            this._container.disconnect(handlerId);
        });
    }

    _applyPosition() {
        const savedX = this._settings.get_int('overlay-position-x');
        const savedY = this._settings.get_int('overlay-position-y');

        if (savedX !== POSITION_UNSET && savedY !== POSITION_UNSET) {
            this._container.set_position(savedX, savedY);
            return;
        }

        const monitor = Main.layoutManager.primaryMonitor;
        const x = monitor.x + monitor.width - this._container.width - MARGIN_FROM_EDGE;
        const y = monitor.y + MARGIN_FROM_EDGE;
        this._container.set_position(x, y);
    }

    _setupDragging() {
        let dragging = false;
        let dragStartX = 0;
        let dragStartY = 0;
        let actorStartX = 0;
        let actorStartY = 0;

        this._container.connect('button-press-event', (actor, event) => {
            dragging = true;
            [dragStartX, dragStartY] = event.get_coords();
            [actorStartX, actorStartY] = actor.get_position();
            return Clutter.EVENT_STOP;
        });

        this._container.connect('motion-event', (actor, event) => {
            if (!dragging)
                return Clutter.EVENT_PROPAGATE;
            const [x, y] = event.get_coords();
            actor.set_position(
                actorStartX + (x - dragStartX),
                actorStartY + (y - dragStartY),
            );
            return Clutter.EVENT_STOP;
        });

        this._container.connect('button-release-event', (actor) => {
            if (dragging) {
                dragging = false;
                const [x, y] = actor.get_position();
                this._settings.set_int('overlay-position-x', x);
                this._settings.set_int('overlay-position-y', y);
            }
            return Clutter.EVENT_STOP;
        });
    }

    /**
     * Replace the displayed notes. Does not change visibility —
     * callers decide when to show()/hide() separately.
     */
    render(notes) {
        this._noteList.destroy_all_children();

        if (notes.length === 0) {
            const emptyLabel = new St.Label({
                style_class: 'pulse-note-text',
                text: 'No notes yet.',
            });
            this._noteList.add_child(emptyLabel);
        } else {
            for (const note of notes)
                this._noteList.add_child(buildNoteRow(note));
        }

        this._clampHeightToContent();
    }

    /**
     * Only fix the scroll view's height (enabling scrolling) once the
     * note list's natural height actually exceeds the cap. Below the
     * cap, leave the height unset so the overlay shrinks to fit a
     * short list instead of always reserving MAX_OVERLAY_HEIGHT of
     * mostly-empty space.
     */
    _clampHeightToContent() {
        // get_preferred_height needs a width to lay out against; use
        // the note list's current width, falling back to a first-pass
        // default before any real allocation has happened yet.
        const forWidth = this._noteList.width > 0 ? this._noteList.width : -1;
        const [, naturalHeight] = this._noteList.get_preferred_height(forWidth);

        if (naturalHeight > MAX_OVERLAY_HEIGHT)
            this._scrollView.set_height(MAX_OVERLAY_HEIGHT);
        else
            this._scrollView.set_height(-1); // -1 = unset, size to content
    }

    /**
     * Connect a callback to clicks on the overlay itself (used by
     * extension.js to dismiss a manually-shown overlay on click).
     * Returns a handler id for disconnectClick().
     */
    onClick(callback) {
        return this._container.connect('button-press-event', () => {
            callback();
            return Clutter.EVENT_STOP;
        });
    }

    disconnectClick(handlerId) {
        this._container.disconnect(handlerId);
    }

    get isVisible() {
        return this._container.visible;
    }

    show() {
        this._container.show();
        this._container.opacity = 0;
        this._container.ease({
            opacity: 255,
            duration: FADE_DURATION_MS,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
        });
    }

    /** Fade out, then hide (so it stops taking input once invisible). */
    hide() {
        this._container.ease({
            opacity: 0,
            duration: FADE_DURATION_MS,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
            onComplete: () => this._container.hide(),
        });
    }

    destroy() {
        Main.layoutManager.removeChrome(this._container);
        this._container.destroy();
        this._container = null;
    }
}
