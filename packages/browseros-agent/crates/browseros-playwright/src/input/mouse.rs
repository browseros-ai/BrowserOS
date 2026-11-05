//! Playwright's mouse as data and state, ported from upstream's `Mouse` (server/input.ts) and
//! Chromium's `RawMouseImpl` (server/chromium/crInput.ts) at v1.64.1: where the pointer is,
//! which buttons are held, and the exact `Input.dispatchMouseEvent` events each call sends.
//!
//! Positions are the page's main-viewport CSS pixels, as in Playwright; `InputEvent::params`
//! applies core's pointer scale on the way out. Upstream's drag interception is left out: a
//! move with the left button held is a plain `mouseMoved`, without `Input.setInterceptDrags`
//! around it, and Chromium's own input handler carries an HTML5 drag from such moves (core's
//! `mouse::dispatch_drag` relies on the same).

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "mouse ops and pointer actions send these bursts once their leaves land"
    )
)]

use std::time::Duration;

use serde_json::{Value, json};

use super::input_devices::{InputEvent, InputStep};
use super::keyboard::Modifiers;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Middle => "middle",
        }
    }

    const fn bit(self) -> u8 {
        match self {
            Self::Left => 1,
            Self::Right => 2,
            Self::Middle => 4,
        }
    }
}

/// Held buttons as CDP's `buttons` mask: left 1, right 2, middle 4.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Buttons(u8);

impl Buttons {
    #[must_use]
    fn with(self, button: MouseButton) -> Self {
        Self(self.0 | button.bit())
    }

    #[must_use]
    fn without(self, button: MouseButton) -> Self {
        Self(self.0 & !button.bit())
    }
}

/// A point in the page's main-viewport CSS pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Point {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

/// `mouse.click`'s options, defaulting as upstream does.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClickOptions {
    pub(crate) button: MouseButton,
    /// Presses in the click: 2 for `dblclick`.
    pub(crate) click_count: u32,
    /// After each press, and between one release and the next press. Zero waits nowhere.
    pub(crate) delay: Duration,
    /// Moves on the way to the point, for `locator.click({ steps })`. Version 1.64.1's
    /// protocol gives `mouse.click` no `steps` (its validator drops the option: measured,
    /// `mouse.click(5, 5, { steps: 3 })` sends one move), so that op leaves it at 1.
    pub(crate) steps: u32,
}

impl Default for ClickOptions {
    fn default() -> Self {
        Self {
            button: MouseButton::Left,
            click_count: 1,
            delay: Duration::ZERO,
            steps: 1,
        }
    }
}

/// One `Input.dispatchMouseEvent`, as `RawMouseImpl` builds it.
pub(crate) struct MouseEvent {
    kind: MouseEventKind,
    at: Point,
    /// Held after this event: a press counts its own button, a release no longer does.
    buttons: Buttons,
    modifiers: Modifiers,
}

enum MouseEventKind {
    /// Reports the last pressed button that is still unreleased, if any.
    Moved {
        button: Option<MouseButton>,
    },
    Pressed {
        button: MouseButton,
        click_count: u32,
    },
    Released {
        button: MouseButton,
        click_count: u32,
    },
    Wheel {
        delta_x: f64,
        delta_y: f64,
    },
}

impl MouseEvent {
    /// The CDP params, with the position times core's `pointer_scale`. Wheel deltas are not
    /// positions and stay unscaled, as core's own scrolling sends them.
    pub(crate) fn params(&self, pointer_scale: f64) -> Value {
        let x = self.at.x * pointer_scale;
        let y = self.at.y * pointer_scale;
        let buttons = self.buttons.0;
        let modifiers = self.modifiers.mask();
        // Upstream presses and moves with a force of 0.5 while any button is held.
        let force = if buttons == 0 { 0.0 } else { 0.5 };
        match self.kind {
            MouseEventKind::Moved { button } => json!({
                "type": "mouseMoved",
                "button": button.map_or("none", MouseButton::name),
                "buttons": buttons,
                "x": x,
                "y": y,
                "modifiers": modifiers,
                "force": force,
            }),
            MouseEventKind::Pressed {
                button,
                click_count,
            } => json!({
                "type": "mousePressed",
                "button": button.name(),
                "buttons": buttons,
                "x": x,
                "y": y,
                "modifiers": modifiers,
                "clickCount": click_count,
                "force": force,
            }),
            MouseEventKind::Released {
                button,
                click_count,
            } => json!({
                "type": "mouseReleased",
                "button": button.name(),
                "buttons": buttons,
                "x": x,
                "y": y,
                "modifiers": modifiers,
                "clickCount": click_count,
            }),
            MouseEventKind::Wheel { delta_x, delta_y } => json!({
                "type": "mouseWheel",
                "x": x,
                "y": y,
                "modifiers": modifiers,
                "deltaX": delta_x,
                "deltaY": delta_y,
            }),
        }
    }
}

