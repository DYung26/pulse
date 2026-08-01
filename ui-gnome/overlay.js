// Renders the note-list overlay: a small St.Widget added to GNOME
// Shell's chrome layer (Main.layoutManager.addChrome), positioned in
// the top-right corner (or a persisted dragged position), showing
// each note's text, properties, and last-updated time.
//
// Owns rendering, drag positioning (persisted via GSettings), the
// show()/hide() fade transitions, and the add/edit forms (list view
// vs. add-mode vs. inline-edit-mode). Timer/keybind trigger logic and
// dismiss-on-click/Escape live in extension.js, which calls into this
// through onClick()/disconnectClick() rather than reaching into
// internals directly. CRUD submission itself also lives in
// extension.js (it owns the daemon connection); this module only
// collects input and reports it via onAddNote()/onUpdateNote().

import St from 'gi://St';
import Clutter from 'gi://Clutter';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const MARGIN_FROM_EDGE = 12;
const FADE_DURATION_MS = 250;
const POSITION_UNSET = -1;
const MAX_OVERLAY_HEIGHT = 400;
const ADD_NOTE_KEY_SYMBOL = Clutter.KEY_n;

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

/**
 * Parse the `key=value key2=value2` properties field syntax (matches
 * the CLI's `--prop` convention). Tokens without an `=` are ignored
 * rather than treated as an error, so a stray space or trailing token
 * while typing doesn't block submission.
 */
function parsePropertiesText(text) {
    const properties = {};
    for (const token of text.trim().split(/\s+/)) {
        if (token === '')
            continue;
        const separatorIndex = token.indexOf('=');
        if (separatorIndex <= 0)
            continue;
        const key = token.slice(0, separatorIndex);
        const value = token.slice(separatorIndex + 1);
        properties[key] = value;
    }
    return properties;
}

function propertiesToText(properties) {
    return Object.entries(properties)
        .map(([key, value]) => `${key}=${value}`)
        .join(' ');
}

