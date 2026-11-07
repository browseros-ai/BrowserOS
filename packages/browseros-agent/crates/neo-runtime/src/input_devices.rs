//! Playwright: `server/input.ts` `Mouse`
//! Playwright: `server/input.ts` `Touchscreen`
//! Playwright: `server/chromium/crInput.ts` `RawMouseImpl`
//! Playwright: `server/chromium/crInput.ts` `RawTouchscreenImpl`
//! Neo: a tab's keyboard and mouse between calls (the input-device seam of med-pw-dom's
//! design, §4), the plans every input call makes from them, and the one writer that sends a
//! plan. Playwright keeps one `Keyboard` and one `Mouse` per page; Neo keeps their state per tab
//! per `Runtime`, inside the tab's input turn (`page.rs` `TabInputs`), so only a call holding the
//! turn reads or changes it.
//!
//! ```text
//!  call holds the turn ─▶ plan from a copy of the devices (each command applied to the copy as
//!                         it is planned, so every later command sees the held modifiers, the
//!                         held buttons and the pointer where the earlier ones leave them)
//!                       ─▶ write_plan: sends the commands in order, applying each one's effect
//!                         to the real devices as it is written, so the state is what the
//!                         browser saw however the call ends
//! ```
//!
//! A plan's releases are owed: a release (`keyUp`, `mouseReleased`, `touchEnd`, a modifier
//! restore) names the command it undoes, and is written even after the call stopped or a write
//! failed, whenever that command may have been written. Nothing else is written once the writer
//! stopped. Keys and buttons a call presses without releasing (`keyboard.down`, `mouse.down`)
//! stay held, tagged with the run that pressed them, until a later `up` or the run's end
//! (`Dispatcher::finish`, `plan_release`).

use std::future::Future;
use std::time::Duration;

use browseros_cdp::input::DispatchMouseEventParams;
use browseros_cdp::{CdpError, SessionId};
use futures_util::future;
use serde_json::json;
use tokio::time;
use tokio_util::sync::CancellationToken;

use crate::click::{MouseButton, MouseClick, PageError, PressReport};
use crate::progress::InputTicket;
use crate::world;

/// The run that pressed a key or a button: its own CDP session on the tab, since every run
/// attaches a session of its own.
pub(crate) type Owner = SessionId;

/// One tab's keyboard and mouse. A fresh one is Playwright's new page: the pointer at (0, 0),
/// nothing held.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct InputDevices {
    pub(crate) keyboard: KeyboardState,
    pub(crate) mouse: MouseState,
}

/// Playwright's `Keyboard` state.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct KeyboardState {
    /// `_pressedModifiers`, as CDP's modifiers mask (`keyboard.rs` `ALT` … `SHIFT`).
    pub(crate) modifiers: u8,
    /// `_pressedKeys`: every held key's code, oldest first, with the run that pressed it.
    pub(crate) pressed: Vec<(&'static str, Owner)>,
}

/// Playwright's `Mouse` state.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct MouseState {
    pub(crate) x: f64,
    pub(crate) y: f64,
    /// `_lastButton`; `None` is Playwright's `'none'`.
    pub(crate) last_button: Option<MouseButton>,
    /// `_buttons`, oldest first, each with the run that pressed it.
    pub(crate) buttons: Vec<(MouseButton, Owner)>,
}

/// What one written command did to the tab's devices.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Effect {
    None,
    KeyDown {
        code: &'static str,
        /// The key's modifier bit, `0` for a key that is not a modifier.
        modifier: u8,
        owner: Owner,
    },
    KeyUp {
        code: &'static str,
        modifier: u8,
    },
    Moved {
        x: f64,
        y: f64,
    },
    ButtonDown {
        button: MouseButton,
        owner: Owner,
    },
    ButtonUp {
        button: MouseButton,
    },
}

