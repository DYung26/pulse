// Pulse GNOME Shell extension.
//
// Trigger-behavior stage: wires up the periodic timer (auto-fades),
// the manual keybind (stays open until dismissed), and dismiss paths
// for the manually-shown overlay — pressing the keybind again, Escape,
// or clicking the overlay itself. True "click anywhere else on the
// desktop" isn't reliably possible in GNOME Shell (clicks landing on
// regular app windows never reach Shell's own event system), so it's
// deliberately not attempted — see docs/_local/checklist.md.

import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PulseClient from './pulseClient.js';
import {PulseOverlay} from './overlay.js';

const DEFAULT_INTERVAL_SECONDS = 300;
const AUTO_FADE_AFTER_MS = 15000;
const KEYBINDING_NAME = 'show-overlay-keybinding';

export default class PulseExtension extends Extension {
    enable() {
        console.log(`[pulse] enabled (${this.metadata.name} ${this.metadata.uuid})`);

        this._settings = this.getSettings();
        this._overlay = new PulseOverlay(this._settings);
        this._manuallyShown = false;
        this._autoFadeTimeoutId = null;
        this._periodicTimeoutId = null;
        this._escapeKeyHandlerId = null;
        this._overlayClickHandlerId = null;

        this._registerKeybinding();
        this._startPeriodicTimer();
    }

    disable() {
        console.log('[pulse] disabled');

        Main.wm.removeKeybinding(KEYBINDING_NAME);

        if (this._periodicTimeoutId) {
            GLib.source_remove(this._periodicTimeoutId);
            this._periodicTimeoutId = null;
        }
        this._cancelAutoFade();
        this._disconnectDismissHandlers();

        if (this._overlay) {
            this._overlay.destroy();
            this._overlay = null;
        }
        this._settings = null;
    }

    _registerKeybinding() {
        Main.wm.addKeybinding(
            KEYBINDING_NAME,
            this._settings,
            Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.ALL,
            () => this._onKeybindingPressed(),
        );
    }

    _onKeybindingPressed() {
        if (this._manuallyShown) {
            this._dismissManualOverlay();
            return;
        }
        this._showManually();
    }

    async _showManually() {
        // Cancel any pending auto-fade from a periodic show, so a
        // manual summon always behaves as "stays open" even if it
        // happens to land right after a periodic pop-up.
        this._cancelAutoFade();

        const notes = await this._fetchNotes();
        if (notes === null)
            return;

        this._overlay.render(notes);
        this._overlay.show();
        this._manuallyShown = true;
        this._connectDismissHandlers();
    }

    _dismissManualOverlay() {
        this._manuallyShown = false;
        this._disconnectDismissHandlers();
        this._overlay.hide();
    }

    _connectDismissHandlers() {
        this._overlayClickHandlerId = this._overlay.onClick(() => {
            this._dismissManualOverlay();
        });

        this._escapeKeyHandlerId = global.stage.connect('key-press-event', (actor, event) => {
            if (event.get_key_symbol() === Clutter.KEY_Escape) {
                this._dismissManualOverlay();
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        });
    }

    _disconnectDismissHandlers() {
        if (this._escapeKeyHandlerId) {
            global.stage.disconnect(this._escapeKeyHandlerId);
            this._escapeKeyHandlerId = null;
        }
        if (this._overlayClickHandlerId && this._overlay) {
            this._overlay.disconnectClick(this._overlayClickHandlerId);
            this._overlayClickHandlerId = null;
        }
    }

    async _startPeriodicTimer() {
        const intervalSeconds = await this._fetchInterval();
        this._periodicTimeoutId = GLib.timeout_add_seconds(
            GLib.PRIORITY_DEFAULT,
            intervalSeconds,
            () => {
                this._showPeriodically();
                return GLib.SOURCE_CONTINUE;
            },
        );
    }

    async _showPeriodically() {
        // If the person is actively looking at a manually-summoned
        // overlay, don't interrupt it with a periodic auto-fading one.
        if (this._manuallyShown)
            return;

        const notes = await this._fetchNotes();
        if (notes === null)
            return;

        this._overlay.render(notes);
        this._overlay.show();
        this._scheduleAutoFade();
    }

    _scheduleAutoFade() {
        this._cancelAutoFade();
        this._autoFadeTimeoutId = GLib.timeout_add(
            GLib.PRIORITY_DEFAULT,
            AUTO_FADE_AFTER_MS,
            () => {
                this._overlay.hide();
                this._autoFadeTimeoutId = null;
                return GLib.SOURCE_REMOVE;
            },
        );
    }

    _cancelAutoFade() {
        if (this._autoFadeTimeoutId) {
            GLib.source_remove(this._autoFadeTimeoutId);
            this._autoFadeTimeoutId = null;
        }
    }

    /** Returns the note array, or null if the daemon couldn't be reached. */
    async _fetchNotes() {
        try {
            const response = await PulseClient.send({action: 'list_notes'});
            if (response.ok)
                return response.data;
            console.log(`[pulse] daemon responded with an error: ${response.error}`);
            return null;
        } catch (error) {
            console.log(`[pulse] could not reach daemon: ${error.message}`);
            return null;
        }
    }

    async _fetchInterval() {
        try {
            const response = await PulseClient.send({action: 'get_interval'});
            if (response.ok)
                return response.data.seconds;
        } catch (error) {
            console.log(`[pulse] could not fetch interval, using default: ${error.message}`);
        }
        return DEFAULT_INTERVAL_SECONDS;
    }
}
