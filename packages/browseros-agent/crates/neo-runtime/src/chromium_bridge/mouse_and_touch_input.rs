//! Playwright: `server/input.ts` `Mouse`
//! Playwright: `server/input.ts` `Touchscreen`
//! Playwright: `server/chromium/crInput.ts` `RawMouseImpl`
//! Playwright: `server/chromium/crInput.ts` `RawTouchscreenImpl`
//! Neo: a tab's keyboard and mouse between calls (the input-device seam of med-pw-dom's
//! design, section 4), the plans every input call makes from them, and the one writer that
//! sends a plan. Playwright keeps one `Keyboard` and one `Mouse` per page; Neo keeps their state
//! per tab per `Runtime`, inside the tab's input turn (`page.rs` `TabInputs`), so only a call
//! holding the turn reads or changes it.
//!
//! ```text
//!  call holds the turn ─▶ plan from a copy of the devices (each command applied to the copy as
//!                         it is planned, so every later command sees the held modifiers, the
//!                         held buttons and the pointer where the earlier ones leave them)
//!                       ─▶ start_writer: a detached task sends the commands in order, applying
//!                         each one's effect to the real devices as it is written, so the state
//!                         is what the browser saw however the call ends
//! ```
//!
//! A plan's releases are owed: a release (`keyUp`, `mouseReleased`, `touchEnd`, a modifier
//! restore) names the command it undoes, and is written even after the call stopped or a write
//! failed, whenever that command may have been written. Nothing else is written once the writer
//! stopped. Keys and buttons a call presses without releasing (`keyboard.down`, `mouse.down`)
//! stay held, tagged with the run that pressed them, until a later `up` or the run's end
//! (`Dispatcher::finish`). Delivery is `Done` only once every planned command was written and
//! acknowledged; a call that stopped part-way is `Unknown` (design section 4.7).

use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::time::Duration;

use browseros_cdp::{CdpError, SessionId};
use futures_util::future;
use serde_json::Value;
use tokio::sync::oneshot;
use tokio::time;
use tokio_util::sync::CancellationToken;

use crate::chromium_bridge::cdp::input::{self, DispatchMouseEvent};
use crate::chromium_bridge::cdp::{self, Command};
use crate::chromium_bridge::keyboard_input::TextTail;
use crate::chromium_bridge::playwright_injected_methods;
use crate::playwright_methods::click::{MouseButton, MouseClick, PageError, PressReport};
use crate::progress::InputTicket;

/// The run that pressed a key or a button: its own CDP session on the tab, since every run
/// attaches a session of its own.
pub(crate) type Owner = SessionId;

/// One tab's keyboard and mouse. A fresh one is Playwright's new page: the pointer at (0, 0),
/// nothing held.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct InputDevices {
    pub(crate) keyboard: KeyboardState,
    pub(crate) mouse: MouseState,
    /// Runs whose end was recorded. What one holds may still be down, but no live run holds
    /// it (a dead hold), so the next run's end releases it and no live run's release waits for
    /// it. A run is forgotten only between turns, once it lists no hold
    /// ([`InputDevices::forget_ended`]): within a turn, a late answer to a write the run had
    /// started, or a restore planned from a hold it had, records its hold the same way.
    pub(crate) ended: Vec<Owner>,
    /// Chromium's drag manager (Playwright's `DragManager`): the drag the browser handed
    /// over, while one runs.
    pub(crate) drag: Option<DragState>,
}

/// A drag Chromium intercepted (`Input.dragIntercepted`): its data, which every later drag
/// event carries, and where its last one went (`_lastPosition`, for a cancel).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DragState {
    pub(crate) data: Value,
    pub(crate) x: f64,
    pub(crate) y: f64,
}

/// Playwright's `Keyboard` state.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct KeyboardState {
    /// `_pressedModifiers`, as CDP's modifiers mask (`keyboard_input.rs` `ALT` … `SHIFT`).
    pub(crate) modifiers: u8,
    /// `_pressedKeys`: every held key's code, oldest first, once for each run that holds it,
    /// so a run's end releases a key only when no other live run holds it too.
    pub(crate) pressed: Vec<(&'static str, Owner)>,
}

/// Playwright's `Mouse` state.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct MouseState {
    pub(crate) x: f64,
    pub(crate) y: f64,
    /// `_lastButton`; `None` is Playwright's `'none'`.
    pub(crate) last_button: Option<MouseButton>,
    /// `_buttons`, oldest first, once for each run that holds the button.
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
    /// The key went up: no run holds it any more.
    KeyUp {
        code: &'static str,
        modifier: u8,
    },
    /// One more run holds an already held key (a modifier's restore for every run that held
    /// it). Writes nothing.
    KeyHolder {
        code: &'static str,
        owner: Owner,
    },
    /// A run's end lets go of a key another live run still holds. Writes nothing.
    KeyLetGo {
        code: &'static str,
        owner: Owner,
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
    /// A run's end lets go of a button another live run still holds. Writes nothing.
    ButtonLetGo {
        button: MouseButton,
        owner: Owner,
    },
    /// The page started a drag Chromium handed over, at the move's point.
    DragStarted {
        data: Value,
        x: f64,
        y: f64,
    },
    /// A `dragOver` went to the point.
    DragMoved {
        x: f64,
        y: f64,
    },
    /// A `drop` or a `dragCancel` ended the drag.
    DragEnded,
    /// A run ended: what it holds, now or once a write it started answers, is held by no live
    /// run any more, only possibly down (`InputDevices::ended`). Writes nothing.
    Retire {
        owner: Owner,
    },
}

/// Neo: the effect of each written command on the tab's state, which no script reads directly.
impl InputDevices {
    /// Applies a command's effect. A press counts from its write, answered or not, since it may
    /// have reached the browser; a release and a move count only once answered, so a key whose
    /// release failed stays held and the run's end releases it again. Pressing a held key again
    /// adds its run as one more holder: the key is down once (Playwright's `_pressedKeys` is a
    /// set), and a release lets every holder go.
    pub(crate) fn apply(&mut self, effect: &Effect, answered: bool) {
        self.apply_effect(effect, answered);
    }

    /// Neo: drops every ended run that lists no hold. Called as a turn ends, never within one:
    /// a turn's plans name the holders they found, so a run they name stays ended through it.
    pub(crate) fn forget_ended(&mut self) {
        let ended = mem::take(&mut self.ended);
        self.ended = ended
            .into_iter()
            .filter(|owner| self.holds_anything(owner))
            .collect();
    }

    /// Neo: records `owner`'s end (`Effect::Retire`), whatever it holds now, and forgets no
    /// other ended run: another's turn may still be writing.
    pub(crate) fn mark_ended(&mut self, owner: &Owner) {
        if !self.ended.contains(owner) {
            self.ended.push(owner.clone());
        }
    }

    fn apply_effect(&mut self, effect: &Effect, answered: bool) {
        match effect {
            Effect::None => {}
            Effect::KeyDown {
                code,
                modifier,
                owner,
            } => {
                self.keyboard.modifiers |= modifier;
                self.hold_key(code, owner);
            }
            Effect::KeyUp { code, modifier } => {
                if answered {
                    self.keyboard.modifiers &= !modifier;
                    self.keyboard.pressed.retain(|(held, _)| held != code);
                }
            }
            Effect::KeyHolder { code, owner } => self.hold_key(code, owner),
            Effect::KeyLetGo { code, owner } => {
                self.keyboard
                    .pressed
                    .retain(|(held, holder)| (held, holder) != (code, owner));
            }
            Effect::Moved { x, y } => {
                if answered {
                    self.mouse.x = *x;
                    self.mouse.y = *y;
                }
            }
            Effect::ButtonDown { button, owner } => {
                self.mouse.last_button = Some(*button);
                if !self
                    .mouse
                    .buttons
                    .iter()
                    .any(|(held, holder)| (held, holder) == (button, owner))
                {
                    self.mouse.buttons.push((*button, owner.clone()));
                }
            }
            Effect::ButtonUp { button } => {
                if answered {
                    self.mouse.last_button = None;
                    self.mouse.buttons.retain(|(held, _)| held != button);
                }
            }
            Effect::ButtonLetGo { button, owner } => {
                self.mouse
                    .buttons
                    .retain(|(held, holder)| (held, holder) != (button, owner));
            }
            Effect::DragStarted { data, x, y } => {
                self.drag = Some(DragState {
                    data: data.clone(),
                    x: *x,
                    y: *y,
                });
            }
            Effect::DragMoved { x, y } => {
                if let Some(drag) = self.drag.as_mut()
                    && answered
                {
                    (drag.x, drag.y) = (*x, *y);
                }
            }
            Effect::DragEnded => {
                // Playwright clears its drag state only once the drop or cancel was answered.
                if answered {
                    self.drag = None;
                }
            }
            Effect::Retire { owner } => self.mark_ended(owner),
        }
    }

    /// Whether `owner` is listed as holding any key or button.
    fn holds_anything(&self, owner: &Owner) -> bool {
        self.keyboard
            .pressed
            .iter()
            .any(|(_, holder)| holder == owner)
            || self.mouse.buttons.iter().any(|(_, holder)| holder == owner)
    }

    /// Whether `holder` is a live run other than `owner`: one whose end has not come.
    pub(crate) fn live_other(&self, holder: &Owner, owner: &Owner) -> bool {
        holder != owner && !self.ended.contains(holder)
    }

    /// Whether `holder`'s holds are `owner`'s to release at its end: its own, or an ended
    /// run's.
    pub(crate) fn releases(&self, holder: &Owner, owner: &Owner) -> bool {
        holder == owner || self.ended.contains(holder)
    }

    /// `code` held by `owner` too, once.
    fn hold_key(&mut self, code: &'static str, owner: &Owner) {
        if !self
            .keyboard
            .pressed
            .iter()
            .any(|(held, holder)| *held == code && holder == owner)
        {
            self.keyboard.pressed.push((code, owner.clone()));
        }
    }

    /// `toButtonsMask` of the held buttons, each once whatever runs hold it.
    pub(crate) fn buttons_mask(&self) -> i64 {
        self.mouse
            .buttons
            .iter()
            .fold(0, |mask, (button, _)| mask | button_mask(*button))
    }
}

/// One command of a plan, decided before anything is written.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Planned {
    /// The write, its params already JSON and written as planned, or `None` for a change of
    /// state that writes nothing (a move of no steps still moves Playwright's pointer).
    pub(crate) input: Option<PlannedInput>,
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
    /// Written even after the writer stopped: a run's end releases every key and button it can,
    /// each independent of the others.
    pub(crate) always: bool,
    /// What Chromium's drag manager makes of the command, decided as it is written.
    pub(crate) drag: DragStep,
}

/// Neo: one planned write: an ordinary write, or a release ([`cdp::Release`]): a `keyUp`, a
/// `mouseReleased` or a `dragCancel`, which a run's end writes with no call's delivery record
/// ([`Pen::RunEnd`]). A call writes either under its own record.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PlannedInput {
    Write(cdp::Input),
    Release(cdp::ReleaseInput),
}