impl InputDevices {
    /// Applies a command's effect. A press counts from its write, answered or not, since it may
    /// have reached the browser; a release and a move count only once answered, so a key whose
    /// release failed stays held and the run's end releases it again. Pressing a held key again
    /// keeps its first owner, as Playwright's `_pressedKeys` set keeps it once.
    pub(crate) fn apply(&mut self, effect: &Effect, answered: bool) {
        match effect {
            Effect::None => {}
            Effect::KeyDown {
                code,
                modifier,
                owner,
            } => {
                self.keyboard.modifiers |= modifier;
                if !self.keyboard.pressed.iter().any(|(held, _)| held == code) {
                    self.keyboard.pressed.push((code, owner.clone()));
                }
            }
            Effect::KeyUp { code, modifier } => {
                if answered {
                    self.keyboard.modifiers &= !modifier;
                    self.keyboard.pressed.retain(|(held, _)| held != code);
                }
            }
            Effect::Moved { x, y } => {
                if answered {
                    self.mouse.x = *x;
                    self.mouse.y = *y;
                }
            }
            Effect::ButtonDown { button, owner } => {
                self.mouse.last_button = Some(*button);
                if !self.mouse.buttons.iter().any(|(held, _)| held == button) {
                    self.mouse.buttons.push((*button, owner.clone()));
                }
            }
            Effect::ButtonUp { button } => {
                if answered {
                    self.mouse.last_button = None;
                    self.mouse.buttons.retain(|(held, _)| held != button);
                }
            }
        }
    }

    /// `toButtonsMask` of the held buttons.
    fn buttons_mask(&self) -> i64 {
        self.mouse
            .buttons
            .iter()
            .map(|(button, _)| button_mask(*button))
            .sum()
    }
}

/// One command of a plan, decided before anything is written.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Planned {
    /// The `Input.*` method, or `None` for a change of state that writes nothing (a move of no
    /// steps still moves Playwright's pointer).
    pub(crate) method: Option<&'static str>,
    /// Its params, already JSON, written unchanged.
    pub(crate) params: String,
    pub(crate) effect: Effect,
    /// The command this one undoes (a release its press, a modifier restore the change it
    /// restores): written even after the writer stopped when that one may have been written,
    /// and never when it was not.
    pub(crate) undoes: Option<usize>,
    /// Written together with the next command, without waiting for this one's answer: the
    /// pipelined events of Playwright's click and tap.
    pub(crate) with_next: bool,
    /// Slept before this command, unless the writer stopped (Playwright's `delay`). Only on the
    /// first command of a batch.
    pub(crate) pause: Option<Duration>,
}

/// The commands of one input call, in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Plan {
    pub(crate) commands: Vec<Planned>,
}

impl Plan {
    /// Plans `command` and applies it to `simulated`, as if written and answered, so the next
    /// command sees the devices this one leaves. Returns its index, for a later release.
    pub(crate) fn push(&mut self, simulated: &mut InputDevices, command: Planned) -> usize {
        simulated.apply(&command.effect, true);
        self.commands.push(command);
        self.commands.len() - 1
    }

    /// Whether the plan writes anything at all.
    pub(crate) fn writes(&self) -> bool {
        self.commands.iter().any(|command| command.method.is_some())
    }

    /// Makes the last command written together with the next one.
    pub(crate) fn pipeline_last(&mut self) {
        if let Some(last) = self.commands.last_mut() {
            last.with_next = true;
        }
    }
}

impl Planned {
    /// A command written on its own, undoing nothing.
    pub(crate) fn command(method: &'static str, params: String, effect: Effect) -> Self {
        Self {
            method: Some(method),
            params,
            effect,
            undoes: None,
            with_next: false,
            pause: None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The mouse and the touchscreen, planned from the devices.
// ---------------------------------------------------------------------------------------------

/// Playwright: `server/chromium/crProtocolHelper.ts` `toButtonsMask`
fn button_mask(button: MouseButton) -> i64 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Right => 2,
        MouseButton::Middle => 4,
    }
}

fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
    }
}

/// `Input.dispatchMouseEvent`, as JSON.
fn mouse_command(params: &DispatchMouseEventParams, effect: Effect) -> Planned {
    // Serializing this plain struct cannot fail; an empty object would be refused by Chromium.
    let params = serde_json::to_string(params).unwrap_or_default();
    Planned::command("Input.dispatchMouseEvent", params, effect)
}

