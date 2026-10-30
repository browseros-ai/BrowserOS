// Text clipboard integration for Neo's noVNC viewer. Loading this module handles
// desktop shortcuts; an existing touch Paste button may call pasteFromClipboard.
// Clipboard access stays in the owner's browser and requires an explicit action.
import UI from './ui.js';
import KeyTable from '../core/input/keysym.js';
import { isIOS, isMac } from '../core/util/browser.js';

let copyRequest = null;
const heldControls = new Map();
const commandKey = () => isMac() || isIOS();
const active = (rfb = UI.rfb) => UI.connected && rfb === UI.rfb && !rfb.viewOnly;
const screen = (target) => target?.tagName === 'CANVAS' ||
    ((target === document || target === document.body) && document.activeElement?.tagName === 'CANVAS');

function controlKey(rfb, key) {
    if (commandKey()) {
        // Release the legacy Alt mapping and both physical Command keys: a
        // QEMU-capable server uses scancodes even when noVNC remaps the keysym.
        rfb.sendKey(KeyTable.XK_Alt_L, 'AltLeft', false);
        rfb.sendKey(KeyTable.XK_Super_L, 'MetaLeft', false);
        rfb.sendKey(KeyTable.XK_Super_R, 'MetaRight', false);
    }
    // Preserve a physical Ctrl held across several shortcuts, including right
    // Ctrl. A synthetic left-Ctrl release would otherwise break the next chord.
    const held = [...heldControls].find(([, connection]) => connection === rfb);
    const code = held?.[0] || 'ControlLeft';
    const symbol = code === 'ControlRight' ? KeyTable.XK_Control_R : KeyTable.XK_Control_L;
    rfb.sendKey(symbol, code, true);
    rfb.sendKey(key.charCodeAt(0), 'Key' + key.toUpperCase());
    if (!held) rfb.sendKey(symbol, code, false);
}

function paste(text, rfb = UI.rfb) {
    if (!text || !active(rfb)) return;
    copyRequest = null;
    // Keep clipboard ownership and Ctrl+V ordered on the same RFB connection.
    // TigerVNC supplies the text when the Linux application requests selection.
    rfb.clipboardPasteFrom(text);
    controlKey(rfb, 'v');
}

function clipboardPanel() {
    // Moving focus while a shortcut is held would strand keyup on this textarea.
    // Deliver releases through noVNC's canvas listener so its tracked keys and
    // the remote modifiers are cleared together, without private RFB state.
    const canvas = document.querySelector('#noVNC_container canvas');
    for (const [key, codes] of [
        ['Control', ['ControlLeft', 'ControlRight']],
        ['Meta', ['MetaLeft', 'MetaRight']],
        ['c', ['KeyC']], ['x', ['KeyX']], ['v', ['KeyV']],
    ]) {
        for (const code of codes) canvas?.dispatchEvent(new KeyboardEvent('keyup', {
            key, code, bubbles: true,
        }));
    }
    UI.openClipboardPanel();
    const field = document.getElementById('noVNC_clipboard_text');
    field.focus();
    field.select();
}

export async function pasteFromClipboard() {
    const rfb = UI.rfb;
    if (!active(rfb)) return;
    try {
        const text = await navigator.clipboard.readText();
        paste(text, rfb); // A permission prompt may outlive this connection.
    } catch {
        if (active(rfb)) clipboardPanel();
    }
}

// Window capture precedes noVNC's canvas listener. Let the browser's native paste
// event deliver its text without requiring clipboard-read permission. Leave
// viewer form fields and noVNC's phone keyboard input to their native handlers.
for (const type of ['keydown', 'keyup']) {
    window.addEventListener(type, (event) => {
        if (type === 'keydown' && !event.repeat) copyRequest = null;
        if (event.key === 'Control') {
            if (type === 'keyup') heldControls.delete(event.code);
            else if (active() && screen(event.target)) heldControls.set(event.code, UI.rfb);
            return; // noVNC still owns the physical modifier events.
        }
        if (!active() || !screen(event.target) || event.altKey || event.shiftKey) return;
        const key = event.key?.toLowerCase();
        const command = commandKey() && event.metaKey;
        if (!(command || event.ctrlKey) || !['c', 'x', 'v'].includes(key)) return;
        if (key === 'v') {
            event.stopPropagation();
            return;
        }
        // Ctrl+C/X already have the right Linux semantics; keep noVNC's own
        // modifier lifecycle. Only Mac Command needs translation.
        if (command) {
            event.preventDefault();
            event.stopPropagation();
        }
        if (type === 'keydown' && !event.repeat) {
            // RFB does not correlate clipboard updates with keypresses. Only a
            // matching, previously received value may complete this shortcut;
            // new/unattributable text needs an explicit copy from the panel.
            copyRequest = { rfb: UI.rfb, until: performance.now() + 5000,
                text: document.getElementById('noVNC_clipboard_text').value };
            if (command) controlKey(UI.rfb, key);
        }
    }, true);
}

document.addEventListener('paste', (event) => {
    if (!active() || !screen(event.target) || !event.clipboardData?.types.includes('text/plain')) return;
    event.preventDefault();
    paste(event.clipboardData.getData('text/plain'));
});
window.addEventListener('blur', () => { copyRequest = null; heldControls.clear(); });
for (const type of ['pointerdown', 'wheel']) {
    window.addEventListener(type, () => { copyRequest = null; }, { capture: true, passive: true });
}

// Preserve the native Clipboard panel and future reconnects: connect() registers
// UI.clipboardReceive for every new RFB object. EventTarget invokes it with that
// RFB as `this`, so an old connection cannot complete a newer copy request.
// The stock receive/send callbacks log the first 40 clipboard characters in
// debug mode. Keep their panel behavior without sending owner text to console.
const receive = UI.clipboardReceive;
UI.clipboardReceive = async function(event) {
    document.getElementById('noVNC_clipboard_text').value = event.detail.text;
    if (copyRequest?.rfb !== this || performance.now() > copyRequest.until ||
        !active(this) || !document.hasFocus() || document.hidden) return;
    const expected = copyRequest.text;
    copyRequest = null;
    if (event.detail.text !== expected) {
        clipboardPanel();
        return;
    }
    try {
        await navigator.clipboard.writeText(event.detail.text);
    } catch {
        if (active(this)) clipboardPanel();
    }
};
if (UI.rfb) {
    UI.rfb.removeEventListener('clipboard', receive);
    UI.rfb.addEventListener('clipboard', UI.clipboardReceive);
}
const send = UI.clipboardSend;
UI.clipboardSend = function() {
    copyRequest = null;
    if (active()) UI.rfb.clipboardPasteFrom(document.getElementById('noVNC_clipboard_text').value);
};
const field = document.getElementById('noVNC_clipboard_text');
field.removeEventListener('change', send);
field.addEventListener('change', UI.clipboardSend);
