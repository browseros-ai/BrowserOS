//! The `Input` domain as neo-runtime uses it: the mouse, key, touch and drag events of an input
//! plan, a fill's typed text, and drag interception (`chromium_bridge/mouse_and_touch_input.rs`,
//! `keyboard_input.rs`). Every command here is a write ([`Write`]) or a release ([`Release`]),
//! bound to the session's socket incarnation.
//!
//! Neo: browseros-cdp's generated params and events, made from BrowserOS's pinned protocol,
//! under each method's name, written in their protocol order; a local type only where its doc
//! says why.

use browseros_cdp::input as generated;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Ignored, Release, Write, command, event};

pub(crate) use generated::{
    DispatchMouseEventParams as DispatchMouseEvent, SetInterceptDragsParams as SetInterceptDrags,
};

/// `Input.dragIntercepted`: the drag Chromium handed over, kept as JSON. Local: the drag events
/// that carry the drag on write its data back unchanged ([`DispatchDragEvent`]), where the
/// generated `DragData` would re-encode it.
#[derive(Deserialize)]
pub(crate) struct DragIntercepted {
    pub(crate) data: Value,
}

/// One `Input.dispatchDragEvent`, as Playwright's `DragManager` builds it: the event, its point,
/// the held modifiers (none for a cancel), and the drag's data. Local: the data is the JSON the
/// tab's drag state holds (`InputDevices::drag`), written back unchanged, where the generated
/// `DragData` would re-encode it.
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

/// One `Input.dispatchKeyEvent`, serialized exactly as `crInput.ts` builds it. Local: the
/// generated params would write their protocol order and leave out empty fields, where key
/// events keep Playwright's bytes.
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
/// point, or an end with none. Local, as the key events are ([`KeyEvent`]): a touch write keeps
/// the bytes Neo writes, keys in this order, where the generated params would write the
/// protocol's order.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DispatchTouchEvent {
    pub(crate) modifiers: u8,
    pub(crate) touch_points: Vec<TouchPoint>,
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
}

/// A touch point of [`DispatchTouchEvent`]: where it touches, nothing else.
#[derive(Serialize)]
pub(crate) struct TouchPoint {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

/// A key's release: one `Input.dispatchKeyEvent` `keyUp`, byte for byte as [`KeyUp`] writes it.
/// A release ([`Release`]), so a run's end may write it with no call to record it. Local: its one
/// constructor makes only a `keyUp`, whatever `KeyUp` it is given.
#[derive(Serialize)]
#[serde(transparent)]
pub(crate) struct ReleaseKey(KeyUp);

impl ReleaseKey {
    pub(crate) fn new(up: &KeyUp) -> Self {
        Self(KeyUp {
            kind: "keyUp",
            ..*up
        })
    }
}

/// A button's release: one `Input.dispatchMouseEvent` `mouseReleased` at a point, with the
/// buttons the tab still holds after it, as [`DispatchMouseEvent`] writes it. A release
/// ([`Release`]). Local: its one constructor makes only a `mouseReleased`.
#[derive(Serialize)]
#[serde(transparent)]
pub(crate) struct ReleaseMouse(DispatchMouseEvent);

impl ReleaseMouse {
    pub(crate) fn new(
        (x, y): (f64, f64),
        button: &str,
        buttons: i64,
        click_count: i64,
        modifiers: i64,
    ) -> Self {
        Self(DispatchMouseEvent {
            r#type: "mouseReleased".to_string(),
            x,
            y,
            modifiers: Some(modifiers),
            button: Some(button.to_string()),
            buttons: Some(buttons),
            click_count: Some(click_count),
            ..DispatchMouseEvent::default()
        })
    }
}

/// A drag's cancel: one `Input.dispatchDragEvent` `dragCancel` at the drag's last point, with no
/// data and no modifiers, as Playwright's `DragManager.cancelDrag` writes it. A release
/// ([`Release`]): a run's end cancels a drag its released button held. Local: its one constructor
/// makes only a `dragCancel`, its fields in [`DispatchDragEvent`]'s order.
#[derive(Serialize)]
pub(crate) struct CancelDrag {
    data: Value,
    #[serde(rename = "type")]
    kind: &'static str,
    x: f64,
    y: f64,
}

impl CancelDrag {
    /// Playwright: `server/chromium/crDragDrop.ts` `DragManager.cancelDrag`: `dragOperationsMask: 0xFFFF,`
    const OPERATIONS_ALL: i64 = 0xFFFF;

