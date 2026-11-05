//! A page's keyboard and mouse, and the bursts of CDP input they turn Playwright calls into.
//!
//! ```text
//!  keyboard.press('Shift+A')   Keyboard::press   [key down Shift, key down A, key up A, key up Shift]
//!  mouse.click(x, y, {delay})  Mouse::click      [moved, pressed, Wait(delay), released]
//!
//!  the op's leaf, holding the page's input lock for the whole burst:
//!    Wait(d)          OpScope::sleep(d), on Rust's clock
//!    Dispatch(event)  event.method() with event.params(pointer scale) on the page's root session
//! ```
//!
//! A call checks its arguments, then advances the device state for its whole burst and returns
//! the steps; nothing here sends. When the leaf stops part-way (cancelled, session lost), the
//! state is where the finished burst would have left it, so a cancelled `press('Shift+A')` does
//! not leave Shift held for the next call, as it would upstream.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the keyboard, mouse and pointer-action leaves send these bursts once they land"
    )
)]

use std::time::Duration;

use serde_json::{Value, json};

use super::keyboard::{KeyEvent, Keyboard, Platform};
use super::mouse::{Mouse, MouseEvent};

/// What Playwright remembers about one page's input between calls: held keys and modifiers,
/// and the mouse's position and held buttons. The page's state keeps it behind the page's
/// input lock, held for a whole burst, so two ops' events never interleave and each burst
/// starts from the state the last one left.
#[derive(Debug)]
pub(crate) struct InputDevices {
    /// Its modifiers also go on every mouse event.
    pub(crate) keyboard: Keyboard,
    pub(crate) mouse: Mouse,
}

impl InputDevices {
    /// A page as upstream starts it: nothing held, the mouse at (0, 0).
    pub(crate) fn new(platform: Platform) -> Self {
        Self {
            keyboard: Keyboard::new(platform),
            mouse: Mouse::default(),
        }
    }
}

/// One step of a burst, in the order Playwright takes them.
pub(crate) enum InputStep {
    /// Send this event. Steps go out one after another, never reordered.
    Dispatch(InputEvent),
    /// Playwright's `delay` option: wait this long before the next step.
    Wait(Duration),
}

/// One trusted input event for the page's root session. It has no `Debug`, because typed text
/// and keys can be field values (sdk-style P4).
pub(crate) enum InputEvent {
    /// `Input.dispatchKeyEvent`.
    Key(KeyEvent),
    /// `Input.insertText`: text without key events, for characters the US layout lacks.
    InsertText(String),
    /// `Input.dispatchMouseEvent`.
    Mouse(MouseEvent),
}

impl InputEvent {
    /// The CDP method.
    pub(crate) fn method(&self) -> &'static str {
        match self {
            Self::Key(_) => "Input.dispatchKeyEvent",
            Self::InsertText(_) => "Input.insertText",
            Self::Mouse(_) => "Input.dispatchMouseEvent",
        }
    }

    /// The CDP params, as Playwright builds them (server/chromium/crInput.ts). Mouse
    /// coordinates are page CSS pixels times `pointer_scale`, core's factor for the page's
    /// root session (`BrowserView::pointer_scale`, the mobile preview's zoom), so the event
    /// lands where core's own input would. Key and text events have no coordinates.
    pub(crate) fn params(&self, pointer_scale: f64) -> Value {
        match self {
            Self::Key(event) => event.params(),
            Self::InsertText(text) => json!({ "text": text }),
            Self::Mouse(event) => event.params(pointer_scale),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::error::Error;

    use serde_json::{Value, json};

    use super::{InputDevices, InputStep};
    use crate::input::keyboard::Platform;
    use crate::input::mouse::{ClickOptions, Point};

    /// The messages a burst sends, as the goldens record them (`{ method, params }`), at a
    /// pointer scale of 1.
    pub(crate) fn sent(steps: &[InputStep]) -> Vec<Value> {
        sent_at(steps, 1.0)
    }

    fn sent_at(steps: &[InputStep], pointer_scale: f64) -> Vec<Value> {
        steps
            .iter()
            .filter_map(|step| match step {
                InputStep::Dispatch(event) => Some(by_value(json!({
                    "method": event.method(),
                    "params": event.params(pointer_scale),
                }))),
                InputStep::Wait(_) => None,
            })
            .collect()
    }

    /// The burst with its waits, which the goldens cannot show: `"wait 20ms"` between methods.
    pub(crate) fn timeline(steps: &[InputStep]) -> Vec<String> {
        steps
            .iter()
            .map(|step| match step {
                InputStep::Dispatch(event) => {
                    let params = event.params(1.0);
                    let kind = params["type"].as_str().unwrap_or("text").to_owned();
                    format!("{} {kind}", event.method())
                }
                InputStep::Wait(wait) => format!("wait {}ms", wait.as_millis()),
            })
            .collect()
    }

    /// Every number as an f64. Playwright writes a JavaScript number (`0`, `10`) where Rust
    /// writes an f64 (`0.0`, `10.0`); CDP reads both the same.
    pub(crate) fn by_value(value: Value) -> Value {
        match value {
            Value::Number(number) => number.as_f64().map_or(Value::Number(number), Value::from),
            Value::Array(items) => items.into_iter().map(by_value).collect(),
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(key, value)| (key, by_value(value)))
                    .collect(),
            ),
            other => other,
        }
    }

    #[test]
    fn tests_that_only_pointer_positions_take_the_pointer_scale() -> Result<(), Box<dyn Error>> {
        let mut devices = InputDevices::new(Platform::Linux);
        let mut steps = devices.keyboard.down("Shift")?;
        steps.extend(devices.mouse.click(
            Point { x: 10.0, y: 20.0 },
            ClickOptions::default(),
            devices.keyboard.modifiers(),
        ));
        steps.extend(devices.mouse.wheel(3.0, 40.0, devices.keyboard.modifiers()));
        let unscaled = sent(&steps);
        let scaled = sent_at(&steps, 1.5);
        assert_eq!(scaled.len(), 5, "Shift down, move, press, release, wheel");
        assert_eq!(
            scaled[0], unscaled[0],
            "a key event has no position to scale"
        );
        for (scaled, unscaled) in scaled.iter().zip(&unscaled).skip(1) {
            for field in ["x", "y"] {
                let expected = unscaled["params"][field].as_f64().map(|value| value * 1.5);
                assert_eq!(
                    scaled["params"][field].as_f64(),
                    expected,
                    "{field} is scaled"
                );
            }
            assert_eq!(
                scaled["params"]["modifiers"], 8.0,
                "Shift stays held under the mouse"
            );
        }
        assert_eq!(
            (
                scaled[4]["params"]["deltaX"].as_f64(),
                scaled[4]["params"]["deltaY"].as_f64()
            ),
            (Some(3.0), Some(40.0)),
            "wheel deltas are distances, not positions, and core sends them unscaled"
        );
        Ok(())
    }
}
