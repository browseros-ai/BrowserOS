//! The keyboard and mouse ops and their registration metadata.
//!
//! ```text
//!  keyboard.* / mouse.* ── admit the page ── focus lease (tabs core did not create)
//!    ── the tab's input lock ── the burst, built on a copy of the tab's devices
//!    ── send_effect ── the typing window (keys) ── the burst, on the page's root session
//! ```
//!
//! Positions are the page's main-viewport CSS pixels, as in Playwright: the root session hit
//! tests them through every frame, and keys go to the focused frame, out-of-process ones
//! included (decisions.md, root-session input). The tab's devices take the copy's state only
//! when its burst goes out, so a call refused before that (an unknown key, a refused
//! recheck) leaves nothing held for the next one.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use ts_rs::TS;

use crate::input::{
    BurstFailure, ClickOptions, InputDevices, InputStep, Keyboard, MouseButton, PageInput, Point,
    Typed, UnknownKey,
};
use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, PAGE};
use crate::page::OpPage;
use crate::target::PageRef;
use crate::wire::{self, Describe, Nothing, OpWire, Route, Run, options};
use crate::{ErrorCode, Playwright, PlaywrightError, action};

listed_enum! {
    /// Page-level keyboard and mouse input. `mouse.dblclick` is `mouse.click` with a click
    /// count of 2, as in Playwright's client.
    pub(crate) enum InputOp {
        KeyboardDown,
        KeyboardUp,
        KeyboardPress,
        KeyboardType,
        KeyboardInsertText,
        MouseMove,
        MouseDown,
        MouseUp,
        MouseClick,
        MouseWheel,
    }
}

impl InputOp {
    /// Describes the same input shapes and null answers the runtime route handles.
    pub(crate) fn wire(self) -> Option<OpWire> {
        route(self, Describe)
    }

    /// This op's registration metadata.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::KeyboardDown => OpSpec::write("keyboard.down", PAGE, "hold a key down"),
            Self::KeyboardUp => OpSpec::write("keyboard.up", PAGE, "release a held key"),
            Self::KeyboardPress => OpSpec::write(
                "keyboard.press",
                PAGE,
                "press a key or chord on the focused element",
            ),
            Self::KeyboardType => OpSpec::write(
                "keyboard.type",
                PAGE,
                "type text key by key into the focused element",
            ),
            Self::KeyboardInsertText => OpSpec::write(
                "keyboard.insertText",
                PAGE,
                "insert text into the focused element without key events",
            ),
            Self::MouseMove => OpSpec::write("mouse.move", PAGE, "move the mouse to a point"),
            Self::MouseDown => OpSpec::write(
                "mouse.down",
                PAGE,
                "press a mouse button where the mouse is",
            ),
            Self::MouseUp => OpSpec::write("mouse.up", PAGE, "release a mouse button"),
            Self::MouseClick => {
                OpSpec::write("mouse.click", PAGE, "click at a point; also mouse.dblclick")
            }
            Self::MouseWheel => {
                OpSpec::write("mouse.wheel", PAGE, "scroll the wheel where the mouse is")
            }
        }
    }
}

/// `keyboard.down(key)` and `keyboard.up(key)`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "KeyboardKeyInput")]
struct KeyInput {
    page: PageRef,
    key: Typed,
}
options!(KeyInput);

/// `keyboard.press(key, { delay })`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "KeyboardPressInput")]
struct PressInput {
    page: PageRef,
    key: Typed,
    options: DelayOptions,
}
options!(PressInput => DelayOptions);

/// `keyboard.type(text, { delay })`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "KeyboardTypeInput")]
struct TypeInput {
    page: PageRef,
    text: Typed,
    options: DelayOptions,
}
options!(TypeInput => DelayOptions);

/// `keyboard.insertText(text)`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "KeyboardInsertTextInput")]
struct InsertTextInput {
    page: PageRef,
    text: Typed,
}
options!(InsertTextInput);

#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "KeyboardDelayOptions")]
#[ts(optional_fields)]
struct DelayOptions {
    /// Milliseconds, as upstream's `delay`.
    delay: Option<f64>,
}

/// `mouse.move(x, y, { steps })`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "MouseMoveInput")]
struct MoveInput {
    page: PageRef,
    x: f64,
    y: f64,
    options: StepOptions,
}
options!(MoveInput => StepOptions);

#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "MouseStepOptions")]
#[ts(optional_fields)]
struct StepOptions {
    steps: Option<u32>,
}

