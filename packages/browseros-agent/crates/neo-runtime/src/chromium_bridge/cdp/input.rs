//! The `Input` domain as neo-runtime uses it: the mouse, key, touch and drag events of an input
//! plan, a fill's typed text, and drag interception (`chromium_bridge/mouse_and_touch_input.rs`,
//! `keyboard_input.rs`). Every command here is a write ([`Write`]), bound to the session's
//! socket incarnation.
//!
//! Neo: key events are local structs in Playwright's field order, written byte for byte as
//! `crInput.ts` builds them; mouse events use browseros-cdp's generated params, written in their
//! protocol order; touch and drag events are local structs whose fields are in alphabetical
//! order, the order their JSON has always gone out in.

use browseros_cdp::input as generated;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Ignored, Write, command, event};

pub(crate) use generated::DispatchMouseEventParams as DispatchMouseEvent;

/// One `Input.dispatchDragEvent`, as Playwright's `DragManager` builds it: the event, the
/// intercepted drag's data, its point, and the held modifiers (none for a cancel).
#[derive(Serialize)]
pub(crate) struct DispatchDragEvent<'a> {
    pub(crate) data: &'a Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) modifiers: Option<i64>,
    #[serde(rename = "type")]
    pub(crate) kind: &'a str,
    pub(crate) x: f64,
    pub(crate) y: f64,
}

/// One `Input.dispatchKeyEvent`, serialized exactly as `crInput.ts` builds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum KeyEvent {
    Down(KeyDown),
    Up(KeyUp),
}

/// `RawKeyboardImpl.keydown`'s params, in its field order. Every field is always present:
/// `commands` is empty off mac and `text` empty for a key that types nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KeyDown {
    /// `keyDown` when the key types text, else `rawKeyDown`.
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) modifiers: u8,
    pub(crate) windows_virtual_key_code: u32,
    pub(crate) code: &'static str,
    pub(crate) commands: Vec<&'static str>,
    pub(crate) key: &'static str,
    pub(crate) text: &'static str,
    pub(crate) unmodified_text: &'static str,
    pub(crate) auto_repeat: bool,
    pub(crate) location: u32,
    pub(crate) is_keypad: bool,
}

/// `RawKeyboardImpl.keyup`'s params, in its field order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KeyUp {
    /// Always `keyUp`.
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) modifiers: u8,
    pub(crate) key: &'static str,
    pub(crate) windows_virtual_key_code: u32,
    pub(crate) code: &'static str,
    pub(crate) location: u32,
    pub(crate) is_keypad: bool,
}

/// One `Input.dispatchTouchEvent` of Playwright's `RawTouchscreenImpl.tap`: a start with one
/// point, or an end with none.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DispatchTouchEvent {
    pub(crate) modifiers: u8,
    pub(crate) touch_points: Vec<TouchPoint>,
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
}

#[derive(Serialize)]
pub(crate) struct TouchPoint {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

/// `Input.insertText {text}`. No `Debug`: the text may be a secret that a fill types.
#[derive(Serialize)]
pub(crate) struct InsertText<'a> {
    pub(crate) text: &'a str,
}

/// `Input.setInterceptDrags`, on for one move of a drag and off again.
#[derive(Serialize)]
pub(crate) struct SetInterceptDrags {
    pub(crate) enabled: bool,
}

/// `Input.dragIntercepted`: the drag Chromium handed over, kept as JSON for the drag events
/// that carry it on.
#[derive(Deserialize)]
pub(crate) struct DragIntercepted {
    #[serde(default)]
    pub(crate) data: Value,
}

command! {
    // Playwright: `server/chromium/crDragDrop.ts` `DragManager.interceptDragCausedByMove`: `await progress.race(this._crPage._mainFrameSession._client.send('Input.dispatchDragEvent', {`
    DispatchDragEvent<'_> => "Input.dispatchDragEvent", Write, Ignored;
    // Playwright: `server/chromium/crInput.ts` `RawKeyboardImpl.keydown`: `await progress.race(this._client.send('Input.dispatchKeyEvent', {`
    KeyEvent => "Input.dispatchKeyEvent", Write, Ignored;
    // Playwright: `server/chromium/crInput.ts` `RawMouseImpl.down`: `await progress.race(this._client.send('Input.dispatchMouseEvent', {`
    DispatchMouseEvent => "Input.dispatchMouseEvent", Write, Ignored;
    // Playwright: `server/chromium/crInput.ts` `RawTouchscreenImpl.tap`: `this._client.send('Input.dispatchTouchEvent', {`
    DispatchTouchEvent => "Input.dispatchTouchEvent", Write, Ignored;
    // Playwright: `server/chromium/crInput.ts` `RawKeyboardImpl.sendText`: `await progress.race(this._client.send('Input.insertText', { text }));`
    InsertText<'_> => "Input.insertText", Write, Ignored;
    // Playwright: `server/chromium/crDragDrop.ts` `DragManager.interceptDragCausedByMove`: `await progress.race(client.send('Input.setInterceptDrags', { enabled: true }));`
    SetInterceptDrags => "Input.setInterceptDrags", Write, Ignored;
}

event! {
    // Playwright: `server/chromium/crDragDrop.ts` `DragManager.interceptDragCausedByMove`: `client.on('Input.dragIntercepted', onDragIntercepted!);`
    DragIntercepted => "Input.dragIntercepted";
}