/// Neo: a planned write as it goes out, and as a stop patches it.
impl PlannedInput {
    /// The write as it goes out under a call's delivery record, a release's included.
    pub(crate) fn input(&self) -> &cdp::Input {
        match self {
            Self::Write(input) => input,
            Self::Release(release) => release.input(),
        }
    }

    /// [`cdp::Input::with_number`], keeping a release a release.
    fn with_number(&self, name: &str, value: i64) -> Self {
        match self {
            Self::Write(input) => Self::Write(input.with_number(name, value)),
            Self::Release(release) => Self::Release(release.with_number(name, value)),
        }
    }
}

/// Neo: what a writer writes under. A call's writes all go under its delivery record (the
/// ticket it began); a run's end has no call, so its releases go under none
/// ([`cdp::Session::release`]), and nothing but a release may go out there.
#[derive(Clone, Copy)]
pub(crate) enum Pen<'a> {
    Call(&'a InputTicket),
    RunEnd,
}

/// Neo: how a command meets a drag, decided as it is written (Playwright's `RawMouseImpl`,
/// `RawKeyboardImpl`), since a move may start one in the middle of a plan.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum DragStep {
    /// Written as planned: a click's own move (`forClick`) and every other command.
    #[default]
    None,
    /// A move that is not a click's: `dragOver` while a drag runs, the intercept protocol while
    /// the left button is held, else the move.
    Move,
    /// A mouse press: skipped while a drag runs.
    Down,
    /// A mouse release: `drop` while a drag runs.
    Up,
    /// A run end's mouse release: `dragCancel` while a drag runs.
    Cancel,
    /// An Escape keyDown: `dragCancel` in its place while a drag runs.
    Escape,
}

/// Neo: building a plan, which no script sees.
impl Planned {
    /// `command`, written on its own, undoing nothing, its params as its struct serializes them,
    /// so key events keep Playwright's field order ([`cdp::Input`]).
    pub(crate) fn write<C: Command<Via = cdp::Write>>(command: &C, effect: Effect) -> Self {
        Self::of(PlannedInput::Write(cdp::Input::new(command)), effect)
    }

    /// `command`, a release, written on its own, undoing nothing ([`PlannedInput::Release`]).
    pub(crate) fn release<C: Command<Via = cdp::Release>>(command: &C, effect: Effect) -> Self {
        Self::of(
            PlannedInput::Release(cdp::ReleaseInput::new(command)),
            effect,
        )
    }

    fn of(input: PlannedInput, effect: Effect) -> Self {
        Self {
            input: Some(input),
            effect,
            undoes: None,
            with_next: false,
            pause: None,
            always: false,
            drag: DragStep::None,
        }
    }

    /// The planned write's method, `None` for a change of state.
    #[cfg(test)]
    pub(crate) fn method(&self) -> Option<&'static str> {
        self.input.as_ref().map(|input| input.input().method())
    }

    /// The planned write's params, empty for a change of state.
    #[cfg(test)]
    pub(crate) fn params(&self) -> &str {
        self.input
            .as_ref()
            .map_or("", |input| input.input().params())
    }

    /// A change of state that writes nothing.
    pub(crate) fn state_only(effect: Effect) -> Self {
        Self {
            input: None,
            effect,
            undoes: None,
            with_next: false,
            pause: None,
            always: false,
            drag: DragStep::None,
        }
    }
}

/// The commands of one input call, in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Plan {
    pub(crate) commands: Vec<Planned>,
    /// The text `keyboard.type` has still to type, planned a chunk at a time as the writer
    /// reaches it (`keyboard_input.rs` `TextTail`), so a long text never holds a command per
    /// character in memory.
    pub(crate) tail: Option<TextTail>,
}

/// Neo: building a plan, which no script sees.
impl Plan {
    /// Plans `command` and applies it to `simulated`, as if written and answered, so the next
    /// command sees the devices this one leaves. Returns its index, for a later release.
    pub(crate) fn push(&mut self, simulated: &mut InputDevices, command: Planned) -> usize {
        simulated.apply(&command.effect, true);
        self.commands.push(command);
        self.commands.len() - 1
    }

    /// Makes the last command written together with the next one.
    pub(crate) fn pipeline_last(&mut self) {
        if let Some(last) = self.commands.last_mut() {
            last.with_next = true;
        }
    }

    /// The number of commands that write something, before any tail.
    pub(crate) fn writes(&self) -> usize {
        self.commands
            .iter()
            .filter(|command| command.input.is_some())
            .count()
    }
}

// ---------------------------------------------------------------------------------------------
// The mouse and the touchscreen, planned from the devices.
// ---------------------------------------------------------------------------------------------

/// Playwright: `server/chromium/crProtocolHelper.ts` `toButtonsMask`: `mask |= 1;`
fn button_mask(button: MouseButton) -> i64 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Right => 2,
        MouseButton::Middle => 4,
    }
}

/// Neo: the protocol's own names for the three buttons, as Playwright passes them through.
fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
    }
}

/// Playwright: `server/input.ts` `Mouse.move`: `const middleX = fromX + (x - fromX) * (i / steps);`
///
/// `steps` moves from the pointer's last position to `to`, each a `RawMouseImpl.move`: the last
/// button or `none`, the held buttons, the held modifiers, and force while a button is held.
/// No steps moves the pointer without an event, as Playwright's loop does.
pub(crate) fn plan_move(plan: &mut Plan, simulated: &mut InputDevices, to: (f64, f64), steps: u32) {
    let (from_x, from_y) = (simulated.mouse.x, simulated.mouse.y);
    if steps == 0 {
        plan.push(
            simulated,
            Planned::state_only(Effect::Moved { x: to.0, y: to.1 }),
        );
        return;
    }
    for step in 1..=steps {
        let fraction = f64::from(step) / f64::from(steps);
        let x = from_x + (to.0 - from_x) * fraction;
        let y = from_y + (to.1 - from_y) * fraction;
        let held = !simulated.mouse.buttons.is_empty();
        // Playwright: `server/chromium/crInput.ts` `RawMouseImpl.move`: `force: buttons.size > 0 ? 0.5 : 0,`
        let force = if held { 0.5 } else { 0.0 };
        let params = DispatchMouseEvent {
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
            force: Some(force),
            ..DispatchMouseEvent::default()
        };
        let command = Planned::write(&params, Effect::Moved { x, y });
        plan.push(simulated, command);
    }
}

/// Playwright: `server/chromium/crInput.ts` `RawMouseImpl.move`: `await this._dragManager.interceptDragCausedByMove(progress, x, y, button, buttons, modifiers, actualMove);`
///
/// A move that is not a click's (`hover`, a drag's two moves, `mouse.move`): as [`plan_move`],
/// each step meeting Chromium's drag manager as it is written.
pub(crate) fn plan_free_move(
    plan: &mut Plan,
    simulated: &mut InputDevices,
    to: (f64, f64),
    steps: u32,
) {
    let first = plan.commands.len();
    plan_move(plan, simulated, to, steps);
    for command in &mut plan.commands[first..] {
        if command.input.is_some() {
            command.drag = DragStep::Move;
        }
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
    let effect = Effect::ButtonDown {
        button,
        owner: owner.clone(),
    };
    let mut pressed = simulated.clone();
    pressed.apply(&effect, true);
    // Playwright: `server/chromium/crInput.ts` `RawMouseImpl.down`: `force: buttons.size > 0 ? 0.5 : 0,`
    let force = 0.5;
    let params = DispatchMouseEvent {
        r#type: "mousePressed".to_string(),
        x: simulated.mouse.x,
        y: simulated.mouse.y,
        modifiers: Some(i64::from(simulated.keyboard.modifiers)),
        button: Some(button_name(button).to_string()),
        buttons: Some(pressed.buttons_mask()),
        click_count: Some(click_count),
        force: Some(force),
        ..DispatchMouseEvent::default()
    };
    // Playwright: `server/chromium/crInput.ts` `RawMouseImpl.down`: `if (this._dragManager.isDragging())`
    let command = Planned {
        drag: DragStep::Down,
        ..Planned::write(&params, effect)
    };
    plan.push(simulated, command)
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
    let effect = Effect::ButtonUp { button };
    let mut released = simulated.clone();
    released.apply(&effect, true);
    let release = input::ReleaseMouse::new(
        (simulated.mouse.x, simulated.mouse.y),
        button_name(button),
        released.buttons_mask(),
        click_count,
        i64::from(simulated.keyboard.modifiers),
    );
    // Playwright: `server/chromium/crInput.ts` `RawMouseImpl.up`: `await this._dragManager.drop(progress, x, y, modifiers);`
    let command = Planned {
        undoes,
        drag: DragStep::Up,
        ..Planned::release(&release, effect)
    };
    plan.push(simulated, command)
}

/// Playwright: `server/input.ts` `Mouse.click`: `for (let cc = 1; cc <= clickCount; ++cc) {`
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
pub(crate) fn plan_wheel(
    plan: &mut Plan,
    simulated: &mut InputDevices,
    delta_x: f64,
    delta_y: f64,
) {
    let params = DispatchMouseEvent {
        r#type: "mouseWheel".to_string(),
        x: simulated.mouse.x,
        y: simulated.mouse.y,
        modifiers: Some(i64::from(simulated.keyboard.modifiers)),
        delta_x: Some(delta_x),
        delta_y: Some(delta_y),
        ..DispatchMouseEvent::default()
    };
    plan.push(simulated, Planned::write(&params, Effect::None));
}

/// Playwright: `server/chromium/crInput.ts` `RawTouchscreenImpl.tap`: `await progress.race(Promise.all([`
///
/// A touch start with one point at `at` and the end with none, pipelined, both with the held
/// modifiers. Touch moves no mouse pointer in Playwright's state.
pub(crate) fn plan_tap(plan: &mut Plan, simulated: &mut InputDevices, at: (f64, f64)) {
    let modifiers = simulated.keyboard.modifiers;
    // Playwright: `server/chromium/crInput.ts` `RawTouchscreenImpl.tap`: `type: 'touchStart',`
    let start = input::DispatchTouchEvent {
        modifiers,
        touch_points: vec![input::TouchPoint { x: at.0, y: at.1 }],
        kind: "touchStart",
    };
    // Playwright: `server/chromium/crInput.ts` `RawTouchscreenImpl.tap`: `touchPoints: []`
    let end = input::DispatchTouchEvent {
        modifiers,
        touch_points: Vec::new(),
        kind: "touchEnd",
    };
    let started = plan.push(
        simulated,
        Planned {
            with_next: true,
            ..Planned::write(&start, Effect::None)
        },
    );
    plan.push(
        simulated,
        Planned {
            undoes: Some(started),
            ..Planned::write(&end, Effect::None)
        },
    );
}

/// Deviation: `server/input.ts` `Mouse.up`: `this._buttons.delete(button);` Playwright keeps a page's buttons held until its script releases them; Neo releases at a run's end every button the run left held, oldest first at the pointer's last position, because the tab is the user's and outlives the run.
pub(crate) fn plan_button_releases(plan: &mut Plan, simulated: &mut InputDevices, owner: &Owner) {
    // First, before any release, as for keys: the run is over whatever its releases do.
    retire(plan, simulated, owner);
    let mut held: Vec<MouseButton> = Vec::new();
    for (button, holder) in &simulated.mouse.buttons {
        if simulated.releases(holder, owner) && !held.contains(button) {
            held.push(*button);
        }
    }
    for button in held {
        // A button another live run still holds stays down; this run only lets go of it.
        let shared = simulated
            .mouse
            .buttons
            .iter()
            .any(|(held, holder)| *held == button && simulated.live_other(holder, owner));
        let at = if shared {
            if !simulated.mouse.buttons.contains(&(button, owner.clone())) {
                continue;
            }
            plan.push(
                simulated,
                Planned::state_only(Effect::ButtonLetGo {
                    button,
                    owner: owner.clone(),
                }),
            )
        } else {
            let at = plan_up(plan, simulated, button, 1, None);
            // Only the left button holds a drag: its release cancels the drag, never drops it.
            // Another button's release neither drops nor cancels a running drag, which belongs
            // to whichever run holds the left button.
            plan.commands[at].drag = if button == MouseButton::Left {
                DragStep::Cancel
            } else {
                DragStep::None
            };
            at
        };
        plan.commands[at].always = true;
    }
}

/// Neo: a run end's last command: what the run still holds after its releases (one failed)
/// is held by no live run any more. Written even after the writer stopped, as every release
/// before it.
fn retire(plan: &mut Plan, simulated: &mut InputDevices, owner: &Owner) {
    let at = plan.push(
        simulated,
        Planned::state_only(Effect::Retire {
            owner: owner.clone(),
        }),
    );
    plan.commands[at].always = true;
}

/// Neo: [`retire`] for `keyboard_input.rs`'s key releases.
pub(crate) fn plan_retire(plan: &mut Plan, simulated: &mut InputDevices, owner: &Owner) {
    retire(plan, simulated, owner);
}

// ---------------------------------------------------------------------------------------------
// The seam a turn provides, and the writer.
// ---------------------------------------------------------------------------------------------

/// Neo: where a writer's commands go, the run's session at its socket incarnation (`page.rs`,
/// through `cdp::Session::write`), or a recording fake under paused time in the tests. Every
/// write carries the call's delivery record.
pub(crate) trait InputSender: Send + Sync + 'static {
    /// Writes one planned `Input.*` command under `pen`.
    fn send_input(
        &self,
        input: &PlannedInput,
        pen: Pen<'_>,
    ) -> impl Future<Output = Result<(), CdpError>> + Send;

    /// Playwright's `DragManager.interceptDragCausedByMove` around one `mouseMoved` (`moved`)
    /// made with the left button held: the drag Chromium handed over when the move started one
    /// the page did not prevent, else `None`. A sender that cannot see the page just moves.
    fn move_intercepting_drag(
        &self,
        moved: &PlannedInput,
        delivery: &InputTicket,
    ) -> impl Future<Output = Result<Option<Value>, CdpError>> + Send {
        async move {
            self.send_input(moved, Pen::Call(delivery))
                .await
                .map(|()| None)
        }
    }
}