/// One page's mouse. Events carry the keyboard's held modifiers, which each call takes.
#[derive(Debug, Default)]
pub(crate) struct Mouse {
    position: Point,
    /// The button pressed last, until any release: upstream forgets it then, even with another
    /// button still held, and later moves report `button: "none"`.
    last_button: Option<MouseButton>,
    buttons: Buttons,
}

impl Mouse {
    /// `mouse.move(x, y, { steps })`: `steps` moves along the line from the current position,
    /// the last one on `to`. With zero steps the pointer moves without an event, as upstream.
    pub(crate) fn move_to(
        &mut self,
        to: Point,
        steps: u32,
        modifiers: Modifiers,
    ) -> Vec<InputStep> {
        let from = self.position;
        self.position = to;
        (1..=steps)
            .map(|step| {
                // Upstream's arithmetic, in its order, so the points match to the last bit.
                let fraction = f64::from(step) / f64::from(steps);
                let at = Point {
                    x: from.x + (to.x - from.x) * fraction,
                    y: from.y + (to.y - from.y) * fraction,
                };
                self.event(
                    MouseEventKind::Moved {
                        button: self.last_button,
                    },
                    at,
                    modifiers,
                )
            })
            .collect()
    }

    /// `mouse.down({ button, clickCount })`, where the pointer is.
    pub(crate) fn down(
        &mut self,
        button: MouseButton,
        click_count: u32,
        modifiers: Modifiers,
    ) -> Vec<InputStep> {
        self.last_button = Some(button);
        self.buttons = self.buttons.with(button);
        vec![self.event(
            MouseEventKind::Pressed {
                button,
                click_count,
            },
            self.position,
            modifiers,
        )]
    }

    /// `mouse.up({ button, clickCount })`, where the pointer is.
    pub(crate) fn up(
        &mut self,
        button: MouseButton,
        click_count: u32,
        modifiers: Modifiers,
    ) -> Vec<InputStep> {
        self.last_button = None;
        self.buttons = self.buttons.without(button);
        vec![self.event(
            MouseEventKind::Released {
                button,
                click_count,
            },
            self.position,
            modifiers,
        )]
    }

    /// `mouse.click(x, y, options)`, and `mouse.dblclick` as `click_count: 2`: a move, then a
    /// press and a release for each count up to `click_count`.
    pub(crate) fn click(
        &mut self,
        at: Point,
        options: ClickOptions,
        modifiers: Modifiers,
    ) -> Vec<InputStep> {
        let mut steps = self.move_to(at, options.steps, modifiers);
        for count in 1..=options.click_count {
            steps.extend(self.down(options.button, count, modifiers));
            if !options.delay.is_zero() {
                steps.push(InputStep::Wait(options.delay));
            }
            steps.extend(self.up(options.button, count, modifiers));
            if !options.delay.is_zero() && count < options.click_count {
                steps.push(InputStep::Wait(options.delay));
            }
        }
        steps
    }

    /// `mouse.wheel(deltaX, deltaY)`, where the pointer is; the pointer stays.
    pub(crate) fn wheel(&self, delta_x: f64, delta_y: f64, modifiers: Modifiers) -> Vec<InputStep> {
        vec![self.event(
            MouseEventKind::Wheel { delta_x, delta_y },
            self.position,
            modifiers,
        )]
    }