    pub(crate) fn new((x, y): (f64, f64)) -> Self {
        Self {
            data: serde_json::json!({ "items": [], "dragOperationsMask": Self::OPERATIONS_ALL }),
            kind: "dragCancel",
            x,
            y,
        }
    }
}

/// `Input.insertText {text}`. Local: the generated params derive `Debug`, and the text may be a
/// secret that a fill types.
#[derive(Serialize)]
pub(crate) struct InsertText<'a> {
    pub(crate) text: &'a str,
}

command! {
    // Playwright: `server/chromium/crDragDrop.ts` `DragManager.interceptDragCausedByMove`: `await progress.race(this._crPage._mainFrameSession._client.send('Input.dispatchDragEvent', {`
    DispatchDragEvent<'_> => "Input.dispatchDragEvent", Write, Ignored;
    // Playwright: `server/chromium/crDragDrop.ts` `DragManager.cancelDrag`: `type: 'dragCancel',`
    CancelDrag => "Input.dispatchDragEvent", Release, Ignored;
    // Playwright: `server/chromium/crInput.ts` `RawKeyboardImpl.keydown`: `await progress.race(this._client.send('Input.dispatchKeyEvent', {`
    KeyEvent => "Input.dispatchKeyEvent", Write, Ignored;
    // Playwright: `server/chromium/crInput.ts` `RawKeyboardImpl.keyup`: `type: 'keyUp',`
    ReleaseKey => "Input.dispatchKeyEvent", Release, Ignored;
    // Playwright: `server/chromium/crInput.ts` `RawMouseImpl.down`: `await progress.race(this._client.send('Input.dispatchMouseEvent', {`
    DispatchMouseEvent => "Input.dispatchMouseEvent", Write, Ignored;
    // Playwright: `server/chromium/crInput.ts` `RawMouseImpl.up`: `type: 'mouseReleased',`
    ReleaseMouse => "Input.dispatchMouseEvent", Release, Ignored;
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A command's bytes as `cdp::Input` and `cdp::ReleaseInput` write them.
    fn bytes(command: &impl Serialize) -> String {
        serde_json::to_string(command).expect("the command serializes")
    }

    #[test]
    fn tests_that_a_release_writes_the_bytes_of_the_write_it_stands_for() {
        let up = KeyUp {
            kind: "keyUp",
            modifiers: 8,
            key: "Shift",
            windows_virtual_key_code: 16,
            code: "ShiftLeft",
            location: 1,
            is_keypad: false,
        };
        assert_eq!(bytes(&ReleaseKey::new(&up)), bytes(&KeyEvent::Up(up)));
        let released = DispatchMouseEvent {
            r#type: "mouseReleased".to_string(),
            x: 10.5,
            y: 20.0,
            modifiers: Some(2),
            button: Some("left".to_string()),
            buttons: Some(0),
            click_count: Some(1),
            ..DispatchMouseEvent::default()
        };
        assert_eq!(
            bytes(&ReleaseMouse::new((10.5, 20.0), "left", 0, 1, 2)),
            bytes(&released)
        );
        let data = json!({ "items": [], "dragOperationsMask": 0xFFFF });
        let cancel = DispatchDragEvent {
            data: &data,
            modifiers: None,
            kind: "dragCancel",
            x: 3.0,
            y: 4.0,
        };
        assert_eq!(bytes(&CancelDrag::new((3.0, 4.0))), bytes(&cancel));
    }
}