/// Playwright: `server/input.ts` `Mouse.move`: `const middleX = fromX + (x - fromX) * (i / steps);`
///
/// `steps` moves from the pointer's last position to `to`, each a `RawMouseImpl.move`: the last
/// button or `none`, the held buttons, the held modifiers, and force while a button is held.
/// No steps moves the pointer without an event, as Playwright's loop does.
pub(crate) fn plan_move(plan: &mut Plan, simulated: &mut InputDevices, to: (f64, f64), steps: u32) {
    let (from_x, from_y) = (simulated.mouse.x, simulated.mouse.y);
    if steps == 0 {
        let moved = Effect::Moved { x: to.0, y: to.1 };
        plan.push(
            simulated,
            Planned {
                method: None,
                ..Planned::command("", String::new(), moved)
            },
        );
        return;
    }
    for step in 1..=steps {
        let fraction = f64::from(step) / f64::from(steps);
        let x = from_x + (to.0 - from_x) * fraction;
        let y = from_y + (to.1 - from_y) * fraction;
        let held = !simulated.mouse.buttons.is_empty();
        // Playwright: `server/chromium/crInput.ts` `RawMouseImpl.move`: `force: buttons.size > 0 ? 0.5 : 0,`
        let params = DispatchMouseEventParams {
            r#type: "mouseMoved".to_string(),
            x,
            y,
            modifiers: Some(i64::from(simulated.keyboard.modifiers)),
            button: Some(
                simulated
                    .mouse
                    .last_button
                    .map_or("none", button_name)
                    .to_string(),
            ),
            buttons: Some(simulated.buttons_mask()),
            force: Some(if held { 0.5 } else { 0.0 }),
            ..DispatchMouseEventParams::default()
        };
        let command = mouse_command(&params, Effect::Moved { x, y });
        plan.push(simulated, command);
    }
}

/// Playwright: `server/input.ts` `Mouse.down`: `this._buttons.add(button);`
///
/// A press at the pointer's position, with every held button and modifier. Returns its index,
/// for the release that undoes it.
pub(crate) fn plan_down(
    plan: &mut Plan,
    simulated: &mut InputDevices,
    button: MouseButton,
    click_count: i64,
    owner: &Owner,
) -> usize {
    let mut pressed = simulated.clone();
    pressed.apply(
        &Effect::ButtonDown {
            button,
            owner: owner.clone(),
        },
        true,
    );
    // Playwright: `server/chromium/crInput.ts` `RawMouseImpl.down`: `force: buttons.size > 0 ? 0.5 : 0,`
    let params = DispatchMouseEventParams {
        r#type: "mousePressed".to_string(),
        x: simulated.mouse.x,
        y: simulated.mouse.y,
        modifiers: Some(i64::from(simulated.keyboard.modifiers)),
        button: Some(button_name(button).to_string()),
        buttons: Some(pressed.buttons_mask()),
        click_count: Some(click_count),
        force: Some(0.5),
        ..DispatchMouseEventParams::default()
    };
    let effect = Effect::ButtonDown {
        button,
        owner: owner.clone(),
    };
    plan.push(simulated, mouse_command(&params, effect))
}

/// Playwright: `server/input.ts` `Mouse.up`: `this._buttons.delete(button);`
///
/// A release at the pointer's position, with the buttons still held after it. `undoes` is the
/// press it releases, if this plan made it.
pub(crate) fn plan_up(
    plan: &mut Plan,
    simulated: &mut InputDevices,
    button: MouseButton,
    click_count: i64,
    undoes: Option<usize>,
) -> usize {
    let mut released = simulated.clone();
    released.apply(&Effect::ButtonUp { button }, true);
    let params = DispatchMouseEventParams {
        r#type: "mouseReleased".to_string(),
        x: simulated.mouse.x,
        y: simulated.mouse.y,
        modifiers: Some(i64::from(simulated.keyboard.modifiers)),
        button: Some(button_name(button).to_string()),
        buttons: Some(released.buttons_mask()),
        click_count: Some(click_count),
        ..DispatchMouseEventParams::default()
    };
    let command = Planned {
        undoes,
        ..mouse_command(&params, Effect::ButtonUp { button })
    };
    plan.push(simulated, command)
}