/// Neo: where a writer records what it wrote, the tab's devices through the call's input turn,
/// or a test's own state.
pub(crate) trait EffectSink: Send + Sync {
    /// Applies `effect` (`InputDevices::apply`); `answered` when the command was acknowledged.
    fn apply(&self, effect: &Effect, answered: bool);

    /// The devices as the writes so far left them, for a release written after a stop, whose
    /// planned masks assumed every earlier command went out.
    fn devices(&self) -> InputDevices;

    /// Whether the run whose turn writes has ended (`InputDevices::ended`): its writer then
    /// writes only what it owes, as after a stop.
    fn run_ended(&self) -> bool {
        false
    }
}

/// Neo: what a turn at a tab's input gives a call, the tab's devices to plan from and to record
/// into, and who holds the turn. Only a held turn reaches the devices.
pub(crate) trait Devices: EffectSink + Clone + Send + Sync + 'static {
    /// A copy of the tab's devices, to plan from.
    ///
    /// # Errors
    ///
    /// `PageError::Contract` when the state cannot be read.
    fn snapshot(&self) -> Result<InputDevices, PageError>;

    /// The run that holds the turn.
    fn owner(&self) -> Owner;
}

/// A writer's outcome: every written command's answer, in order, and whether every planned
/// command was written.
#[derive(Debug, Default)]
pub(crate) struct Written {
    pub(crate) outcomes: Vec<Result<(), CdpError>>,
    pub(crate) complete: bool,
}

/// Neo: the writer, which writes a plan in order, a batch at a time (a command and every next
/// one it is written with), applying each written command's effect to `sink`, then the plan's
/// text tail a chunk at a time. After `cut_short` (the call stopped) or a failed write, it
/// writes only what it owes: releases whose command may have been written, and a run end's
/// releases, each independent of the others; each with the modifiers and buttons the tab
/// actually holds, since the plan assumed every earlier command went out. A refused write
/// (`ConnectionEpochChanged`, raised before writing) ends everything: the socket that carried
/// the session is gone. A `pause` is cut short too.
pub(crate) async fn write_plan(
    sender: &impl InputSender,
    pen: Pen<'_>,
    sink: &impl EffectSink,
    plan: &Plan,
    cut_short: &CancellationToken,
) -> Written {
    let mut writer = Writer::default();
    writer
        .write(sender, pen, sink, &plan.commands, cut_short)
        .await;
    let mut planned = plan.writes();
    let mut typed = true;
    if let Some(tail) = &plan.tail {
        let mut tail = tail.clone();
        loop {
            if cut_short.is_cancelled() || sink.run_ended() {
                writer.stopped = true;
            }
            if writer.stopped || writer.refused {
                // The rest of the text never goes out.
                typed = tail.is_empty();
                break;
            }
            let Some(chunk) = tail.next_chunk() else {
                break;
            };
            planned += chunk.writes();
            writer
                .write(sender, pen, sink, &chunk.commands, cut_short)
                .await;
        }
    }
    let complete = !writer.refused && typed && writer.outcomes.len() == planned;
    Written {
        outcomes: writer.outcomes,
        complete,
    }
}

/// Neo: one writer's progress across a plan and its tail's chunks.
#[derive(Default)]
struct Writer {
    outcomes: Vec<Result<(), CdpError>>,
    stopped: bool,
    refused: bool,
}

/// Neo: one command as the writer sends it: as planned (its params patched after a stop),
/// replaced by a drag event, or written as nothing (a press while a drag runs).
enum Outgoing {
    Planned(Option<PlannedInput>),
    Drag { input: PlannedInput, ends: bool },
    Skipped,
}

/// Neo: writing a plan's commands and its tail's chunks, each chunk's `undoes` its own.
impl Writer {
    /// Writes `commands`, whose `undoes` indices are their own.
    async fn write(
        &mut self,
        sender: &impl InputSender,
        pen: Pen<'_>,
        sink: &impl EffectSink,
        commands: &[Planned],
        cut_short: &CancellationToken,
    ) {
        let mut written = vec![false; commands.len()];
        let mut index = 0;
        while index < commands.len() {
            if self.refused {
                // The socket that carried the session is gone: nothing more is written, but a
                // run end's bookkeeping (state only, written even after a stop) still applies.
                let command = &commands[index];
                if command.input.is_none() && command.always {
                    sink.apply(&command.effect, true);
                }
                index += 1;
                continue;
            }
            if commands[index].drag == DragStep::Move && !commands[index].with_next {
                // A move that is not a click's goes alone: it may start a drag. A move is never
                // owed, so nothing moves once the writer stopped.
                if cut_short.is_cancelled() || sink.run_ended() {
                    self.stopped = true;
                }
                if !self.stopped {
                    written[index] = self.free_move(sender, pen, sink, &commands[index]).await;
                }
                index += 1;
                continue;
            }
            let mut end = index;
            while commands[end].with_next && end + 1 < commands.len() {
                end += 1;
            }
            let mut batch: Vec<(usize, Outgoing)> = Vec::with_capacity(end + 1 - index);
            // What the tab holds after the batch so far, for releases after a stop and for a
            // drag the batch's commands meet.
            let mut actual: Option<InputDevices> = None;
            for (at, command) in commands.iter().enumerate().take(end + 1).skip(index) {
                // What it undoes went out before, or goes out in this very batch (a pipelined
                // click's release, a tap's end).
                let undone = command.undoes.is_some_and(|undone| {
                    written[undone] || batch.iter().any(|(sent, _)| *sent == undone)
                });
                if command.undoes.is_some() && !undone {
                    // It releases a command that never went out.
                    continue;
                }
                let owed = command.always || undone;
                if let Some(delay) = command.pause
                    && !self.stopped
                {
                    pause(delay, cut_short).await;
                }
                if cut_short.is_cancelled() || sink.run_ended() {
                    self.stopped = true;
                }
                if self.stopped && !owed {
                    continue;
                }
                let devices = actual.get_or_insert_with(|| sink.devices());
                let outgoing = drag_outgoing(command, devices).unwrap_or_else(|| {
                    Outgoing::Planned(
                        self.stopped
                            .then(|| {
                                let mut after = devices.clone();
                                after.apply(&command.effect, true);
                                actual_params(command, &after)
                            })
                            .flatten(),
                    )
                });
                devices.apply(&command.effect, true);
                if let Outgoing::Drag { ends: true, .. } = outgoing {
                    devices.apply(&Effect::DragEnded, true);
                }
                batch.push((at, outgoing));
            }
            // Each command's write as it goes out, owned here so the sends below can borrow it.
            let outgoing: Vec<(usize, Option<PlannedInput>, bool)> = batch
                .into_iter()
                .map(|(at, outgoing)| match outgoing {
                    Outgoing::Skipped => (at, None, false),
                    Outgoing::Drag { input, ends } => (at, Some(input), ends),
                    Outgoing::Planned(patched) => {
                        (at, patched.or_else(|| commands[at].input.clone()), false)
                    }
                })
                .collect();
            let sends = outgoing.iter().map(|(at, input, ends)| async move {
                match input {
                    Some(input) => (*at, Some(sender.send_input(input, pen).await), *ends),
                    None => (*at, None, *ends),
                }
            });
            for (at, result, ends) in future::join_all(sends).await {
                written[at] = self.record(sink, &commands[at], result, ends);
            }
            index = end + 1;
        }
    }

