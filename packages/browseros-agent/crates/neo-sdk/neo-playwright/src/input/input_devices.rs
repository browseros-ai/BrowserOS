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

use std::time::Duration;

use serde_json::{Value, json};

use super::keyboard::{KeyEvent, Keyboard, Platform};
use super::mouse::{Mouse, MouseButton, MouseEvent};

/// What Playwright remembers about one page's input between calls: held keys and modifiers,
/// and the mouse's position and held buttons. The page's state keeps it behind the page's
/// input lock, held for a whole burst, so two ops' events never interleave and each burst
/// starts from the state the last one left.
#[derive(Clone, Debug)]
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

impl InputStep {
    fn event(&self) -> Option<&InputEvent> {
        match self {
            Self::Dispatch(event) => Some(event),
            Self::Wait(_) => None,
        }
    }
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

/// A key or a mouse button: what a burst can leave held down if it stops part-way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Held {
    /// By its code (`ShiftLeft`, `KeyA`).
    Key(&'static str),
    Button(MouseButton),
}

/// Whether an event presses or lets go of a key or a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hold {
    Press(Held),
    Release(Held),
}

impl InputEvent {
    /// What this event presses or lets go of, so a burst that stops part-way knows which of
    /// its later events still have to be sent for the browser to match the device state.
    pub(crate) fn hold(&self) -> Option<Hold> {
        match self {
            Self::Key(event) => Some(event.hold()),
            Self::Mouse(event) => event.hold(),
            Self::InsertText(_) => None,
        }
    }

    pub(crate) fn is_mouse_move(&self) -> bool {
        matches!(self, Self::Mouse(event) if event.is_move())
    }

    fn is_mouse_button(&self) -> bool {
        matches!(
            self.hold(),
            Some(Hold::Press(Held::Button(_)) | Hold::Release(Held::Button(_)))
        )
    }

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

/// Whether a key down in `steps` copies or cuts the selection on `platform` (D14): such a
/// burst first checks that no sensitive field has the focus.
pub(crate) fn copies(steps: &[InputStep], platform: Platform) -> bool {
    steps.iter().any(|step| {
        matches!(step, InputStep::Dispatch(InputEvent::Key(event)) if event.copies(platform))
    })
}

/// How long `steps` wait in all, the `delay`s between their events.
pub(crate) fn waits(steps: &[InputStep]) -> Duration {
    steps
        .iter()
        .filter_map(|step| match step {
            InputStep::Wait(wait) => Some(*wait),
            InputStep::Dispatch(_) => None,
        })
        .fold(Duration::ZERO, Duration::saturating_add)
}

/// How many of `steps` lead up to the burst's first mouse button: everything through the last
/// move before it, or through the last move of a burst without buttons. `None` without a move.
/// A pointer action checks the pointer reached the element's frame right there, so no button
/// goes down until it has.
pub(crate) fn approach(steps: &[InputStep]) -> Option<usize> {
    let buttons = steps
        .iter()
        .position(|step| step.event().is_some_and(InputEvent::is_mouse_button))
        .unwrap_or(steps.len());
    let last_move = steps
        .iter()
        .take(buttons)
        .rposition(|step| step.event().is_some_and(InputEvent::is_mouse_move))?;
    Some(last_move + 1)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::error::Error;

    use serde_json::{Value, json};

    use super::{InputDevices, InputStep, approach};
    use crate::input::keyboard::{Modifier, Modifiers, Platform};
    use crate::input::mouse::{ClickOptions, MouseButton, Point};

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
    fn tests_that_the_approach_ends_with_the_move_before_the_first_button()
    -> Result<(), Box<dyn Error>> {
        let mut devices = InputDevices::new(Platform::Linux);
        let at = Point { x: 10.0, y: 20.0 };
        let shift = Modifiers::default().with(Modifier::Shift);
        let (restore, mut click) = devices.keyboard.ensure_modifiers(shift)?;
        click.extend(devices.mouse.click(
            at,
            ClickOptions::default(),
            devices.keyboard.modifiers(),
        ));
        click.extend(devices.keyboard.ensure_modifiers(restore)?.1);
        let approached = timeline(&click)
            .into_iter()
            .take(approach(&click).unwrap_or_default())
            .collect::<Vec<_>>();
        assert_eq!(
            approached,
            [
                "Input.dispatchKeyEvent rawKeyDown",
                "Input.dispatchMouseEvent mouseMoved"
            ],
            "Shift and the move go first; the press waits"
        );
        let mut held = devices
            .mouse
            .down(MouseButton::Left, 1, Modifiers::default());
        held.extend(
            devices
                .mouse
                .move_to(Point { x: 40.0, y: 50.0 }, 3, Modifiers::default()),
        );
        assert_eq!(
            approach(&held),
            None,
            "a button before any move leaves no approach"
        );
        let hover = devices.mouse.move_to(at, 3, Modifiers::default());
        assert_eq!(
            approach(&hover),
            Some(3),
            "a hover's approach is all its moves"
        );
        Ok(())
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