/// Playwright: `server/input.ts` `Mouse.click`
///
/// The move (in `steps` from the last position), then a press and a release per click count.
/// Without `delay` the events are pipelined, the move with them unless it takes more than one
/// step; with `delay`, each event waits for the one before, sleeping `delay` between a press
/// and its release and between clicks.
pub(crate) fn plan_click(
    plan: &mut Plan,
    simulated: &mut InputDevices,
    to: (f64, f64),
    click: &MouseClick,
    steps: u32,
    owner: &Owner,
) {
    plan_move(plan, simulated, to, steps);
    if click.delay.is_none() && steps <= 1 && click.click_count > 0 {
        plan.pipeline_last();
    }
    for count in 1..=click.click_count {
        let count = i64::from(count);
        let pressed = plan_down(plan, simulated, click.button, count, owner);
        if let Some(delay) = click.delay {
            if count > 1 {
                plan.commands[pressed].pause = Some(delay);
            }
        } else {
            plan.pipeline_last();
        }
        let released = plan_up(plan, simulated, click.button, count, Some(pressed));
        match click.delay {
            Some(delay) => plan.commands[released].pause = Some(delay),
            None if count < i64::from(click.click_count) => plan.pipeline_last(),
            None => {}
        }
    }
}

/// Playwright: `server/chromium/crInput.ts` `RawMouseImpl.wheel`: `type: 'mouseWheel',`
///
/// One wheel event at the pointer's position, with the held modifiers.
pub(crate) fn plan_wheel(plan: &mut Plan, simulated: &mut InputDevices, delta_x: f64, delta_y: f64) {
    let params = DispatchMouseEventParams {
        r#type: "mouseWheel".to_string(),
        x: simulated.mouse.x,
        y: simulated.mouse.y,
        modifiers: Some(i64::from(simulated.keyboard.modifiers)),
        delta_x: Some(delta_x),
        delta_y: Some(delta_y),
        ..DispatchMouseEventParams::default()
    };
    plan.push(simulated, mouse_command(&params, Effect::None));
}

/// Playwright: `server/chromium/crInput.ts` `RawTouchscreenImpl.tap`
///
/// A touch start with one point at `at` and the end with none, pipelined, both with the held
/// modifiers. Touch moves no mouse pointer in Playwright's state.
pub(crate) fn plan_tap(plan: &mut Plan, simulated: &mut InputDevices, at: (f64, f64)) {
    let modifiers = simulated.keyboard.modifiers;
    // Playwright: `server/chromium/crInput.ts` `RawTouchscreenImpl.tap`: `type: 'touchStart',`
    let start = json!({
        "type": "touchStart",
        "modifiers": modifiers,
        "touchPoints": [{ "x": at.0, "y": at.1 }],
    });
    // Playwright: `server/chromium/crInput.ts` `RawTouchscreenImpl.tap`: `type: 'touchEnd',`
    let end = json!({ "type": "touchEnd", "modifiers": modifiers, "touchPoints": [] });
    let started = plan.push(
        simulated,
        Planned {
            with_next: true,
            ..Planned::command("Input.dispatchTouchEvent", start.to_string(), Effect::None)
        },
    );
    plan.push(
        simulated,
        Planned {
            undoes: Some(started),
            ..Planned::command("Input.dispatchTouchEvent", end.to_string(), Effect::None)
        },
    );
}

/// Neo: the mouse half of a run's end. A release per button `owner` left held, oldest first, at
/// the pointer's last position. Playwright keeps a page's buttons held for its life; Neo's tab
/// outlives the run, and a held button would drag in the user's tab.
pub(crate) fn plan_button_releases(plan: &mut Plan, simulated: &mut InputDevices, owner: &Owner) {
    let held: Vec<MouseButton> = simulated
        .mouse
        .buttons
        .iter()
        .filter(|(_, pressed_by)| pressed_by == owner)
        .map(|(button, _)| *button)
        .collect();
    for button in held {
        plan_up(plan, simulated, button, 1, None);
    }
}

// ---------------------------------------------------------------------------------------------
// The writer.
// ---------------------------------------------------------------------------------------------

/// Where a writer's commands go: the run's session at its socket incarnation (`page.rs`), or a
/// recording fake under paused time in the tests.
pub(crate) trait InputSender: Send + Sync + 'static {
    /// Sends one `Input.*` command, its params already serialized.
    fn send_input(
        &self,
        method: &'static str,
        params: String,
    ) -> impl Future<Output = Result<(), CdpError>> + Send;
}