    /// One batch command's answer, `None` for one that wrote nothing: a change of state alone,
    /// or a press a drag skipped, never refused or failed, answered and its release owed as if
    /// it went out. Whether it counts as written.
    fn record(
        &mut self,
        sink: &impl EffectSink,
        command: &Planned,
        result: Option<Result<(), CdpError>>,
        ends: bool,
    ) -> bool {
        let Some(result) = result else {
            sink.apply(&command.effect, true);
            if command.input.is_none() {
                return false;
            }
            // Playwright: `server/chromium/crInput.ts` `RawMouseImpl.down`: `if (this._dragManager.isDragging())`
            // A press a running drag skipped returns as if answered, and counts as written.
            self.outcomes.push(Ok(()));
            return true;
        };
        let refused = matches!(result, Err(CdpError::ConnectionEpochChanged { .. }));
        if refused {
            self.refused = true;
        } else {
            sink.apply(&command.effect, result.is_ok());
            if ends {
                sink.apply(&Effect::DragEnded, result.is_ok());
            }
        }
        if result.is_err() {
            self.stopped = true;
        }
        self.outcomes.push(result);
        !refused
    }

    /// Playwright: `server/chromium/crDragDrop.ts` `DragManager.interceptDragCausedByMove`: `if (this._dragState) {`
    ///
    /// One move that is not a click's: `dragOver` while a drag runs, the intercept protocol
    /// with the left button held (and `dragEnter` when it started a drag), else the move.
    /// Whether it went out (a refused write did not).
    async fn free_move(
        &mut self,
        sender: &impl InputSender,
        pen: Pen<'_>,
        sink: &impl EffectSink,
        command: &Planned,
    ) -> bool {
        let (Some(input), Effect::Moved { x, y }) = (&command.input, &command.effect) else {
            return false;
        };
        // A move is never owed, so a run's end, which writes only what it owes, never moves.
        let Pen::Call(delivery) = pen else {
            return false;
        };
        let (x, y) = (*x, *y);
        let devices = sink.devices();
        let modifiers = i64::from(devices.keyboard.modifiers);
        let planned: Value = serde_json::from_str(input.input().params()).unwrap_or_default();
        let result = if let Some(drag) = &devices.drag {
            let over = drag_event("dragOver", x, y, &drag.data, modifiers);
            let result = sender.send_input(&over, pen).await;
            sink.apply(&command.effect, result.is_ok());
            sink.apply(&Effect::DragMoved { x, y }, result.is_ok());
            result
        } else if planned["button"] == "left" {
            match sender.move_intercepting_drag(input, delivery).await {
                Ok(Some(data)) => {
                    sink.apply(&command.effect, true);
                    let enter = drag_event("dragEnter", x, y, &data, modifiers);
                    sink.apply(&Effect::DragStarted { data, x, y }, true);
                    sender.send_input(&enter, pen).await
                }
                Ok(None) => {
                    sink.apply(&command.effect, true);
                    Ok(())
                }
                Err(error) => {
                    sink.apply(&command.effect, false);
                    Err(error)
                }
            }
        } else {
            let result = sender.send_input(input, pen).await;
            sink.apply(&command.effect, result.is_ok());
            result
        };
        let refused = matches!(result, Err(CdpError::ConnectionEpochChanged { .. }));
        if refused {
            self.refused = true;
        }
        if result.is_err() {
            self.stopped = true;
        }
        self.outcomes.push(result);
        !refused
    }
}

/// Neo: what a drag makes of a press, a release or an Escape while one runs (Playwright's
/// `RawMouseImpl.down`/`up`, `RawKeyboardImpl.keydown`): the press is skipped, the release
/// drops, a run end's release and the Escape cancel; `None` writes the command as planned.
fn drag_outgoing(command: &Planned, devices: &InputDevices) -> Option<Outgoing> {
    let drag = devices.drag.as_ref()?;
    let modifiers = i64::from(devices.keyboard.modifiers);
    let planned = || {
        command
            .input
            .as_ref()
            .and_then(|input| serde_json::from_str::<Value>(input.input().params()).ok())
            .unwrap_or_default()
    };
    match command.drag {
        DragStep::Down => Some(Outgoing::Skipped),
        DragStep::Up => {
            let planned = planned();
            let (x, y) = (
                planned["x"].as_f64().unwrap_or(drag.x),
                planned["y"].as_f64().unwrap_or(drag.y),
            );
            Some(Outgoing::Drag {
                input: drag_event("drop", x, y, &drag.data, modifiers),
                ends: true,
            })
        }
        DragStep::Cancel | DragStep::Escape => Some(Outgoing::Drag {
            input: PlannedInput::Release(cdp::ReleaseInput::new(&input::CancelDrag::new((
                drag.x, drag.y,
            )))),
            ends: true,
        }),
        DragStep::None | DragStep::Move => None,
    }
}

/// Neo: one `Input.dispatchDragEvent`, as Playwright's `DragManager` builds it: the event, its
/// point, the intercepted drag's data and the held modifiers.
fn drag_event(kind: &str, x: f64, y: f64, data: &Value, modifiers: i64) -> PlannedInput {
    PlannedInput::Write(cdp::Input::new(&input::DispatchDragEvent {
        data,
        modifiers: Some(modifiers),
        kind,
        x,
        y,
    }))
}

/// Neo: the `Input` methods the writer tells apart, named once, by their declarations.
const KEY: &str = input::KeyEvent::METHOD;
/// Neo: as [`KEY`].
const MOUSE: &str = DispatchMouseEvent::METHOD;
/// Neo: as [`KEY`].
const TOUCH: &str = input::DispatchTouchEvent::METHOD;

/// Neo: `command`'s write with the modifiers (and, for a mouse event, the buttons) that
/// `after`, the tab once this command is applied, holds; `None` for any other command. Only
/// the numbers change, in place, so the params keep Playwright's field order.
fn actual_params(command: &Planned, after: &InputDevices) -> Option<PlannedInput> {
    let input = command.input.as_ref()?;
    let modifiers = i64::from(after.keyboard.modifiers);
    match input.input().method() {
        KEY | TOUCH => Some(input.with_number("modifiers", modifiers)),
        MOUSE => Some(
            input
                .with_number("modifiers", modifiers)
                .with_number("buttons", after.buttons_mask()),
        ),
        _ => None,
    }
}

/// Neo: what every writer reports. The whole plan written and every command acknowledged →
/// `ticket.acked()`; nothing sent, or every send refused with `ConnectionEpochChanged` →
/// `ticket.unsent()`; otherwise the ticket drops (`Unknown`: design section 4.7, a call that
/// stopped part-way, a double click cut short after its first click included) and the first
/// error, or the cut, is reported.
pub(crate) fn report(written: &Written, ticket: InputTicket) -> PressReport {
    let refused = |outcome: &Result<(), CdpError>| {
        matches!(outcome, Err(CdpError::ConnectionEpochChanged { .. }))
    };
    let outcomes = &written.outcomes;
    match outcomes.iter().find_map(|outcome| outcome.as_ref().err()) {
        None if outcomes.is_empty() => {
            ticket.unsent();
            PressReport::NothingWritten
        }
        None if written.complete => PressReport::Acked(ticket.acked()),
        None => {
            // The call stopped part-way: what was written was answered, the rest never went.
            drop(ticket);
            PressReport::Failed {
                closed: false,
                message: "the input stopped before its last event".to_string(),
            }
        }
        Some(_) if outcomes.iter().all(refused) => {
            ticket.unsent();
            PressReport::NothingWritten
        }
        Some(error) => {
            drop(ticket);
            PressReport::Failed {
                closed: playwright_injected_methods::classify(error) == PageError::Closed,
                message: error.to_string(),
            }
        }
    }
}

/// A writer task, boxed so one spawning seam serves the run's tracker and the tests' runtime.
pub(crate) type WriterTask = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Neo: starts the writer of `plan` *now* through `spawn`, holding `turn` (which also records
/// the effects) until the last command is answered, and returns a future of its report that
/// holds a drop guard on the writer's `cut_short` token: dropping the future (the call
/// stopped) cuts a `delay` short and starts nothing new, while every owed release is still
/// written and the ticket still marked. `carried` (an armed hit-target interceptor) stays in
/// the writer until the last command is answered and comes back with the report; a caller that
/// stopped never gets it back, and it drops there. Starting eagerly matters: a future that
/// started the writer only when polled could be dropped unpolled by a `Progress::race` that had
/// already lost, leaving a begun ticket nobody marks.
pub(crate) fn start_writer<D: Devices, C: Send + 'static>(
    spawn: impl FnOnce(WriterTask),
    sender: impl InputSender,
    plan: Plan,
    ticket: InputTicket,
    turn: D,
    carried: C,
) -> impl Future<Output = (PressReport, Option<C>)> + Send {
    let cut_short = CancellationToken::new();
    let (report_sender, receiver) = oneshot::channel();
    let writer_cut_short = cut_short.clone();
    spawn(Box::pin(async move {
        let written =
            write_plan(&sender, Pen::Call(&ticket), &turn, &plan, &writer_cut_short).await;
        let reported = report(&written, ticket);
        // Every command is answered: the tab's next input may start.
        drop(turn);
        // Nobody may be waiting any more; the ticket already holds the outcome.
        let _waiting = report_sender.send((reported, carried));
    }));
    let cut_short = cut_short.drop_guard();
    async move {
        let reported = receiver.await.map_or_else(
            |_| {
                let failed = PressReport::Failed {
                    closed: true,
                    message: "the input writer stopped before reporting".to_string(),
                };
                (failed, None)
            },
            |(reported, carried)| (reported, Some(carried)),
        );
        drop(cut_short);
        reported
    }
}

/// Neo: a `delay` sleep that the call stopping ends early.
async fn pause(delay: Duration, cut_short: &CancellationToken) {
    tokio::select! {
        () = time::sleep(delay) => {}
        () = cut_short.cancelled() => {}
    }
}

#[cfg(test)]
mod tests {
    //! The devices' state transitions and the writer, under paused time against a recording
    //! sender: what each plan carries from the state (held modifiers and buttons, the pointer's
    //! last position), what a stopped or failed writer still owes, and what a run's end
    //! releases. Never timings beyond the scripted round trips.

    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use serde_json::Value;

    use super::*;
    use crate::chromium_bridge::keyboard_input::{self, Platform, SHIFT};
    use crate::progress::DeliveryCell;
    use crate::run::Delivery;