/// `mouse.down({ button, clickCount })` and `mouse.up(..)`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "MouseButtonInput")]
struct ButtonInput {
    page: PageRef,
    options: ButtonOptions,
}
options!(ButtonInput => ButtonOptions);

#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename = "MouseButtonOptions")]
#[ts(optional_fields)]
struct ButtonOptions {
    button: Option<WireButton>,
    click_count: Option<u32>,
}

/// `mouse.click(x, y, { button, clickCount, delay })` and `mouse.dblclick`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "MouseClickInput")]
struct ClickInput {
    page: PageRef,
    x: f64,
    y: f64,
    options: MouseClickOptions,
}
options!(ClickInput => MouseClickOptions);

#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(optional_fields)]
struct MouseClickOptions {
    button: Option<WireButton>,
    click_count: Option<u32>,
    delay: Option<f64>,
}

/// `mouse.wheel(deltaX, deltaY)`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename = "MouseWheelInput")]
struct WheelInput {
    page: PageRef,
    delta_x: f64,
    delta_y: f64,
}
options!(WheelInput);

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename = "MouseButton")]
enum WireButton {
    Left,
    Right,
    Middle,
}

fn button(wire: Option<WireButton>) -> MouseButton {
    match wire {
        None | Some(WireButton::Left) => MouseButton::Left,
        Some(WireButton::Right) => MouseButton::Right,
        Some(WireButton::Middle) => MouseButton::Middle,
    }
}

/// Runs one keyboard or mouse op through the shared decode/call/encode boundary. The route
/// boxes the burst future so the dispatcher does not inherit its device state machine.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: InputOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    route(op, Run { pw, scope, args }).await
}

/// Run and Describe share this exhaustive list. Admission, leases and device-state commit
/// remain inside `burst`; describing a contract never acquires any of them.
fn route<'a, R: Route<'a>>(op: InputOp, r: R) -> R::Out {
    match op {
        InputOp::KeyboardDown => r.op(wire::command("Page.keyboardDown"), keyboard_down),
        InputOp::KeyboardUp => r.op(wire::command("Page.keyboardUp"), keyboard_up),
        InputOp::KeyboardPress => r.op(wire::command("Page.keyboardPress"), keyboard_press),
        InputOp::KeyboardType => r.op(wire::command("Page.keyboardType"), keyboard_type),
        InputOp::KeyboardInsertText => r.op(
            wire::command("Page.keyboardInsertText"),
            keyboard_insert_text,
        ),
        InputOp::MouseMove => r.op(wire::command("Page.mouseMove"), mouse_move),
        InputOp::MouseDown => r.op(wire::command("Page.mouseDown"), mouse_down),
        InputOp::MouseUp => r.op(wire::command("Page.mouseUp"), mouse_up),
        InputOp::MouseClick => r.op(wire::command("Page.mouseClick"), mouse_click),
        InputOp::MouseWheel => r.op(wire::command("Page.mouseWheel"), mouse_wheel),
    }
}

async fn keyboard_down(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: KeyInput,
) -> Result<Nothing, PlaywrightError> {
    burst(pw, scope, &input.page, |devices| {
        devices.keyboard.down(&input.key.0)
    })
    .await
}

async fn keyboard_up(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: KeyInput,
) -> Result<Nothing, PlaywrightError> {
    burst(pw, scope, &input.page, |devices| {
        devices.keyboard.up(&input.key.0)
    })
    .await
}

async fn keyboard_press(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PressInput,
) -> Result<Nothing, PlaywrightError> {
    let delay = millis(input.options.delay);
    burst(pw, scope, &input.page, |devices| {
        devices.keyboard.press(&input.key.0, delay)
    })
    .await
}

async fn keyboard_type(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: TypeInput,
) -> Result<Nothing, PlaywrightError> {
    let delay = millis(input.options.delay);
    burst(pw, scope, &input.page, |devices| {
        Ok(devices.keyboard.type_text(&input.text.0, delay))
    })
    .await
}

async fn keyboard_insert_text(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: InsertTextInput,
) -> Result<Nothing, PlaywrightError> {
    burst(pw, scope, &input.page, |_| {
        Ok(Keyboard::insert_text(&input.text.0))
    })
    .await
}

async fn mouse_move(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: MoveInput,
) -> Result<Nothing, PlaywrightError> {
    let to = Point {
        x: input.x,
        y: input.y,
    };
    let steps = input.options.steps.unwrap_or(1);
    burst(pw, scope, &input.page, |devices| {
        let held = devices.keyboard.modifiers();
        Ok(devices.mouse.move_to(to, steps, held))
    })
    .await
}