/// Where a writer records what it wrote: the tab's devices through the call's input turn, or a
/// test's own state.
pub(crate) trait EffectSink: Send + Sync {
    /// Applies `effect` (`InputDevices::apply`); `answered` when the command was acknowledged.
    fn apply(&self, effect: &Effect, answered: bool);
}

/// Writes `plan` in order, a batch at a time (a command and every next one it is written with),
/// applying each written command's effect to `sink`. After `cut_short` (the call stopped) or a
/// failed write, it writes only the releases it owes: those whose command may have been
/// written, newest planned last as they come. A refused write (`ConnectionEpochChanged`, raised
/// before writing) ends everything: the socket that carried the session is gone. A `pause` is
/// cut short too. Returns every written command's outcome, in order.
pub(crate) async fn write_plan(
    sender: &impl InputSender,
    sink: &impl EffectSink,
    plan: &Plan,
    cut_short: &CancellationToken,
) -> Vec<Result<(), CdpError>> {
    let commands = &plan.commands;
    let mut written = vec![false; commands.len()];
    let mut outcomes = Vec::with_capacity(commands.len());
    let mut stopped = false;
    let mut index = 0;
    while index < commands.len() {
        let mut end = index;
        while commands[end].with_next && end + 1 < commands.len() {
            end += 1;
        }
        let mut batch = Vec::with_capacity(end + 1 - index);
        for (offset, command) in commands[index..=end].iter().enumerate() {
            let at = index + offset;
            let owed = command.undoes.is_some_and(|undone| written[undone]);
            if command.undoes.is_some() && !owed {
                // It releases a command that never went out.
                continue;
            }
            if let Some(delay) = command.pause
                && !stopped
            {
                pause(delay, cut_short).await;
            }
            if cut_short.is_cancelled() {
                stopped = true;
            }
            if stopped && !owed {
                continue;
            }
            batch.push(at);
        }
        let mut sends = Vec::with_capacity(batch.len());
        for &at in &batch {
            let command = &commands[at];
            match command.method {
                Some(method) => sends.push((at, Some(sender.send_input(method, command.params.clone())))),
                None => sends.push((at, None)),
            }
        }
        let results = future::join_all(sends.into_iter().map(|(at, send)| async move {
            match send {
                Some(send) => (at, Some(send.await)),
                None => (at, None),
            }
        }))
        .await;
        let mut refused = false;
        for (at, result) in results {
            let command = &commands[at];
            let Some(result) = result else {
                // A change of state alone, never refused or failed.
                sink.apply(&command.effect, true);
                continue;
            };
            if matches!(result, Err(CdpError::ConnectionEpochChanged { .. })) {
                refused = true;
            } else {
                written[at] = true;
                sink.apply(&command.effect, result.is_ok());
            }
            if result.is_err() {
                stopped = true;
            }
            outcomes.push(result);
        }
        if refused {
            break;
        }
        index = end + 1;
    }
    outcomes
}

/// What a writer reports: every written command acknowledged → `ticket.acked()`; nothing sent,
/// or every send refused with `ConnectionEpochChanged` → `ticket.unsent()`; otherwise the ticket
/// drops (`Unknown`) and the first error is reported.
pub(crate) fn report(outcomes: &[Result<(), CdpError>], ticket: InputTicket) -> PressReport {
    let refused = |outcome: &Result<(), CdpError>| {
        matches!(outcome, Err(CdpError::ConnectionEpochChanged { .. }))
    };
    match outcomes.iter().find_map(|outcome| outcome.as_ref().err()) {
        None if outcomes.is_empty() => {
            ticket.unsent();
            PressReport::NothingWritten
        }
        None => PressReport::Acked(ticket.acked()),
        Some(_) if outcomes.iter().all(refused) => {
            ticket.unsent();
            PressReport::NothingWritten
        }
        Some(error) => {
            drop(ticket);
            PressReport::Failed {
                closed: world::classify(error) == PageError::Closed,
                message: error.to_string(),
            }
        }
    }
}

/// A `delay` sleep that the call stopping ends early.
async fn pause(delay: Duration, cut_short: &CancellationToken) {
    tokio::select! {
        () = time::sleep(delay) => {}
        () = cut_short.cancelled() => {}
    }
}