    /// Known review issue: `keyboard.type` plans a long text a chunk at a time, so its plan
    /// never holds a command per character; the writer types all of it, and a stop mid-text
    /// writes nothing past the key it was releasing.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_long_text_is_typed_a_chunk_at_a_time() {
        let text = "ab".repeat(500);
        let mut plan = Plan::default();
        keyboard_input::plan_type(
            &mut plan,
            &mut InputDevices::default(),
            &text,
            Platform::Linux,
            &run("A"),
            None,
        );
        assert_eq!(plan.commands.len(), 2 * 64, "the first chunk only");
        assert!(plan.tail.is_some());
        let turn = Turn::of(InputDevices::default(), "A");
        let sender = Sender::answering([]);
        let written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &plan,
            &CancellationToken::new(),
        )
        .await;
        assert!(written.complete);
        assert_eq!(written.outcomes.len(), 2 * 1000);
        let sent = sender.sent();
        assert_eq!(sent.len(), 2000);
        assert!(
            sent.chunks(2).all(|pair| pair == ["keyDown", "keyUp"]),
            "every character pressed and released in order"
        );
        assert_eq!(turn.devices(), InputDevices::default(), "nothing left held");

        let cut_short = CancellationToken::new();
        let sender = Sender::answering([]);
        let turn = Turn::of(InputDevices::default(), "A");
        let stopper = async {
            time::sleep(ROUND_TRIP * 301).await;
            cut_short.cancel();
        };
        let delivery = unread_ticket();
        let (written, ()) = tokio::join!(
            write_plan(&sender, Pen::Call(&delivery), &turn, &plan, &cut_short),
            stopper
        );
        assert!(!written.complete, "the rest of the text never went out");
        let sent = sender.sent();
        assert!(
            sent.len() < 2000 && sent.len().is_multiple_of(2),
            "{}",
            sent.len()
        );
        assert_eq!(
            turn.devices(),
            InputDevices::default(),
            "the last key released"
        );
    }

    /// Review P2: a key and a button two live runs hold stay down when one of them ends; that
    /// run only lets go of them, and the other's end releases them.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_run_end_keeps_what_another_live_run_holds() {
        let mut simulated = InputDevices::default();
        let mut setup = Plan::default();
        for holder in ["A", "B"] {
            keyboard_input::plan_key_down(
                &mut setup,
                &mut simulated,
                "Shift",
                Platform::Linux,
                &run(holder),
            )
            .expect("key");
            plan_down(
                &mut setup,
                &mut simulated,
                MouseButton::Left,
                1,
                &run(holder),
            );
        }
        assert_eq!(
            simulated.keyboard.pressed,
            [("ShiftLeft", run("A")), ("ShiftLeft", run("B"))]
        );
        let turn = Turn::of(simulated.clone(), "A");
        let mut release = Plan::default();
        keyboard_input::plan_key_releases(&mut release, &mut simulated, &run("A"));
        plan_button_releases(&mut release, &mut simulated, &run("A"));
        assert_eq!(release.writes(), 0, "A's end writes nothing B still holds");
        let sender = Sender::answering([]);
        let _written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &release,
            &CancellationToken::new(),
        )
        .await;
        assert!(sender.sent().is_empty());
        let held = turn.devices();
        assert_eq!(held.keyboard.pressed, [("ShiftLeft", run("B"))]);
        assert_eq!(
            held.keyboard.modifiers,
            keyboard_input::SHIFT,
            "Shift still down for B"
        );
        assert_eq!(held.mouse.buttons, [(MouseButton::Left, run("B"))]);

        let mut simulated = held;
        let mut release = Plan::default();
        keyboard_input::plan_key_releases(&mut release, &mut simulated, &run("B"));
        plan_button_releases(&mut release, &mut simulated, &run("B"));
        let _written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &release,
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(
            sender.sent(),
            ["keyUp", "mouseReleased"],
            "the last holder releases"
        );
        assert_eq!(turn.devices().keyboard, KeyboardState::default());
        assert!(turn.devices().mouse.buttons.is_empty());
    }

    /// Chromium's drag manager (Playwright's `DragManager`): a move with the left button held that starts
    /// a drag the page allows is followed by `dragEnter`, every later move is a `dragOver`, and
    /// the release drops it; a move that starts none is a move.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_drag_starts_moves_and_drops() {
        let data = serde_json::json!({ "items": [{ "mimeType": "text/plain", "data": "x" }], "dragOperationsMask": 1 });
        let mut simulated = InputDevices::default();
        let mut plan = Plan::default();
        plan_free_move(&mut plan, &mut simulated, (10.0, 10.0), 1);
        plan_down(&mut plan, &mut simulated, MouseButton::Left, 1, &run("A"));
        plan_free_move(&mut plan, &mut simulated, (50.0, 50.0), 3);
        plan_up(&mut plan, &mut simulated, MouseButton::Left, 1, None);
        let sender = Sender::dragging([Some(data.clone())]);
        let turn = Turn::of(InputDevices::default(), "A");
        let written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &plan,
            &CancellationToken::new(),
        )
        .await;
        assert!(written.complete);
        assert_eq!(
            sender.sent(),
            [
                "mouseMoved",
                "mousePressed",
                "mouseMoved",
                "intercepted",
                "dragEnter",
                "dragOver",
                "dragOver",
                "drop"
            ],
            "the press's first move starts the drag; the later ones drag it over; the release drops"
        );
        let after = turn.devices();
        assert_eq!(after.drag, None, "the drop ended it");
        assert!(after.mouse.buttons.is_empty());
        assert_eq!((after.mouse.x, after.mouse.y), (50.0, 50.0));

        let sender = Sender::dragging([None, None]);
        let turn = Turn::of(InputDevices::default(), "A");
        let written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &plan,
            &CancellationToken::new(),
        )
        .await;
        assert!(written.complete);
        assert_eq!(
            sender.sent(),
            [
                "mouseMoved",
                "mousePressed",
                "mouseMoved",
                "not dragged",
                "mouseMoved",
                "not dragged",
                "mouseMoved",
                "not dragged",
                "mouseReleased"
            ],
            "a page that starts no drag sees plain moves and the release"
        );
    }

    /// While a drag runs, an Escape keyDown cancels it in place of the key, a click's press is
    /// skipped and its release drops, and a run end cancels the drag it left running.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_running_drag_meets_escape_clicks_and_run_end() {
        let running = InputDevices {
            drag: Some(DragState {
                data: serde_json::json!({ "items": [], "dragOperationsMask": 1 }),
                x: 5.0,
                y: 5.0,
            }),
            mouse: MouseState {
                x: 5.0,
                y: 5.0,
                last_button: Some(MouseButton::Left),
                buttons: vec![(MouseButton::Left, run("A"))],
            },
            ..InputDevices::default()
        };
        let mut simulated = running.clone();
        let mut plan = Plan::default();
        keyboard_input::plan_press(
            &mut plan,
            &mut simulated,
            "Escape",
            Platform::Linux,
            &run("A"),
            None,
        )
        .expect("Escape");
        let sender = Sender::answering([]);
        let turn = Turn::of(running.clone(), "A");
        let _written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &plan,
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(sender.sent(), ["dragCancel", "keyUp"]);
        assert_eq!(turn.devices().drag, None);

        let mut simulated = running.clone();
        let mut plan = Plan::default();
        plan_click(
            &mut plan,
            &mut simulated,
            (9.0, 9.0),
            &MouseClick {
                button: MouseButton::Left,
                click_count: 1,
                delay: None,
            },
            1,
            &run("A"),
        );
        let sender = Sender::answering([]);
        let turn = Turn::of(running.clone(), "A");
        let written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &plan,
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(
            sender.sent(),
            ["mouseMoved", "drop"],
            "a click's move is never a drag's; its press is skipped and its release drops"
        );
        assert!(
            written.complete,
            "the click resolves: its skipped press counts as answered"
        );

        let mut simulated = running.clone();
        let mut release = Plan::default();
        plan_button_releases(&mut release, &mut simulated, &run("A"));
        let sender = Sender::answering([]);
        let turn = Turn::of(running, "A");
        let _written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &release,
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(
            sender.sent(),
            ["dragCancel"],
            "a drag the run left running is cancelled"
        );
        let after = turn.devices();
        assert_eq!(after.drag, None);
        assert!(after.mouse.buttons.is_empty());
    }

    /// A run end's release of a button other than the left neither drops nor cancels a drag
    /// another live run holds with the left button: it goes out as a plain release, and the
    /// drag runs on.
    #[tokio::test(start_paused = true)]
    async fn tests_that_another_buttons_release_leaves_a_live_runs_drag_running() {
        let running = InputDevices {
            drag: Some(DragState {
                data: serde_json::json!({ "items": [], "dragOperationsMask": 1 }),
                x: 5.0,
                y: 5.0,
            }),
            mouse: MouseState {
                x: 5.0,
                y: 5.0,
                last_button: Some(MouseButton::Right),
                buttons: vec![
                    (MouseButton::Left, run("B")),
                    (MouseButton::Right, run("A")),
                ],
            },
            ..InputDevices::default()
        };
        let mut simulated = running.clone();
        let mut end_a = Plan::default();
        plan_button_releases(&mut end_a, &mut simulated, &run("A"));
        let sender = Sender::answering([]);
        let turn = Turn::of(running, "observer");
        let written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &end_a,
            &CancellationToken::new(),
        )
        .await;
        assert!(written.complete);
        assert_eq!(
            sender.sent(),
            ["mouseReleased"],
            "neither dragCancel nor drop"
        );
        let after = turn.devices();
        assert!(after.drag.is_some(), "B's drag runs on");
        assert_eq!(after.mouse.buttons, [(MouseButton::Left, run("B"))]);
    }

    /// A run end whose drag cancel is refused, fails or is dropped mid-write (the run's
    /// deadline) still ends the run, and leaves the drag running with its button, as the page may
    /// still have them; the next run's end cancels the drag and lets go of the button.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_drag_cancel_cut_short_is_left_to_the_next_run_end() {
        let running = InputDevices {
            drag: Some(DragState {
                data: serde_json::json!({ "items": [], "dragOperationsMask": 1 }),
                x: 5.0,
                y: 5.0,
            }),
            mouse: MouseState {
                x: 5.0,
                y: 5.0,
                last_button: Some(MouseButton::Left),
                buttons: vec![(MouseButton::Left, run("A"))],
            },
            ..InputDevices::default()
        };
        for (exit, answer) in [
            ("refused", Answer::Refuse),
            ("failed", Answer::Fail),
            ("dropped", Answer::Hang),
        ] {
            let mut simulated = running.clone();
            let mut end_a = Plan::default();
            keyboard_input::plan_key_releases(&mut end_a, &mut simulated, &run("A"));
            plan_button_releases(&mut end_a, &mut simulated, &run("A"));
            let turn = Turn::of(running.clone(), "observer");
            let sender = Sender::answering([answer]);
            let never = CancellationToken::new();
            let delivery = unread_ticket();
            let write = write_plan(&sender, Pen::Call(&delivery), &turn, &end_a, &never);
            if exit == "dropped" {
                let dropped = time::timeout(Duration::from_secs(1), write).await;
                assert!(dropped.is_err(), "the run's deadline dropped it");
            } else {
                let _written = write.await;
            }
            assert_eq!(sender.sent(), ["dragCancel"], "{exit}");
            let after = turn.devices();
            assert_eq!(after.ended, [run("A")], "{exit}: A's run is over");
            assert!(after.drag.is_some(), "{exit}: the page may still drag");

            let mut simulated = after;
            let mut end_b = Plan::default();
            keyboard_input::plan_key_releases(&mut end_b, &mut simulated, &run("B"));
            plan_button_releases(&mut end_b, &mut simulated, &run("B"));
            let sender = Sender::answering([]);
            let _written = write_plan(
                &sender,
                Pen::Call(&unread_ticket()),
                &turn,
                &end_b,
                &CancellationToken::new(),
            )
            .await;
            assert_eq!(
                sender.sent(),
                ["dragCancel"],
                "{exit}: B's end cancels A's drag"
            );
            let after = turn.devices();
            assert_eq!(after.drag, None, "{exit}");
            assert!(after.mouse.buttons.is_empty(), "{exit}");
            assert!(after.ended.is_empty(), "{exit}");
        }
    }

    /// Known review issue: one failed run-end release must not prevent independent releases.
    #[tokio::test(start_paused = true)]
    async fn review_run_end_keeps_releasing_after_a_cdp_error() {
        let mut simulated = InputDevices::default();
        let mut setup = Plan::default();
        for key in ["Shift", "Control"] {
            keyboard_input::plan_key_down(
                &mut setup,
                &mut simulated,
                key,
                Platform::Linux,
                &run("A"),
            )
            .expect("key");
        }
        plan_down(&mut setup, &mut simulated, MouseButton::Left, 1, &run("A"));
        let turn = Turn::of(simulated.clone(), "A");
        let mut release = Plan::default();
        keyboard_input::plan_key_releases(&mut release, &mut simulated, &run("A"));
        plan_button_releases(&mut release, &mut simulated, &run("A"));
        let sender = Sender::answering([Answer::Fail, Answer::Ok, Answer::Ok]);
        let _written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &release,
            &CancellationToken::new(),
        )
        .await;
        let actual = turn.devices();
        assert_eq!(
            actual.keyboard.pressed,
            [("ControlLeft", run("A"))],
            "only the key whose release failed should remain held"
        );
        assert!(
            actual.mouse.buttons.is_empty(),
            "the mouse release is independent"
        );
    }

    /// A concurrent observer keeps the tab state alive while A's cleanup fails. A has ended,
    /// so after B presses and ends, A must not count as another live holder of Shift.
    #[tokio::test(start_paused = true)]
    async fn review_failed_cleanup_does_not_leave_a_live_holder() {
        let mut devices = InputDevices::default();
        let mut setup = Plan::default();
        keyboard_input::plan_key_down(
            &mut setup,
            &mut devices,
            "Shift",
            Platform::Linux,
            &run("A"),
        )
        .expect("key");
        let observer = Turn::of(devices.clone(), "observer");

        let mut end_a = Plan::default();
        keyboard_input::plan_key_releases(&mut end_a, &mut devices, &run("A"));
        let failed = Sender::answering([Answer::Fail]);
        let written = write_plan(
            &failed,
            Pen::Call(&unread_ticket()),
            &observer,
            &end_a,
            &CancellationToken::new(),
        )
        .await;
        assert!(written.outcomes[0].is_err());
        assert_eq!(observer.devices().keyboard.modifiers, keyboard_input::SHIFT);

        let mut devices = observer.devices();
        let mut down_b = Plan::default();
        keyboard_input::plan_key_down(
            &mut down_b,
            &mut devices,
            "Shift",
            Platform::Linux,
            &run("B"),
        )
        .expect("key");
        let sender = Sender::answering([]);
        let _written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &observer,
            &down_b,
            &CancellationToken::new(),
        )
        .await;

        let mut devices = observer.devices();
        let mut end_b = Plan::default();
        keyboard_input::plan_key_releases(&mut end_b, &mut devices, &run("B"));
        let release = Sender::answering([]);
        let _written = write_plan(
            &release,
            Pen::Call(&unread_ticket()),
            &observer,
            &end_b,
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(
            release.sent(),
            ["keyUp"],
            "A has ended; its failed release must not keep Shift held after B ends: {:?}",
            observer.devices()
        );
        assert_eq!(observer.devices().keyboard, KeyboardState::default());
    }

    const ROUND_TRIP: Duration = Duration::from_millis(1);

    fn run(name: &str) -> Owner {
        SessionId(name.to_string())
    }

    /// What a command sends, as `(type, key or button, modifiers, buttons, x, y)`.
    fn sent_events(plan: &Plan) -> Vec<Value> {
        plan.commands
            .iter()
            .filter_map(|command| command.input.as_ref())
            .map(|input| serde_json::from_str(input.input().params()).expect("params are JSON"))
            .collect()
    }

    #[derive(Clone, Copy)]
    enum Answer {
        Ok,
        Fail,
        Refuse,
        Hang,
        /// The session went (`SessionGone`): a closed tab or detached session.
        Gone,
    }

    /// Records each command's `type` (or `insertText`) and answers the next scripted outcome
    /// after a round trip.
    #[derive(Clone)]
    struct Sender {
        sent: Arc<Mutex<Vec<String>>>,
        answers: Arc<Mutex<VecDeque<Answer>>>,
        /// What each move with the left button held finds: the drag the page started, or none.
        drags: Arc<Mutex<VecDeque<Option<Value>>>>,
    }

    impl Sender {
        fn answering(answers: impl IntoIterator<Item = Answer>) -> Self {
            Self {
                sent: Arc::default(),
                answers: Arc::new(Mutex::new(answers.into_iter().collect())),
                drags: Arc::default(),
            }
        }

        /// A page whose next moves with the left button held start these drags, in order.
        fn dragging(drags: impl IntoIterator<Item = Option<Value>>) -> Self {
            Self {
                drags: Arc::new(Mutex::new(drags.into_iter().collect())),
                ..Self::answering([])
            }
        }

        fn sent(&self) -> Vec<String> {
            self.sent.lock().expect("sent").clone()
        }
    }

    impl InputSender for Sender {
        async fn send_input(&self, input: &PlannedInput, _pen: Pen<'_>) -> Result<(), CdpError> {
            let input = input.input();
            let params: Value = serde_json::from_str(input.params()).expect("params are JSON");
            let kind = match input.method() {
                "Input.insertText" => "insertText".to_string(),
                _ => params["type"].as_str().unwrap_or_default().to_string(),
            };
            self.sent.lock().expect("sent").push(kind);
            let answer = self
                .answers
                .lock()
                .expect("answers")
                .pop_front()
                .unwrap_or(Answer::Ok);
            time::sleep(ROUND_TRIP).await;
            match answer {
                Answer::Ok => Ok(()),
                Answer::Fail => Err(CdpError::Timeout {
                    method: input.method().to_string(),
                }),
                Answer::Refuse => Err(CdpError::ConnectionEpochChanged {
                    expected: 1,
                    actual: 2,
                }),
                Answer::Hang => future::pending().await,
                Answer::Gone => Err(CdpError::SessionGone),
            }
        }

        async fn move_intercepting_drag(
            &self,
            moved: &PlannedInput,
            delivery: &InputTicket,
        ) -> Result<Option<Value>, CdpError> {
            self.send_input(moved, Pen::Call(delivery)).await?;
            let drag = self.drags.lock().expect("drags").pop_front().flatten();
            self.sent.lock().expect("sent").push(
                if drag.is_some() {
                    "intercepted"
                } else {
                    "not dragged"
                }
                .to_string(),
            );
            Ok(drag)
        }
    }

    /// A test turn: its devices and the run that holds it.
    #[derive(Clone)]
    struct Turn {
        devices: Arc<Mutex<InputDevices>>,
        owner: Owner,
    }

    impl Turn {
        fn of(devices: InputDevices, owner: &str) -> Self {
            Self {
                devices: Arc::new(Mutex::new(devices)),
                owner: run(owner),
            }
        }

        fn devices(&self) -> InputDevices {
            self.devices.lock().expect("devices").clone()
        }
    }

    impl EffectSink for Turn {
        fn apply(&self, effect: &Effect, answered: bool) {
            // Each write is its own turn here.
            let mut devices = self.devices.lock().expect("devices");
            devices.apply(effect, answered);
            devices.forget_ended();
        }

        fn devices(&self) -> InputDevices {
            self.devices.lock().expect("devices").clone()
        }
    }

    impl Devices for Turn {
        fn snapshot(&self) -> Result<InputDevices, PageError> {
            Ok(self.devices())
        }

        fn owner(&self) -> Owner {
            self.owner.clone()
        }
    }

    /// A delivery record no test reads, for a writer the test drives itself.
    fn unread_ticket() -> InputTicket {
        fresh_ticket().1
    }

    fn fresh_ticket() -> (Arc<DeliveryCell>, InputTicket) {
        let cell = Arc::new(DeliveryCell::default());
        let ticket = cell.begin().expect("a fresh cell");
        (cell, ticket)
    }

    fn click(click_count: u32, delay: Option<Duration>) -> MouseClick {
        MouseClick {
            button: MouseButton::Left,
            click_count,
            delay,
        }
    }

    /// A click from a fresh page: Playwright's exact events, a move with no button, then a
    /// press and release per count with the button's mask and the count.
    #[test]
    fn tests_that_a_click_from_a_fresh_page_writes_playwrights_events() {
        let mut simulated = InputDevices::default();
        let mut plan = Plan::default();
        plan_click(
            &mut plan,
            &mut simulated,
            (1.0, 2.0),
            &click(2, None),
            1,
            &run("A"),
        );
        let summary: Vec<(String, String, i64, Option<i64>, f64)> = sent_events(&plan)
            .iter()
            .map(|event| {
                (
                    event["type"].as_str().unwrap_or_default().to_string(),
                    event["button"].as_str().unwrap_or_default().to_string(),
                    event["buttons"].as_i64().unwrap_or(-1),
                    event["clickCount"].as_i64(),
                    event["force"].as_f64().unwrap_or(-1.0),
                )
            })
            .collect();
        let row = |kind: &str, button: &str, buttons, count, force| {
            (kind.to_string(), button.to_string(), buttons, count, force)
        };
        assert_eq!(
            summary,
            [
                row("mouseMoved", "none", 0, None, 0.0),
                row("mousePressed", "left", 1, Some(1), 0.5),
                row("mouseReleased", "left", 0, Some(1), -1.0),
                row("mousePressed", "left", 1, Some(2), 0.5),
                row("mouseReleased", "left", 0, Some(2), -1.0),
            ]
        );
        assert!(
            plan.commands[..4].iter().all(|command| command.with_next),
            "without delay the whole click is one pipelined batch"
        );
        assert_eq!(
            (simulated.mouse.x, simulated.mouse.y),
            (1.0, 2.0),
            "the pointer ends at the click"
        );
        assert!(simulated.mouse.buttons.is_empty());
    }

    /// Design section 4.3: a modifier held by an earlier `keyboard.down` reaches every mouse
    /// and touch event of a later call, and a button held by `mouse.down` reaches a move.
    #[test]
    fn tests_that_a_held_modifier_and_button_reach_later_input() {
        let mut simulated = InputDevices::default();
        let mut plan = Plan::default();
        keyboard_input::plan_key_down(&mut plan, &mut simulated, "Shift", Platform::Mac, &run("A"))
            .expect("Shift");
        plan_down(&mut plan, &mut simulated, MouseButton::Right, 1, &run("A"));
        let mut later = Plan::default();
        plan_click(
            &mut later,
            &mut simulated,
            (5.0, 5.0),
            &click(1, None),
            1,
            &run("A"),
        );
        plan_tap(&mut later, &mut simulated, (6.0, 6.0));
        for event in sent_events(&later) {
            assert_eq!(event["modifiers"], i64::from(SHIFT), "{event}");
        }
        let moved = &sent_events(&later)[0];
        assert_eq!(moved["button"], "right", "the last button pressed");
        assert_eq!(moved["buttons"], 2, "the right button is still held");
        assert_eq!(moved["force"], 0.5, "a held button moves with force");
        let pressed = &sent_events(&later)[1];
        assert_eq!(pressed["buttons"], 3, "left pressed while right is held");
    }

    /// Design section 4.4: `steps` interpolate from the pointer's last position, as
    /// Playwright's `Mouse.move` does; no steps moves the pointer without an event.
    #[test]
    fn tests_that_steps_start_at_the_last_position() {
        let mut simulated = InputDevices::default();
        simulated.mouse.x = 100.0;
        simulated.mouse.y = 100.0;
        let mut plan = Plan::default();
        plan_move(&mut plan, &mut simulated, (200.0, 300.0), 5);
        let points: Vec<(f64, f64)> = sent_events(&plan)
            .iter()
            .map(|event| {
                (
                    event["x"].as_f64().unwrap_or(-1.0),
                    event["y"].as_f64().unwrap_or(-1.0),
                )
            })
            .collect();
        assert_eq!(
            points,
            [
                (120.0, 140.0),
                (140.0, 180.0),
                (160.0, 220.0),
                (180.0, 260.0),
                (200.0, 300.0)
            ],
            "Playwright's MO-012 tween"
        );
        let mut none = Plan::default();
        plan_move(&mut none, &mut simulated, (5.0, 6.0), 0);
        assert!(sent_events(&none).is_empty());
        assert_eq!((simulated.mouse.x, simulated.mouse.y), (5.0, 6.0));
    }

    /// An action's `modifiers` press the missing ones and release the extra ones around its
    /// input, then restore the keyboard; a modifier another run held comes back as that run's.
    #[test]
    fn tests_that_modifiers_are_ensured_around_the_input_and_restored() {
        let mut simulated = InputDevices::default();
        let mut setup = Plan::default();
        keyboard_input::plan_key_down(
            &mut setup,
            &mut simulated,
            "Control",
            Platform::Linux,
            &run("B"),
        )
        .expect("Control");
        let before = simulated.clone();
        let mut plan = Plan::default();
        let changes = keyboard_input::plan_ensure_modifiers(
            &mut plan,
            &mut simulated,
            SHIFT,
            Platform::Linux,
            &run("A"),
        );
        plan_click(
            &mut plan,
            &mut simulated,
            (1.0, 1.0),
            &click(1, None),
            1,
            &run("A"),
        );
        keyboard_input::plan_restore_modifiers(
            &mut plan,
            &mut simulated,
            &changes,
            Platform::Linux,
            &run("A"),
        );
        let kinds: Vec<(String, String)> = sent_events(&plan)
            .iter()
            .map(|event| {
                (
                    event["type"].as_str().unwrap_or_default().to_string(),
                    event["key"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect();
        let row = |kind: &str, key: &str| (kind.to_string(), key.to_string());
        assert_eq!(
            kinds,
            [
                row("keyUp", "Control"),
                row("rawKeyDown", "Shift"),
                row("mouseMoved", ""),
                row("mousePressed", ""),
                row("mouseReleased", ""),
                row("rawKeyDown", "Control"),
                row("keyUp", "Shift"),
            ],
            "Playwright's kModifiers order, for the change and for its restore"
        );
        for event in &sent_events(&plan)[2..5] {
            assert_eq!(
                event["modifiers"],
                i64::from(SHIFT),
                "the click sees Shift only"
            );
        }
        assert_eq!(
            simulated.keyboard, before.keyboard,
            "the keyboard is restored, Control still B's"
        );
        assert_eq!(
            plan.commands[5].undoes,
            Some(0),
            "the re-press undoes the release"
        );
        assert_eq!(
            plan.commands[6].undoes,
            Some(1),
            "the release undoes the press"
        );
    }

    /// Design section 4.5: a call stopped mid-input writes only what it owes, the release of
    /// every press and the restore of every modifier it changed, and reads `Unknown`.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_stopped_writer_still_writes_what_it_owes() {
        let mut simulated = InputDevices::default();
        let mut plan = Plan::default();
        let changes = keyboard_input::plan_ensure_modifiers(
            &mut plan,
            &mut simulated,
            SHIFT,
            Platform::Linux,
            &run("A"),
        );
        let delay = Duration::from_millis(50);
        plan_click(
            &mut plan,
            &mut simulated,
            (1.0, 1.0),
            &click(2, Some(delay)),
            1,
            &run("A"),
        );
        keyboard_input::plan_restore_modifiers(
            &mut plan,
            &mut simulated,
            &changes,
            Platform::Linux,
            &run("A"),
        );
        let sender = Sender::answering([]);
        let turn = Turn::of(InputDevices::default(), "A");
        let (cell, ticket) = fresh_ticket();
        let report = start_writer(
            |task| {
                tokio::spawn(task);
            },
            sender.clone(),
            plan,
            ticket,
            turn.clone(),
            (),
        );
        assert!(
            time::timeout(Duration::from_millis(20), report)
                .await
                .is_err()
        );
        time::sleep(Duration::from_secs(1)).await;
        assert_eq!(
            sender.sent(),
            [
                "rawKeyDown",
                "mouseMoved",
                "mousePressed",
                "mouseReleased",
                "keyUp"
            ],
            "the first press released, the second click never started, Shift restored"
        );
        assert_eq!(
            cell.delivery(),
            Delivery::Unknown,
            "partial input (design section 4.7)"
        );
        assert_eq!(
            turn.devices(),
            InputDevices {
                mouse: MouseState {
                    x: 1.0,
                    y: 1.0,
                    ..MouseState::default()
                },
                ..InputDevices::default()
            },
            "nothing left held"
        );
    }

    /// A pipelined click is one batch: every press and its release go out together, and the
    /// whole click reads `Done` once every event is answered.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_pipelined_click_writes_its_releases_with_its_presses() {
        let mut plan = Plan::default();
        plan_click(
            &mut plan,
            &mut InputDevices::default(),
            (1.0, 2.0),
            &click(2, None),
            1,
            &run("A"),
        );
        let sender = Sender::answering([]);
        let turn = Turn::of(InputDevices::default(), "A");
        let (cell, ticket) = fresh_ticket();
        let started = time::Instant::now();
        let written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &plan,
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(report(&written, ticket), PressReport::Acked(_)));
        assert_eq!(cell.delivery(), Delivery::Done);
        assert_eq!(
            sender.sent(),
            [
                "mouseMoved",
                "mousePressed",
                "mouseReleased",
                "mousePressed",
                "mouseReleased"
            ]
        );
        assert_eq!(
            started.elapsed(),
            ROUND_TRIP,
            "written at once, answered together"
        );
    }

    /// A press counts from its write, answered or not; a release only once answered, so a key
    /// whose release failed stays held for the run's end to release again.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_failed_release_leaves_the_key_held() {
        let mut simulated = InputDevices::default();
        let mut plan = Plan::default();
        keyboard_input::plan_press(
            &mut plan,
            &mut simulated,
            "Shift",
            Platform::Linux,
            &run("A"),
            None,
        )
        .expect("Shift");
        let sender = Sender::answering([Answer::Ok, Answer::Fail]);
        let turn = Turn::of(InputDevices::default(), "A");
        let written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &plan,
            &CancellationToken::new(),
        )
        .await;
        assert!(!written.outcomes.iter().all(Result::is_ok));
        let devices = turn.devices();
        assert_eq!(devices.keyboard.modifiers, SHIFT);
        assert_eq!(devices.keyboard.pressed, [("ShiftLeft", run("A"))]);
    }

    /// Design section 4.6: a run's end releases what that run left held, newest key first,
    /// then each of its buttons at the pointer's last position, and leaves another run's.
    #[test]
    fn tests_that_a_run_end_releases_only_its_own_input() {
        let mut simulated = InputDevices::default();
        let mut setup = Plan::default();
        for (key, owner) in [("Shift", "A"), ("a", "B"), ("Control", "A")] {
            keyboard_input::plan_key_down(
                &mut setup,
                &mut simulated,
                key,
                Platform::Linux,
                &run(owner),
            )
            .expect("a key");
        }
        plan_move(&mut setup, &mut simulated, (30.0, 40.0), 1);
        plan_down(
            &mut setup,
            &mut simulated,
            MouseButton::Middle,
            1,
            &run("A"),
        );
        plan_down(&mut setup, &mut simulated, MouseButton::Left, 1, &run("B"));
        let mut plan = Plan::default();
        keyboard_input::plan_key_releases(&mut plan, &mut simulated, &run("A"));
        plan_button_releases(&mut plan, &mut simulated, &run("A"));
        let released: Vec<String> = sent_events(&plan)
            .iter()
            .map(|event| {
                format!(
                    "{} {}",
                    event["type"].as_str().unwrap_or_default(),
                    event["key"]
                        .as_str()
                        .or(event["button"].as_str())
                        .unwrap_or_default()
                )
            })
            .collect();
        assert_eq!(
            released,
            ["keyUp Control", "keyUp Shift", "mouseReleased middle"],
            "A's keys newest first, then A's button"
        );
        let up = &sent_events(&plan)[2];
        assert_eq!(
            (up["x"].as_f64(), up["y"].as_f64()),
            (Some(30.0), Some(40.0))
        );
        assert_eq!(up["buttons"], 1, "B's left button is still held");
        assert_eq!(simulated.keyboard.pressed, [("KeyA", run("B"))]);
        assert_eq!(simulated.mouse.buttons, [(MouseButton::Left, run("B"))]);
    }

    /// A tap writes its end together with its start: a lost or failed answer still ends the
    /// touch and reads `Unknown`; a tap whose caller stopped finishes `Done`; a refused tap
    /// wrote nothing.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_tap_always_ends_its_touch() {
        let tap = || {
            let mut plan = Plan::default();
            plan_tap(&mut plan, &mut InputDevices::default(), (1.0, 2.0));
            plan
        };
        let turn = || Turn::of(InputDevices::default(), "A");
        let sender = Sender::answering([Answer::Fail, Answer::Fail]);
        let (cell, ticket) = fresh_ticket();
        let written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn(),
            &tap(),
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(
            report(&written, ticket),
            PressReport::Failed { closed: false, .. }
        ));
        assert_eq!(cell.delivery(), Delivery::Unknown);
        assert_eq!(sender.sent(), ["touchStart", "touchEnd"]);

        let sender = Sender::answering([Answer::Hang, Answer::Ok]);
        let (cell, ticket) = fresh_ticket();
        let report_future = start_writer(
            |task| {
                tokio::spawn(task);
            },
            sender.clone(),
            tap(),
            ticket,
            turn(),
            (),
        );
        // The writer has written the batch before its caller stops.
        time::sleep(Duration::from_micros(100)).await;
        drop(report_future);
        time::sleep(Duration::from_secs(5)).await;
        assert_eq!(
            sender.sent(),
            ["touchStart", "touchEnd"],
            "written together"
        );
        assert_eq!(cell.delivery(), Delivery::Unknown, "an unanswered start");

        let sender = Sender::answering([]);
        let (cell, ticket) = fresh_ticket();
        let report_future = start_writer(
            |task| {
                tokio::spawn(task);
            },
            sender.clone(),
            tap(),
            ticket,
            turn(),
            (),
        );
        time::sleep(Duration::from_micros(100)).await;
        drop(report_future);
        time::sleep(Duration::from_millis(10)).await;
        assert_eq!(
            cell.delivery(),
            Delivery::Done,
            "a stopped caller does not stop a tap"
        );

        let sender = Sender::answering([]);
        let (cell, ticket) = fresh_ticket();
        drop(start_writer(
            |task| {
                tokio::spawn(task);
            },
            sender.clone(),
            tap(),
            ticket,
            turn(),
            (),
        ));
        time::sleep(Duration::from_millis(10)).await;
        assert!(sender.sent().is_empty(), "stopped before its writer ran");
        assert_eq!(cell.delivery(), Delivery::NotSent);

        let sender = Sender::answering([Answer::Refuse, Answer::Refuse]);
        let (cell, ticket) = fresh_ticket();
        let written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn(),
            &tap(),
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(
            report(&written, ticket),
            PressReport::NothingWritten
        ));
        assert_eq!(cell.delivery(), Delivery::NotSent);
    }

    /// Every way a run end's release can stop short leaves the finished run ended, its leftover
    /// held by no live run: a call cut short before anything went out, a closed session, and a
    /// release whose future is dropped while its write hangs (the run's deadline). Refused and
    /// failed writes have cases of their own.
    #[tokio::test(start_paused = true)]
    async fn tests_that_every_early_exit_of_a_run_end_leaves_the_run_ended() {
        let held = || {
            let mut simulated = InputDevices::default();
            let mut setup = Plan::default();
            keyboard_input::plan_key_down(
                &mut setup,
                &mut simulated,
                "Shift",
                Platform::Linux,
                &run("A"),
            )
            .expect("key");
            plan_down(&mut setup, &mut simulated, MouseButton::Left, 1, &run("A"));
            simulated
        };
        let end_of = |simulated: &mut InputDevices| {
            let mut end = Plan::default();
            keyboard_input::plan_key_releases(&mut end, simulated, &run("A"));
            plan_button_releases(&mut end, simulated, &run("A"));
            end
        };

        // Cut short before anything went out: a run end's releases are written all the same,
        // and once they are answered nothing is left, the ended run included.
        let mut simulated = held();
        let turn = Turn::of(simulated.clone(), "observer");
        let end = end_of(&mut simulated);
        let cut_short = CancellationToken::new();
        cut_short.cancel();
        let sender = Sender::answering([]);
        let _written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &end,
            &cut_short,
        )
        .await;
        assert_eq!(sender.sent(), ["keyUp", "mouseReleased"], "cut short");
        assert_eq!(turn.devices().keyboard, KeyboardState::default());
        assert!(turn.devices().ended.is_empty() && turn.devices().mouse.buttons.is_empty());

        // The session went.
        let mut simulated = held();
        let turn = Turn::of(simulated.clone(), "observer");
        let end = end_of(&mut simulated);
        let sender = Sender::answering([Answer::Gone, Answer::Gone]);
        let _written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &end,
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(turn.devices().ended, [run("A")], "session gone");
        assert_eq!(
            sender.sent(),
            ["keyUp", "mouseReleased"],
            "each release still tried"
        );

        // The release's future is dropped while its first write hangs.
        let mut simulated = held();
        let turn = Turn::of(simulated.clone(), "observer");
        let end = end_of(&mut simulated);
        let sender = Sender::answering([Answer::Hang]);
        let dropped = time::timeout(
            Duration::from_secs(1),
            write_plan(
                &sender,
                Pen::Call(&unread_ticket()),
                &turn,
                &end,
                &CancellationToken::new(),
            ),
        )
        .await;
        assert!(dropped.is_err(), "the run's deadline dropped it");
        assert_eq!(turn.devices().ended, [run("A")], "dropped mid-write");

        // In each case a later run's end releases what A left, held by no live run.
        let mut simulated = turn.devices();
        let mut later = Plan::default();
        keyboard_input::plan_key_releases(&mut later, &mut simulated, &run("B"));
        plan_button_releases(&mut later, &mut simulated, &run("B"));
        let sender = Sender::answering([]);
        let _written = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &later,
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(sender.sent(), ["keyUp", "mouseReleased"]);
        assert_eq!(
            turn.devices(),
            InputDevices {
                mouse: MouseState {
                    last_button: None,
                    ..turn.devices().mouse
                },
                ..InputDevices::default()
            }
        );
    }

    /// A rejected write after an epoch change still ends this holder's ownership.
    #[tokio::test(start_paused = true)]
    async fn review_recheck2_epoch_refusal_still_retires_the_finished_owner() {
        let mut simulated = InputDevices::default();
        let mut setup = Plan::default();
        keyboard_input::plan_key_down(
            &mut setup,
            &mut simulated,
            "Shift",
            Platform::Linux,
            &run("A"),
        )
        .expect("key");
        let turn = Turn::of(simulated.clone(), "observer");
        let mut end_a = Plan::default();
        keyboard_input::plan_key_releases(&mut end_a, &mut simulated, &run("A"));
        plan_button_releases(&mut end_a, &mut simulated, &run("A"));
        let refused = Sender::answering([Answer::Refuse]);
        let written = write_plan(
            &refused,
            Pen::Call(&unread_ticket()),
            &turn,
            &end_a,
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(
            &written.outcomes[0],
            Err(CdpError::ConnectionEpochChanged { .. })
        ));
        let mut simulated = turn.devices();
        let mut down_b = Plan::default();
        keyboard_input::plan_key_down(
            &mut down_b,
            &mut simulated,
            "Shift",
            Platform::Linux,
            &run("B"),
        )
        .expect("key");
        let sender = Sender::answering([]);
        let _down = write_plan(
            &sender,
            Pen::Call(&unread_ticket()),
            &turn,
            &down_b,
            &CancellationToken::new(),
        )
        .await;
        let mut simulated = turn.devices();
        let mut end_b = Plan::default();
        keyboard_input::plan_key_releases(&mut end_b, &mut simulated, &run("B"));
        plan_button_releases(&mut end_b, &mut simulated, &run("B"));
        let released = Sender::answering([]);
        let _end = write_plan(
            &released,
            Pen::Call(&unread_ticket()),
            &turn,
            &end_b,
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(
            released.sent(),
            ["keyUp"],
            "a rejected cleanup must not leave A live after B ends: {:?}",
            turn.devices()
        );
        assert_eq!(turn.devices().keyboard, KeyboardState::default());
        assert!(turn.devices().ended.is_empty());
    }

    /// Failed key/button releases can be recovered without a new press, while live holders survive.
    #[tokio::test(start_paused = true)]
    async fn review_recheck2_retired_key_and_button_recovery_preserves_live_holders() {
        for live_b in [false, true] {
            let mut simulated = InputDevices::default();
            let mut setup = Plan::default();
            keyboard_input::plan_key_down(
                &mut setup,
                &mut simulated,
                "Shift",
                Platform::Linux,
                &run("A"),
            )
            .expect("key");
            plan_down(&mut setup, &mut simulated, MouseButton::Left, 1, &run("A"));
            let turn = Turn::of(simulated.clone(), "observer");
            let mut end_a = Plan::default();
            keyboard_input::plan_key_releases(&mut end_a, &mut simulated, &run("A"));
            plan_button_releases(&mut end_a, &mut simulated, &run("A"));
            let failed = Sender::answering([Answer::Fail, Answer::Fail]);
            let _end = write_plan(
                &failed,
                Pen::Call(&unread_ticket()),
                &turn,
                &end_a,
                &CancellationToken::new(),
            )
            .await;
            assert_eq!(turn.devices().ended, [run("A")]);
            if live_b {
                let mut simulated = turn.devices();
                let mut down_b = Plan::default();
                keyboard_input::plan_key_down(
                    &mut down_b,
                    &mut simulated,
                    "Shift",
                    Platform::Linux,
                    &run("B"),
                )
                .expect("key");
                plan_down(&mut down_b, &mut simulated, MouseButton::Left, 1, &run("B"));
                let _down = write_plan(
                    &Sender::answering([]),
                    Pen::Call(&unread_ticket()),
                    &turn,
                    &down_b,
                    &CancellationToken::new(),
                )
                .await;
                let mut simulated = turn.devices();
                let mut observer_end = Plan::default();
                keyboard_input::plan_key_releases(
                    &mut observer_end,
                    &mut simulated,
                    &run("observer"),
                );
                plan_button_releases(&mut observer_end, &mut simulated, &run("observer"));
                let sender = Sender::answering([]);
                let _end = write_plan(
                    &sender,
                    Pen::Call(&unread_ticket()),
                    &turn,
                    &observer_end,
                    &CancellationToken::new(),
                )
                .await;
                assert!(
                    sender.sent().is_empty(),
                    "a live holder still needs its key and button"
                );
            }
            let owner = if live_b { run("B") } else { run("observer") };
            let mut simulated = turn.devices();
            let mut final_end = Plan::default();
            keyboard_input::plan_key_releases(&mut final_end, &mut simulated, &owner);
            plan_button_releases(&mut final_end, &mut simulated, &owner);
            let sender = Sender::answering([]);
            let _end = write_plan(
                &sender,
                Pen::Call(&unread_ticket()),
                &turn,
                &final_end,
                &CancellationToken::new(),
            )
            .await;
            assert_eq!(sender.sent(), ["keyUp", "mouseReleased"]);
            let actual = turn.devices();
            assert_eq!(actual.keyboard, KeyboardState::default());
            assert!(actual.mouse.buttons.is_empty());
            assert!(actual.ended.is_empty());
        }
    }
}