    fn event(&self, kind: MouseEventKind, at: Point, modifiers: Modifiers) -> InputStep {
        InputStep::Dispatch(InputEvent::Mouse(MouseEvent {
            kind,
            at,
            buttons: self.buttons,
            modifiers,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Duration;

    use serde_json::{Map, Value};

    use super::{ClickOptions, MouseButton, Point};
    use crate::input::input_devices::tests::{by_value, sent, timeline};
    use crate::input::input_devices::{InputDevices, InputStep};
    use crate::input::keyboard::Platform;

    /// Recorded from playwright-core 1.64.1 by scripts/capture-goldens.mjs.
    const GOLDENS: &str = include_str!("../../tests/data/mouse-events.json");

    fn number(call: &[Value], index: usize) -> f64 {
        call.get(index).and_then(Value::as_f64).unwrap_or_default()
    }

    /// The options object a recorded call ends with, if any.
    fn options(call: &[Value]) -> Map<String, Value> {
        call.last()
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }

    fn button(options: &Map<String, Value>) -> MouseButton {
        match options.get("button").and_then(Value::as_str) {
            Some("right") => MouseButton::Right,
            Some("middle") => MouseButton::Middle,
            _ => MouseButton::Left,
        }
    }

    fn count(options: &Map<String, Value>, name: &str) -> u32 {
        options
            .get(name)
            .and_then(Value::as_u64)
            .and_then(|count| u32::try_from(count).ok())
            .unwrap_or(1)
    }

    /// One recorded `page[device][method](...args)` call, with upstream's client defaults.
    fn replay(
        devices: &mut InputDevices,
        call: &[Value],
    ) -> Result<Vec<InputStep>, Box<dyn Error>> {
        let name = call.first().and_then(Value::as_str).unwrap_or_default();
        let options = options(call);
        let modifiers = devices.keyboard.modifiers();
        let at = Point {
            x: number(call, 1),
            y: number(call, 2),
        };
        // `mouse.click` and `mouse.dblclick` keep only the options 1.64.1's protocol has.
        let click = ClickOptions {
            button: button(&options),
            click_count: count(&options, "clickCount"),
            delay: Duration::from_secs_f64(
                options
                    .get("delay")
                    .and_then(Value::as_f64)
                    .unwrap_or_default()
                    / 1000.0,
            ),
            ..ClickOptions::default()
        };
        Ok(match name {
            "mouse.move" => devices
                .mouse
                .move_to(at, count(&options, "steps"), modifiers),
            "mouse.down" => {
                devices
                    .mouse
                    .down(button(&options), count(&options, "clickCount"), modifiers)
            }
            "mouse.up" => {
                devices
                    .mouse
                    .up(button(&options), count(&options, "clickCount"), modifiers)
            }
            "mouse.click" => devices.mouse.click(at, click, modifiers),
            // The client's `dblclick` is `click` with `clickCount: 2`.
            "mouse.dblclick" => devices.mouse.click(
                at,
                ClickOptions {
                    click_count: 2,
                    ..click
                },
                modifiers,
            ),
            "mouse.wheel" => devices
                .mouse
                .wheel(number(call, 1), number(call, 2), modifiers),
            "keyboard.down" => devices
                .keyboard
                .down(call.get(1).and_then(Value::as_str).unwrap_or_default())?,
            "keyboard.up" => devices
                .keyboard
                .up(call.get(1).and_then(Value::as_str).unwrap_or_default())?,
            other => panic!("the goldens call {other}, which this replay does not know"),
        })
    }

    #[test]
    fn tests_that_every_recorded_mouse_call_sends_what_playwright_sent()
    -> Result<(), Box<dyn Error>> {
        let goldens: Value = serde_json::from_str(GOLDENS)?;
        let Some(cases) = goldens["cases"].as_array() else {
            panic!("the goldens have no cases");
        };
        assert!(!cases.is_empty(), "the goldens have cases");
        let mut mismatches = Vec::new();
        for case in cases {
            let calls = case["calls"].as_array().cloned().unwrap_or_default();
            let mut devices = InputDevices::new(Platform::Mac);
            let mut steps = Vec::new();
            for call in &calls {
                steps.extend(replay(
                    &mut devices,
                    call.as_array().map_or(&[], Vec::as_slice),
                )?);
            }
            // Neo leaves out upstream's drag interception (see the module doc).
            let expected: Vec<Value> = case["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|message| message["method"] != "Input.setInterceptDrags")
                .map(|message| by_value(message.clone()))
                .collect();
            if sent(&steps) != expected {
                mismatches.push(Value::Array(calls).to_string());
            }
        }
        assert!(
            mismatches.is_empty(),
            "sent other events than Playwright: {mismatches:#?}"
        );
        Ok(())
    }

    /// Upstream's `click` with a delay waits after each press and between clicks, never after
    /// the last release.
    #[test]
    fn tests_that_click_delays_fall_where_upstream_waits() {
        let mut devices = InputDevices::new(Platform::Linux);
        let steps = devices.mouse.click(
            Point { x: 5.0, y: 6.0 },
            ClickOptions {
                click_count: 2,
                delay: Duration::from_millis(20),
                ..ClickOptions::default()
            },
            devices.keyboard.modifiers(),
        );
        assert_eq!(
            timeline(&steps),
            [
                "Input.dispatchMouseEvent mouseMoved",
                "Input.dispatchMouseEvent mousePressed",
                "wait 20ms",
                "Input.dispatchMouseEvent mouseReleased",
                "wait 20ms",
                "Input.dispatchMouseEvent mousePressed",
                "wait 20ms",
                "Input.dispatchMouseEvent mouseReleased",
            ]
        );
    }

    #[test]
    fn tests_that_a_stepped_click_moves_from_where_the_pointer_was() {
        let mut devices = InputDevices::new(Platform::Linux);
        let modifiers = devices.keyboard.modifiers();
        devices
            .mouse
            .move_to(Point { x: 10.0, y: 10.0 }, 1, modifiers);
        let steps = devices.mouse.click(
            Point { x: 40.0, y: 70.0 },
            ClickOptions {
                steps: 3,
                ..ClickOptions::default()
            },
            modifiers,
        );
        let moves: Vec<(Option<f64>, Option<f64>)> = sent(&steps)
            .iter()
            .filter(|message| message["params"]["type"] == "mouseMoved")
            .map(|message| {
                (
                    message["params"]["x"].as_f64(),
                    message["params"]["y"].as_f64(),
                )
            })
            .collect();
        assert_eq!(
            moves,
            [
                (Some(20.0), Some(30.0)),
                (Some(30.0), Some(50.0)),
                (Some(40.0), Some(70.0)),
            ],
            "a third of the way, two thirds, then the point"
        );
    }
}