async fn mouse_down(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ButtonInput,
) -> Result<Nothing, PlaywrightError> {
    let pressed = button(input.options.button);
    let count = input.options.click_count.unwrap_or(1);
    burst(pw, scope, &input.page, |devices| {
        let held = devices.keyboard.modifiers();
        Ok(devices.mouse.down(pressed, count, held))
    })
    .await
}

async fn mouse_up(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ButtonInput,
) -> Result<Nothing, PlaywrightError> {
    let pressed = button(input.options.button);
    let count = input.options.click_count.unwrap_or(1);
    burst(pw, scope, &input.page, |devices| {
        let held = devices.keyboard.modifiers();
        Ok(devices.mouse.up(pressed, count, held))
    })
    .await
}

async fn mouse_click(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ClickInput,
) -> Result<Nothing, PlaywrightError> {
    // 1.64.1's protocol gives `mouse.click` no `steps` (mouse.rs, `ClickOptions`).
    let options = ClickOptions {
        button: button(input.options.button),
        click_count: input.options.click_count.unwrap_or(1),
        delay: millis(input.options.delay),
        steps: 1,
    };
    let at = Point {
        x: input.x,
        y: input.y,
    };
    burst(pw, scope, &input.page, |devices| {
        let held = devices.keyboard.modifiers();
        Ok(devices.mouse.click(at, options, held))
    })
    .await
}

async fn mouse_wheel(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WheelInput,
) -> Result<Nothing, PlaywrightError> {
    burst(pw, scope, &input.page, |devices| {
        let held = devices.keyboard.modifiers();
        Ok(devices.mouse.wheel(input.delta_x, input.delta_y, held))
    })
    .await
}

/// One call's burst on `page`, from admission to its last step. `build` turns the call into
/// steps on a copy of the tab's devices; an unknown key fails the op before anything is sent.
async fn burst(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: &PageRef,
    build: impl FnOnce(&mut InputDevices) -> Result<Vec<InputStep>, UnknownKey>,
) -> Result<(), PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, page).await?;
    let _lease = action::lease(scope, &page).await?;
    let mut input = hold(scope, &page).await?;
    let mut planned = input.plan();
    let steps = build(&mut planned)
        .map_err(|unknown| scope.fail(ErrorCode::InvalidInput, &unknown.to_string()))?;
    let sent = scope.send_effect()?;
    input
        .send_planned(&sent, scope, planned, steps)
        .await
        .map_err(|failure| burst_failed(scope, failure))
}

/// The tab's input lock, within the op's timeout.
pub(crate) async fn hold(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
) -> Result<PageInput, PlaywrightError> {
    let scale = scope.view().pointer_scale(page.core_page());
    let held = PageInput::hold(Arc::clone(page.state().input()), page.root_session(), scale);
    scope
        .within(held)
        .await
        .map_err(|_expired| scope.timed_out(ErrorCode::NotActionable))
}

/// A burst that could not go out whole; what it sent may have landed.
pub(crate) fn burst_failed(scope: &OpScope<'_>, failure: BurstFailure) -> PlaywrightError {
    match failure {
        BurstFailure::Expired => scope.timed_out(ErrorCode::NotActionable),
        BurstFailure::Cdp(error) => scope.core_error(&error).after_input(),
    }
}

/// A Playwright delay in milliseconds; a negative or non-finite one waits nothing, as a
/// negative JavaScript timer does.
pub(crate) fn millis(delay: Option<f64>) -> Duration {
    delay
        .filter(|millis| millis.is_finite() && *millis > 0.0)
        .and_then(|millis| Duration::try_from_secs_f64(millis / 1000.0).ok())
        .unwrap_or(Duration::ZERO)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::InputOp;
    use crate::wire::Nothing;

    #[test]
    fn tests_that_every_input_answer_keeps_its_null_bytes() -> Result<(), Box<dyn Error>> {
        // Input already answered null before route migration. A typed empty object would
        // change those bytes, even though the script's methods discard their answers.
        let answer: Nothing = ();
        assert_eq!(serde_json::to_string(&answer)?, "null");
        for op in InputOp::ALL {
            let wire = op.wire().ok_or("every input op has a typed route")?;
            assert_eq!(
                wire.output,
                "null",
                "{} must still answer null",
                op.spec().name
            );
        }
        Ok(())
    }
}