function buildNoteRow(note, {onSelect} = {}) {
    const row = new St.BoxLayout({
        style_class: 'pulse-note-row',
        vertical: true,
        x_expand: true,
        reactive: Boolean(onSelect),
        track_hover: Boolean(onSelect),
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

    if (onSelect) {
        row.connect('button-press-event', () => {
            onSelect(note);
            return Clutter.EVENT_STOP;
        });
    }

    return row;
}

/**
 * Build the shared text-entry + properties-entry + hint-label form
 * used by both add-mode and inline-edit-mode. Callers wire submit/
 * cancel themselves via the returned entries' key-press handling,
 * since add and edit differ in what Enter/Escape actually do.
 */
function buildEntryForm({textValue = '', propertiesValue = '', textHint = 'Note text'} = {}) {
    const form = new St.BoxLayout({
        style_class: 'pulse-note-form',
        vertical: true,
        x_expand: true,
    });

    const textEntry = new St.Entry({
        style_class: 'pulse-note-form-entry',
        hint_text: textHint,
        text: textValue,
        x_expand: true,
        can_focus: true,
    });
    form.add_child(textEntry);

    const propertiesEntry = new St.Entry({
        style_class: 'pulse-note-form-entry',
        hint_text: 'key=value key2=value2',
        text: propertiesValue,
        x_expand: true,
        can_focus: true,
    });
    form.add_child(propertiesEntry);

    return {form, textEntry, propertiesEntry};
}

export class PulseOverlay {
    /**
     * @param {Gio.Settings} settings - from Extension.getSettings(),
     *     used to persist dragged position across sessions.
     * @param {Object} callbacks
     * @param {(text: string, properties: Object) => void} callbacks.onAddNote
     * @param {(id: string, text: string, properties: Object) => void} callbacks.onUpdateNote
     */
    constructor(settings, {onAddNote, onUpdateNote} = {}) {
        this._settings = settings;
        this._onAddNote = onAddNote ?? (() => {});
        this._onUpdateNote = onUpdateNote ?? (() => {});
        this._notes = [];
        this._modalActor = null;

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

        this._addButton = new St.Button({
            style_class: 'pulse-add-button',
            label: '+ Add note',
            x_expand: true,
        });
        this._addButton.connect('clicked', () => this.enterAddMode());
        this._container.add_child(this._addButton);

        // Note: affectsInputRegion is a real Mutter param historically,
        // but this GJS/GNOME version rejects it as unrecognized — drop
        // it and rely on the two widely-used params instead.
        Main.layoutManager.addChrome(this._container, {
            affectsStruts: false,
            trackFullscreen: false,
        });

        this._positionOnFirstAllocation();
        this._setupDragging();
        this._setupAddKeybind();
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

    _setupAddKeybind() {
        this._container.connect('key-press-event', (actor, event) => {
            if (this._isFormOpen())
                return Clutter.EVENT_PROPAGATE;
            if (event.get_key_symbol() !== ADD_NOTE_KEY_SYMBOL)
                return Clutter.EVENT_PROPAGATE;
            this.enterAddMode();
            return Clutter.EVENT_STOP;
        });
    }

    _isFormOpen() {
        return this._modalActor !== null;
    }

    /**
     * Replace the displayed notes. Does not change visibility —
     * callers decide when to show()/hide() separately. Closes any
     * open add/edit form, since the underlying note list it was
     * showing is being replaced.
     */
    render(notes) {
        this._notes = notes;
        this._closeForm();
        this._renderList();
    }

    _renderList() {
        this._noteList.destroy_all_children();
        this._addButton.show();

        if (this._notes.length === 0) {
            const emptyLabel = new St.Label({
                style_class: 'pulse-note-text',
                text: 'No notes yet.',
            });
            this._noteList.add_child(emptyLabel);
        } else {
            for (const note of this._notes) {
                this._noteList.add_child(buildNoteRow(note, {
                    onSelect: selected => this.enterEditMode(selected),
                }));
            }
        }

        this._clampHeightToContent();
    }

    /**
     * Swap the list view for the add-note form. Grabs modal focus via
     * Main.pushModal so the entries receive keyboard input the same
     * way GNOME's own modal dialogs and Overview search do.
     */
    enterAddMode() {
        if (this._isFormOpen())
            return;

        this._noteList.destroy_all_children();
        this._addButton.hide();

        const {form, textEntry, propertiesEntry} = buildEntryForm({
            textHint: 'Note text (Enter to add, Escape to cancel)',
        });

        const submit = () => this._submitAdd(textEntry, propertiesEntry);
        const cancel = () => {
            this._closeForm();
            this._renderList();
        };
        this._connectFormSubmitCancel(textEntry, propertiesEntry, submit, cancel);

        this._noteList.add_child(form);
        this._clampHeightToContent();
        this._openModal(textEntry);
    }

    _submitAdd(textEntry, propertiesEntry) {
        const text = textEntry.get_text().trim();
        if (text === '')
            return;
        const properties = parsePropertiesText(propertiesEntry.get_text());
        this._closeForm();
        this._onAddNote(text, properties);
    }

    /**
     * Swap a single note row in place for an inline edit form,
     * pre-filled with that note's current text/properties.
     */
    enterEditMode(note) {
        if (this._isFormOpen())
            return;

        this._noteList.destroy_all_children();
        this._addButton.hide();

        const {form, textEntry, propertiesEntry} = buildEntryForm({
            textValue: note.text,
            propertiesValue: propertiesToText(note.properties),
            textHint: 'Note text (Enter to save, Escape to cancel)',
        });

        const submit = () => this._submitEdit(note.id, textEntry, propertiesEntry);
        const cancel = () => {
            this._closeForm();
            this._renderList();
        };
        this._connectFormSubmitCancel(textEntry, propertiesEntry, submit, cancel);

        this._noteList.add_child(form);
        this._clampHeightToContent();
        this._openModal(textEntry);
    }

    /** Wires Enter/Escape on both form entries to shared submit/cancel callbacks. */
    _connectFormSubmitCancel(textEntry, propertiesEntry, submit, cancel) {
        for (const entry of [textEntry, propertiesEntry]) {
            entry.clutter_text.connect('key-press-event', (actor, event) => {
                if (event.get_key_symbol() === Clutter.KEY_Return) {
                    submit();
                    return Clutter.EVENT_STOP;
                }
                if (event.get_key_symbol() === Clutter.KEY_Escape) {
                    cancel();
                    return Clutter.EVENT_STOP;
                }
                return Clutter.EVENT_PROPAGATE;
            });
        }
    }

    _submitEdit(id, textEntry, propertiesEntry) {
        const text = textEntry.get_text().trim();
        if (text === '')
            return;
        const properties = parsePropertiesText(propertiesEntry.get_text());
        this._closeForm();
        this._onUpdateNote(id, text, properties);
    }

    _openModal(focusActor) {
        const grab = Main.pushModal(focusActor);
        this._modalActor = focusActor;
        this._modalGrab = grab;
        focusActor.grab_key_focus();
    }

    _closeForm() {
        if (!this._isFormOpen())
            return;
        Main.popModal(this._modalGrab ?? this._modalActor);
        this._modalActor = null;
        this._modalGrab = null;
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
     * Returns a handler id for disconnectClick(). Ignored while an
     * add/edit form is open, so a click inside a form field doesn't
     * dismiss the whole overlay out from under the user.
     */
    onClick(callback) {
        return this._container.connect('button-press-event', () => {
            if (this._isFormOpen())
                return Clutter.EVENT_PROPAGATE;
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

    get isFormOpen() {
        return this._isFormOpen();
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
        this._closeForm();
        this._container.ease({
            opacity: 0,
            duration: FADE_DURATION_MS,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
            onComplete: () => this._container.hide(),
        });
    }

    destroy() {
        this._closeForm();
        Main.layoutManager.removeChrome(this._container);
        this._container.destroy();
        this._container = null;
    }
}
