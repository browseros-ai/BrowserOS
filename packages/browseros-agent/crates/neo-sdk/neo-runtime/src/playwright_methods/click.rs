//! Playwright's pointer actions, `Frame.click`, `Frame.dblclick`, `Frame.hover` and `Frame.tap`,
//! as one policy in the structure of Playwright's server: `frames.ts`
//! `_retryWithProgressIfNotConnected` (loop A, re-resolve), `dom.ts` `_retryAction`,
//! `_retryPointerAction` and `_performPointerAction` (loop B, same element), the `SignalBarrier`
//! around the input, and `input.ts` `Mouse.click`, `Mouse.move` and `Touchscreen.tap`. A
//! [`PointerAction`] is Playwright's `PointerAction`: its name in the call log, the states it
//! waits for, what its hit-target interceptor judges, and the input it writes. The call log keeps
//! Playwright's wording, so a failed action tells the agent why in words it knows; nothing
//! depends on it matching Playwright's byte for byte. Leading spaces are nesting for
//! `progress::compress_call_log`.
//!
//! Resolution, backoff, and action pre-checks are owned by `retry.rs`; this module keeps
//! typed pointer input, delivery, and the navigation barrier. Shared preparation lives in `aim.rs`.
//!
//! [`Tab`] is the seam between this policy and CDP: one method per step of Playwright's own
//! attempt. `FrameTab` implements it over CDP and the glue; the tests implement it with
//! scripted answers and paused tokio time, so what a click calls, in which order, is exact without
//! a browser. Nothing here names a CDP or JSON type.
//!
//! Neo-only behaviour, each deliberate:
//! - `trial` ends at its checks, the hit check included, and sends no input. Playwright presses
//!   into an interceptor that blocks every event, which listeners the page added to `window`
//!   first still see, and under `force` it arms none and delivers a real click.
//! - `tap` needs no `hasTouch`: Neo's context is the user's attached BrowserOS context, which has
//!   no context options, and its tab answers `Input.dispatchTouchEvent` with the events of a
//!   `hasTouch` context. Playwright's `Frame.tap` refuses without `hasTouch`.
//! - The verdict is read after every press. Playwright skips it under `noWaitAfter`, and so
//!   reports a press its interceptor blocked as a click; here `noWaitAfter` skips only the
//!   navigation barrier.
//! - The interceptor also judges the press's release (the glue's `judgeRelease`): a target the
//!   page moved off the press point before the release (a popover placing itself again while it
//!   handles the press) gets its release and click blocked, and the verdict names what is under
//!   the point, so loop B presses again where the target went. Playwright judges the first event
//!   only and reports that click done although the target never got it.
//! - A CDP failure after a press was written ends the call with `Error` and delivery `Unknown`;
//!   Playwright's loop A would retry it and might press again.
//! - A `clickCount` above [`devices::MAX_CLICK_COUNT`] and `steps` above [`devices::MAX_STEPS`] are refused
//!   before any work, with no call log.
//! - Pointer actions on one tab take turns at its input ([`Tab::input`]), across every run of the
//!   `Runtime`. Playwright lets a second click's setup replace the first one's interceptor
//!   (`setupHitTargetInterceptor`'s own comment), so overlapping clicks judge each other's events.

use std::sync::Arc;
use std::time::Duration;

use neo_playwright_protocol::injected::{HitTargetAction, Point};
use neo_playwright_protocol::serve::{
    FrameClickParams, FrameClickParamsButton, FrameClickParamsModifiersItem,
    FrameClickParamsScroll, FrameDblclickParams, FrameDblclickParamsButton,
    FrameDblclickParamsModifiersItem, FrameDblclickParamsScroll, FrameHoverParams,
    FrameHoverParamsModifiersItem, FrameHoverParamsScroll, FrameTapParams,
    FrameTapParamsModifiersItem, FrameTapParamsScroll, Point as WirePoint,
};

use crate::chromium_bridge::keyboard_input::{self, KeyboardModifier, KeyboardTab, Platform};
use crate::chromium_bridge::mouse_and_touch_input::{
    self, Devices, EffectSink, MouseButton, MouseClick, Plan,
};
use crate::playwright_methods::aim::{self, Actionability, Aim, Readiness};
use crate::playwright_methods::devices;
use crate::playwright_methods::retry::{self, ActionResult, ElementAction, PreChecks, RetryAction};
use crate::playwright_methods::tab::{NotReady, PageError, PressReport, Stop, Tab, Verdict};
use crate::progress::{CallError, DeliveryCell, PlaywrightError, Progress};

// ---------------------------------------------------------------------------------------------
// The pointer action model.
// ---------------------------------------------------------------------------------------------

/// What a pointer action writes at its point, decided before anything is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PointerInput {
    /// Playwright: `server/dom.ts` `ElementHandle._hover`: `perform: (progress, point) => this._page.mouse.move(progress, point.x, point.y),`
    Move,
    /// Playwright: `server/dom.ts` `ElementHandle._click`: `perform: (progress, point) => this._page.mouse.click(progress, point.x, point.y, options),`
    Click { click: MouseClick, steps: u32 },
    /// Playwright: `server/dom.ts` `ElementHandle._tap`: `perform: (progress, point) => this._page.touchscreen.tap(progress, point.x, point.y),`
    Tap,
    /// Playwright: `server/frames.ts` `Frame.dragAndDrop`: `await this._page.mouse.down(progress);`
    MoveAndDown,
    /// Playwright: `server/frames.ts` `Frame.dragAndDrop`: `await this._page.mouse.up(progress);`
    MoveAndUp { steps: u32 },
}

/// Playwright's `PointerAction` (`dom.ts`): the action loop B performs once the element is
/// ready. Its name goes into the call log; [`PointerAction::actionability`] selects its waits,
/// [`PointerAction::hit_target`] what its interceptor judges, and [`PointerAction::input`] what
/// its writer sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PointerAction {
    Click {
        click: MouseClick,
        steps: u32,
        wait_after: WaitAfter,
    },
    /// A click of `clickCount: 2` under its own name; its [`MouseClick`] always counts two.
    Dblclick {
        click: MouseClick,
        steps: u32,
    },
    Hover,
    Tap,
    /// `dragAndDrop`'s first half, on the source: a move, then a left press.
    MoveAndDown,
    /// `dragAndDrop`'s second half, on the target: a move in `steps`, then the release, which
    /// drops when the press started a drag.
    MoveAndUp {
        steps: u32,
    },
}

impl PointerAction {
    /// `action.name`, as the call log says it (`attempting hover action`).
    /// Playwright: `server/dom.ts` `ElementHandle._retryPointerAction`: `return await this._retryAction(progress, action.name, async (progress, retry) => {`
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Click { .. } => "click",
            Self::Dblclick { .. } => "dblclick",
            Self::Hover => "hover",
            Self::Tap => "tap",
            Self::MoveAndDown => "move and down",
            Self::MoveAndUp { .. } => "move and up",
        }
    }

    /// Playwright: `server/frames.ts` `Frame.dragAndDrop`: `// Note: do not perform locator handlers checkpoint to avoid moving the mouse in the middle of a drag operation.`
    ///
    /// Whether loops A and B skip `performActionPreChecks` (the navigation checks and the
    /// locator handlers' checkpoint): only a drag's target half, which runs with the source's
    /// button held, so a handler's own input would break the drag. Its actionability checks,
    /// the deadline, the signal and the input turn stay.
    pub(crate) fn skips_action_pre_checks(self) -> bool {
        matches!(self, Self::MoveAndUp { .. })
    }

    /// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`
    fn actionability(self) -> Actionability {
        match self {
            Self::Hover | Self::MoveAndDown | Self::MoveAndUp { .. } => Actionability::Stable,
            Self::Click { .. } | Self::Dblclick { .. } | Self::Tap => Actionability::Enabled,
        }
    }

    /// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `const actionType = action.name === 'move and up' ? 'drag' : ((action.name === 'hover' || action.name === 'tap') ? action.name : 'mouse');`
    pub(crate) fn hit_target(self) -> HitTargetAction {
        match self {
            Self::Hover => HitTargetAction::Hover,
            Self::Tap => HitTargetAction::Tap,
            Self::MoveAndUp { .. } => HitTargetAction::Drag,
            Self::Click { .. } | Self::Dblclick { .. } | Self::MoveAndDown => {
                HitTargetAction::Mouse
            }
        }
    }

    pub(crate) fn input(self) -> PointerInput {
        match self {
            Self::Click { click, steps, .. } | Self::Dblclick { click, steps } => {
                PointerInput::Click { click, steps }
            }
            Self::Hover => PointerInput::Move,
            Self::Tap => PointerInput::Tap,
            Self::MoveAndDown => PointerInput::MoveAndDown,
            Self::MoveAndUp { steps } => PointerInput::MoveAndUp { steps },
        }
    }
}

/// The locator and preparation policy shared by pointer-family actions. Input-specific
/// fields live in each action's options and enum variant, so unrelated options cannot mix.
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent Playwright element-preparation options"
)]
pub(crate) struct PointerTarget {
    pub(crate) selector: String,
    /// Locators are strict; page/frame forms default to the first match.
    pub(crate) strict: bool,
    pub(crate) force: bool,
    pub(crate) scroll: bool,
    pub(crate) position: Option<Point>,
    pub(crate) trial: bool,
}

/// One click's decoded input and navigation policy.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClickOptions {
    pub(crate) target: PointerTarget,
    pub(crate) click: MouseClick,
    pub(crate) modifiers: Option<Vec<KeyboardModifier>>,
    pub(crate) steps: u32,
    pub(crate) wait_after: WaitAfter,
}

/// A double click always presses twice and never waits for navigation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DblclickOptions {
    pub(crate) target: PointerTarget,
    pub(crate) button: MouseButton,
    pub(crate) delay: Option<Duration>,
    pub(crate) modifiers: Option<Vec<KeyboardModifier>>,
    pub(crate) steps: u32,
}

/// Hover has modifiers but uses the mouse's default single movement step.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HoverOptions {
    pub(crate) target: PointerTarget,
    pub(crate) modifiers: Option<Vec<KeyboardModifier>>,
}

/// Touch input has modifiers and no mouse step or navigation-barrier options.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TapOptions {
    pub(crate) target: PointerTarget,
    pub(crate) modifiers: Option<Vec<KeyboardModifier>>,
}

/// A borrowed target plus only the fields its input action uses. Check and drag construct
/// these directly while keeping their own surrounding policy.
pub(crate) struct PointerCall<'a> {
    pub(crate) target: &'a PointerTarget,
    pub(crate) action: PointerAction,
}

impl PointerCall<'_> {
    fn pre_checks(&self) -> PreChecks {
        if self.action.skips_action_pre_checks() {
            PreChecks::Skip
        } else {
            PreChecks::unless_forced(self.target.force)
        }
    }
}

/// Resolve modifier meaning once before loop A, outside the input turn. In particular,
/// `Some([])` releases held modifiers for the input and still requires a platform read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeldModifiers {
    Keep,
    Exactly { mask: u8, platform: Platform },
}

/// Each command's own modifiers enum, as Playwright's `KeyboardModifier`.
macro_rules! keyboard_modifiers {
    ($modifiers:expr, $item:ident) => {
        $modifiers.map(|modifiers| {
            modifiers
                .into_iter()
                .map(|modifier| match modifier {
                    $item::Alt => KeyboardModifier::Alt,
                    $item::Control => KeyboardModifier::Control,
                    $item::ControlOrMeta => KeyboardModifier::ControlOrMeta,
                    $item::Meta => KeyboardModifier::Meta,
                    $item::Shift => KeyboardModifier::Shift,
                })
                .collect::<Vec<_>>()
        })
    };
}

impl ClickOptions {
    /// Reads `Frame.click`'s params. Refuses before any work, so the reply has no log, more than
    /// [`devices::MAX_CLICK_COUNT`] clicks and more than [`devices::MAX_STEPS`] steps.
    pub(crate) fn from_params(params: FrameClickParams) -> Result<Self, PlaywrightError> {
        let FrameClickParams {
            selector,
            strict,
            force,
            scroll,
            no_wait_after,
            modifiers,
            position,
            delay,
            button,
            click_count,
            trial,
            steps,
        } = params;
        let steps = devices::steps_of(steps)?;
        let click_count = devices::click_count_of(click_count)?;
        let button = match button.unwrap_or(FrameClickParamsButton::Left) {
            FrameClickParamsButton::Left => MouseButton::Left,
            FrameClickParamsButton::Right => MouseButton::Right,
            FrameClickParamsButton::Middle => MouseButton::Middle,
        };
        Ok(Self {
            target: PointerTarget {
                selector,
                strict: strict.unwrap_or(false),
                force: force.unwrap_or(false),
                scroll: scroll != Some(FrameClickParamsScroll::None),
                position: position.as_ref().map(point),
                trial: trial.unwrap_or(false),
            },
            wait_after: if no_wait_after.unwrap_or(false) {
                WaitAfter::Disabled
            } else {
                WaitAfter::Enabled
            },
            click: MouseClick {
                button,
                click_count,
                delay: devices::click_delay(delay),
            },
            modifiers: keyboard_modifiers!(modifiers, FrameClickParamsModifiersItem),
            steps,
        })
    }
}

impl DblclickOptions {
    /// Reads `Frame.dblclick`'s params: a click of two with its `button` and `delay`. The
    /// protocol has no `noWaitAfter` for it (the client drops the option), and Playwright never
    /// waits for its navigations. Refuses `steps` as [`ClickOptions::from_params`] does.
    pub(crate) fn from_params(params: FrameDblclickParams) -> Result<Self, PlaywrightError> {
        let FrameDblclickParams {
            selector,
            strict,
            force,
            scroll,
            modifiers,
            position,
            delay,
            button,
            trial,
            steps,
        } = params;
        let steps = devices::steps_of(steps)?;
        let button = match button.unwrap_or(FrameDblclickParamsButton::Left) {
            FrameDblclickParamsButton::Left => MouseButton::Left,
            FrameDblclickParamsButton::Right => MouseButton::Right,
            FrameDblclickParamsButton::Middle => MouseButton::Middle,
        };
        Ok(Self {
            target: PointerTarget {
                selector,
                strict: strict.unwrap_or(false),
                force: force.unwrap_or(false),
                scroll: scroll != Some(FrameDblclickParamsScroll::None),
                position: position.as_ref().map(point),
                trial: trial.unwrap_or(false),
            },
            button,
            delay: devices::click_delay(delay),
            modifiers: keyboard_modifiers!(modifiers, FrameDblclickParamsModifiersItem),
            steps,
        })
    }
}

impl HoverOptions {
    /// Reads `Frame.hover`'s params: one move, waiting for a visible and stable element only.
    pub(crate) fn from_params(params: FrameHoverParams) -> Self {
        let FrameHoverParams {
            selector,
            strict,
            force,
            scroll,
            modifiers,
            position,
            trial,
        } = params;
        Self {
            target: PointerTarget {
                selector,
                strict: strict.unwrap_or(false),
                force: force.unwrap_or(false),
                scroll: scroll != Some(FrameHoverParamsScroll::None),
                position: position.as_ref().map(point),
                trial: trial.unwrap_or(false),
            },
            modifiers: keyboard_modifiers!(modifiers, FrameHoverParamsModifiersItem),
        }
    }
}

impl TapOptions {
    /// Reads `Frame.tap`'s params: a touch start and end at the point.
    pub(crate) fn from_params(params: FrameTapParams) -> Self {
        let FrameTapParams {
            selector,
            strict,
            force,
            scroll,
            modifiers,
            position,
            trial,
        } = params;
        // Deviation: `server/frames.ts` `Frame.tap`: `if (!this._page.browserContext._options.hasTouch)` Neo's run attaches to the user's BrowserOS context, which has no hasTouch option, and its tab answers Input.dispatchTouchEvent with the events of a hasTouch context, so Neo taps where Playwright refuses.
        Self {
            target: PointerTarget {
                selector,
                strict: strict.unwrap_or(false),
                force: force.unwrap_or(false),
                scroll: scroll != Some(FrameTapParamsScroll::None),
                position: position.as_ref().map(point),
                trial: trial.unwrap_or(false),
            },
            modifiers: keyboard_modifiers!(modifiers, FrameTapParamsModifiersItem),
        }
    }
}

/// The wire's `Point` and the page call's are two generated types of one shape.
/// Neo: a copy between two generated types, which no script can see.
pub(crate) fn point(position: &WirePoint) -> Point {
    Point {
        x: position.x,
        y: position.y,
    }
}

/// Playwright: `server/frames.ts` `Frame.click`
pub(crate) async fn click<T: KeyboardTab>(
    tab: &T,
    options: &ClickOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    perform(
        tab,
        &PointerCall {
            target: &options.target,
            action: PointerAction::Click {
                click: options.click,
                steps: options.steps,
                wait_after: options.wait_after,
            },
        },
        options.modifiers.as_deref(),
        progress,
    )
    .await
}

/// Playwright: `server/frames.ts` `Frame.dblclick`
pub(crate) async fn dblclick<T: KeyboardTab>(
    tab: &T,
    options: &DblclickOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    // Playwright: `server/dom.ts` `ElementHandle._dblclick`: `{ ...options, clickCount: 2 }`
    perform(
        tab,
        &PointerCall {
            target: &options.target,
            action: PointerAction::Dblclick {
                click: MouseClick {
                    button: options.button,
                    click_count: 2,
                    delay: options.delay,
                },
                steps: options.steps,
            },
        },
        options.modifiers.as_deref(),
        progress,
    )
    .await
}

/// Playwright: `server/frames.ts` `Frame.hover`
pub(crate) async fn hover<T: KeyboardTab>(
    tab: &T,
    options: &HoverOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    perform(
        tab,
        &PointerCall {
            target: &options.target,
            action: PointerAction::Hover,
        },
        options.modifiers.as_deref(),
        progress,
    )
    .await
}

/// Playwright: `server/frames.ts` `Frame.tap`
pub(crate) async fn tap<T: KeyboardTab>(
    tab: &T,
    options: &TapOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    perform(
        tab,
        &PointerCall {
            target: &options.target,
            action: PointerAction::Tap,
        },
        options.modifiers.as_deref(),
        progress,
    )
    .await
}

/// Resolves modifier meaning before loop A; planning under the input turn then needs no
/// platform read. Delivery belongs to the whole call, including both halves of a drag.
pub(crate) async fn perform<T: KeyboardTab>(
    tab: &T,
    call: &PointerCall<'_>,
    modifiers: Option<&[KeyboardModifier]>,
    progress: &Progress,
) -> Result<(), CallError> {
    let Some(delivery) = progress.delivery() else {
        return Err(PlaywrightError::error(format!(
            "neo-runtime: Frame.{} has no delivery cell",
            call.action.name()
        ))
        .into());
    };
    let modifiers = if let Some(modifiers) = modifiers {
        let platform = progress
            .race(tab.platform())
            .await?
            .map_err(platform_error)?;
        HeldModifiers::Exactly {
            mask: keyboard_input::modifiers_mask(modifiers, platform),
            platform,
        }
    } else {
        HeldModifiers::Keep
    };
    retry::retry_with_progress_if_not_connected(
        tab,
        &call.target.selector,
        call.target.strict,
        call.pre_checks(),
        progress,
        &ResolvedPointer {
            call,
            modifiers,
            delivery,
        },
    )
    .await
}

/// Adapter for loop A: each newly resolved element runs the same typed call and modifiers.
struct ResolvedPointer<'a, 'target> {
    call: &'a PointerCall<'target>,
    modifiers: HeldModifiers,
    delivery: &'a Arc<DeliveryCell>,
}

impl<T: Tab> ElementAction<T> for ResolvedPointer<'_, '_> {
    type Output = ();
    async fn act(
        &self,
        tab: &T,
        element: &T::Element,
        progress: &Progress,
    ) -> Result<ActionResult<()>, Stop> {
        retry_pointer_action(
            tab,
            element,
            self.call,
            self.modifiers,
            self.delivery,
            progress,
        )
        .await
    }
}

/// What a pointer action awaits after its input: Playwright's `true` and `'disabled'`. Its
/// `false` (`noWaitAfter`) skips the verdict too, and so reports a press its interceptor blocked
/// as a click; Neo reads the verdict after every press, so `noWaitAfter` is `Disabled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WaitAfter {
    /// The hit-target verdict, then the navigation barrier.
    Enabled,
    /// The verdict only: `noWaitAfter`, and check/uncheck.
    Disabled,
}

/// What one attempt came to.
enum Attempt {
    Done,
    NotReady(NotReady),
}

/// Loop B (`_retryAction` with `_retryPointerAction`): the same element until its action is
/// done, it is detached, or the call stops. Under `force`, `NotVisible` and `NotInViewport` end
/// the call (Playwright's `NonRecoverableDOMError`).
pub(crate) async fn retry_pointer_action<T: Tab>(
    tab: &T,
    element: &T::Element,
    call: &PointerCall<'_>,
    modifiers: HeldModifiers,
    delivery: &Arc<DeliveryCell>,
    progress: &Progress,
) -> Result<ActionResult<()>, Stop> {
    let mut retries = RetryAction::new(call.action.name()).with_trial(call.target.trial);
    let pre_checks = call.pre_checks();
    loop {
        let retry = retries.before(tab, pre_checks, progress).await?;
        let reason = match attempt(tab, element, retry, call, modifiers, progress, delivery).await?
        {
            Attempt::Done => return Ok(ActionResult::Done(())),
            Attempt::NotReady(NotReady::NotConnected) => return Ok(ActionResult::NotConnected),
            Attempt::NotReady(reason) => reason,
        };
        if call.target.force {
            match reason {
                NotReady::NotVisible => {
                    return Err(PlaywrightError::error("Element is not visible").into());
                }
                NotReady::NotInViewport => {
                    return Err(PlaywrightError::error("Element is outside of the viewport").into());
                }
                _ => {}
            }
        }
        if let Some(line) = retry::not_ready_line(&reason) {
            progress.log(line);
        }
    }
}

/// Prepare the element, then either finish a trial or send this action's input. The
/// returned turn and interceptor stay owned together through the input and its verdict.
async fn attempt<T: Tab>(
    tab: &T,
    element: &T::Element,
    retry: usize,
    call: &PointerCall<'_>,
    modifiers: HeldModifiers,
    progress: &Progress,
    delivery: &Arc<DeliveryCell>,
) -> Result<Attempt, Stop> {
    let action = call.action;
    let (point, input, armed) = match aim::ready_to_perform(
        tab,
        element,
        &Aim {
            actionability: action.actionability(),
            hit_target: action.hit_target(),
            position: call.target.position.as_ref(),
            scroll: call.target.scroll,
            force: call.target.force,
        },
        retry,
        progress,
    )
    .await?
    {
        Readiness::NotReady(reason) => return Ok(Attempt::NotReady(reason)),
        Readiness::Ready {
            point,
            input,
            armed,
        } => (point, input, armed),
    };
    if call.target.trial {
        // Deviation: `server/dom.ts` `ElementHandle._performPointerAction`: `await action.perform(progress, point);` Playwright's trial still performs the input into an interceptor that blocks every event it judges, which the page's earlier listeners and the hover state still see; Neo's trial ends at its checks and sends no input.
        // Dropping the interceptor disarms it, and the tab's input comes back once that stop has answered.
        drop(armed);
        progress.log(format!("  trial {} action done", action.name()));
        return Ok(Attempt::Done);
    }
    act(
        tab, point, armed, input, call, modifiers, progress, delivery,
    )
    .await
}

/// Input and everything after it, inside Playwright's `waitForSignalsCreatedBy`.
///
/// Delivery follows the cell's typestate (`progress.rs`): the ticket is taken right before the
/// press and moves into the writer; an acknowledged press is `Done` unless the interceptor
/// verifiably blocked it (its verdict names another element), which puts it back to `NotSent`
/// so loop B may press again, exactly when Playwright would. Every other way out of
/// here after the press ends the call: no code path retries a press the page may have seen.
/// The armed interceptor rides in the writer with the ticket, so it is still armed when the
/// last release lands however this returns, the call stopping mid-press included. `input`, the
/// click's turn at the tab, goes with the verdict; the writer and the interceptor hold their own
/// copies until they are done.
#[expect(
    clippy::too_many_arguments,
    reason = "the pointer step carries its input ownership and epilogue separately"
)]
async fn act<T: Tab>(
    tab: &T,
    point: Point,
    armed: Option<T::Armed>,
    input: T::Input,
    call: &PointerCall<'_>,
    modifiers: HeldModifiers,
    progress: &Progress,
    delivery: &Arc<DeliveryCell>,
) -> Result<Attempt, Stop> {
    let name = call.action.name();
    // Planned under the turn, from the tab's devices as they are now: the held modifiers and
    // buttons, and the pointer where the last input left it.
    let plan = plan_pointer_input(&input, &point, call, modifiers)?;
    let watched = matches!(
        call.action,
        PointerAction::Click {
            wait_after: WaitAfter::Enabled,
            ..
        }
    )
    .then(|| tab.watch_navigations());
    // The last chance to stop before an irreversible step.
    progress.check()?;
    let (acked, armed) = if plan.writes() == 0 {
        // `clickCount: 0` with `steps: 0`: Playwright's action writes no event and succeeds,
        // and its pointer still moves. Nothing was sent: the cell stays `NotSent`.
        progress.log(format!("  performing {name} action"));
        for command in &plan.commands {
            input.apply(&command.effect, true);
        }
        (None, armed)
    } else {
        // A drag's second half writes after its first: its ticket keeps the cell `Unknown`.
        let ticket = if matches!(call.action, PointerAction::MoveAndUp { .. }) {
            delivery.begin_more()
        } else {
            delivery.begin()
        };
        let Ok(ticket) = ticket else {
            return Err(PlaywrightError::error(format!(
                "neo-runtime: a {name} tried to write its input again"
            ))
            .into());
        };
        progress.log(format!("  performing {name} action"));
        let (report, armed) = progress
            .race(tab.press(plan, ticket, armed, input.clone()))
            .await?;
        let acked = match report {
            PressReport::Acked(acked) => acked,
            // Nothing was written because the session's socket was replaced, or the session
            // went mid-press: either way the tab is gone for this run.
            PressReport::NothingWritten | PressReport::Failed { closed: true, .. } => {
                return Err(PlaywrightError::target_closed().into());
            }
            PressReport::Failed {
                closed: false,
                message,
            } => return Err(PlaywrightError::error(message).into()),
        };
        (Some(acked), armed)
    };

    // Read after every press, `noWaitAfter` too: it cannot stall, because it races a dialog
    // opening and a navigation being requested.
    let blocked = match armed {
        Some(armed) => match progress.race(tab.verdict(armed)).await? {
            Verdict::Done => None,
            Verdict::Blocked(target) => Some(target),
        },
        None => None,
    };
    // Not held through the navigation barrier, which only waits: an interceptor not yet stopped
    // keeps its own copy until it is.
    drop(input);
    match (acked, blocked.is_some()) {
        (Some(acked), true) => acked.blocked(),
        (Some(acked), false) => acked.delivered(),
        (None, _) => {}
    }

    if blocked.is_none() {
        progress.log(format!("  {name} action done"));
        progress.log("  waiting for scheduled navigations to finish");
    }
    // Playwright's barrier runs after its action returns, blocked or not.
    if let Some(watched) = watched {
        match progress.race(tab.settle_after_input(watched)).await? {
            Ok(committed) => {
                for url in committed {
                    progress.log(format!("  navigated to \"{url}\""));
                }
            }
            Err(PageError::Closed) => return Err(PlaywrightError::target_closed().into()),
            // Playwright's epilogue swallows its errors; the input already happened.
            Err(_) => {}
        }
    }
    if let Some(target) = blocked {
        return Ok(Attempt::NotReady(NotReady::Intercepted(target)));
    }
    progress.log("  navigations have finished");
    Ok(Attempt::Done)
}

/// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `if (options && options.modifiers)`
///
/// The action's input as one plan, from the tab's devices under the turn: Playwright's
/// `ensureModifiers` for `modifiers`, the action's own input at `point` (a hover's move and a
/// click's in `steps` from the pointer's last position, with the held modifiers and buttons),
/// then the modifiers' restore, each restore owed so a stopped call still restores.
fn plan_pointer_input<D: Devices>(
    input: &D,
    point: &Point,
    call: &PointerCall<'_>,
    modifiers: HeldModifiers,
) -> Result<Plan, Stop> {
    let mut simulated = input.snapshot()?;
    let owner = input.owner();
    let mut plan = Plan::default();
    let target = match modifiers {
        HeldModifiers::Keep => None,
        HeldModifiers::Exactly { mask, platform } => Some((mask, platform)),
    };
    let changes = target.map(|(mask, platform)| {
        keyboard_input::plan_ensure_modifiers(&mut plan, &mut simulated, mask, platform, &owner)
    });
    let to = (point.x, point.y);
    match call.action.input() {
        PointerInput::Move => {
            // Playwright: `server/input.ts` `Mouse.move`: `const { steps = 1 } = options;`
            mouse_and_touch_input::plan_free_move(&mut plan, &mut simulated, to, 1);
        }
        PointerInput::MoveAndDown => {
            // Playwright: `server/frames.ts` `Frame.dragAndDrop`: `await this._page.mouse.move(progress, point.x, point.y);`
            mouse_and_touch_input::plan_free_move(&mut plan, &mut simulated, to, 1);
            mouse_and_touch_input::plan_down(
                &mut plan,
                &mut simulated,
                MouseButton::Left,
                1,
                &owner,
            );
        }
        PointerInput::MoveAndUp { steps } => {
            // Playwright: `server/frames.ts` `Frame.dragAndDrop`: `await this._page.mouse.move(progress, point.x, point.y, { steps: options.steps });`
            mouse_and_touch_input::plan_free_move(&mut plan, &mut simulated, to, steps);
            mouse_and_touch_input::plan_up(&mut plan, &mut simulated, MouseButton::Left, 1, None);
        }
        PointerInput::Click { click, steps } => {
            mouse_and_touch_input::plan_click(&mut plan, &mut simulated, to, &click, steps, &owner);
        }
        PointerInput::Tap => mouse_and_touch_input::plan_tap(&mut plan, &mut simulated, to),
    }
    if let (Some((_, platform)), Some(changes)) = (target, changes) {
        keyboard_input::plan_restore_modifiers(
            &mut plan,
            &mut simulated,
            &changes,
            platform,
            &owner,
        );
    }
    Ok(plan)
}

/// The platform read failing ends the call as any page step's terminal error does.
fn platform_error(error: PageError) -> CallError {
    match error {
        PageError::Closed => PlaywrightError::target_closed().into(),
        PageError::NotConnected => {
            PlaywrightError::error("neo-runtime: the browser's platform could not be read").into()
        }
        PageError::Threw(message) | PageError::Contract(message) => {
            PlaywrightError::error(message).into()
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    //! Policy tests against [`FakeTab`], under paused tokio time, so every wait is exact and no
    //! test depends on the machine's speed. They check what a click calls, in which order, and
    //! what it delivers; never the call log's wording or how many attempts fit in a timeout.
    //! `checkElementStates` takes 33 ms when `stable` is checked (two animation frames at 60 Hz,
    //! `injectedScript.ts` `_checkElementIsStable` with `stableRafCount: 1`); every other step
    //! takes 1 ms, a loopback CDP round trip.

    use std::collections::BTreeSet;

    use tokio::time;
    use tokio::time::Instant;

    use serde_json::Value;

    use super::*;
    use crate::playwright_methods::aim::{CLICK_STATES, HOVER_STATES};
    use crate::playwright_methods::tab::fake::{
        ArmAnswer, FakeTab, OK_QUAD, OVERLAY, ROUND_TRIP, SELECTOR, STABLE, Step, Writes,
        fake_owner,
    };
    use crate::playwright_methods::tab::{Geometry, SelectorFacts, StatesCheck};
    use crate::progress::ErrorName;
    use crate::run::Delivery;
    use neo_playwright_protocol::injected::ElementState;

    fn options(selector: &str) -> ClickOptions {
        ClickOptions::from_params(
            serde_json::from_value(serde_json::json!({
                "selector": selector,
                "strict": true,
            }))
            .expect("click params"),
        )
        .expect("served options")
    }

    /// Runs one pointer action against `tab` with Playwright's `timeout`, and returns its
    /// outcome, its folded log and its delivery.
    async fn run(
        tab: &FakeTab,
        options: &impl TestPointer,
        timeout_ms: f64,
    ) -> (Result<(), CallError>, Vec<String>, Delivery) {
        let progress = Progress::for_test(timeout_ms);
        let result = options.invoke(tab, &progress).await;
        let delivery = progress.delivery().expect("an input call").delivery();
        (result, progress.folded_log(), delivery)
    }

    fn timeout_error(timeout_ms: f64) -> Result<(), CallError> {
        Err(PlaywrightError::timeout(timeout_ms).into())
    }

    /// A selector that never matches polls until the timeout, takes no input turn and delivers
    /// nothing.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_selector_that_never_matches_times_out_without_input() {
        let mut tab = FakeTab::ready();
        tab.resolve = Step::always(ROUND_TRIP, Ok(None));
        let (result, _, delivery) = run(&tab, &options(SELECTOR), 500.0).await;

        assert_eq!(result, timeout_error(500.0));
        assert!(tab.count("resolve") > 1, "it polled: {:?}", tab.calls());
        assert_eq!(tab.count("input"), 0, "{:?}", tab.calls());
        assert_eq!(delivery, Delivery::NotSent);
    }

    /// A strict mode violation ends the call at once with the page's message, before any wait
    /// or input.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_strict_mode_violation_fails_at_once() {
        let message = "Error: strict mode violation: getByRole('button', { name: 'Submit' }) \
                       resolved to 2 elements";
        let mut tab = FakeTab::ready();
        tab.resolve = Step::always(ROUND_TRIP, Err(PageError::Threw(message.to_string())));
        let started = Instant::now();
        let (result, _, delivery) = run(&tab, &options(SELECTOR), 30_000.0).await;

        assert_eq!(result, Err(PlaywrightError::error(message).into()));
        assert_eq!(tab.count("resolve"), 1);
        assert!(
            started.elapsed() < Duration::from_millis(20),
            "no backoff wait before failing: {:?}",
            started.elapsed()
        );
        assert_eq!(delivery, Delivery::NotSent);
    }

    /// A point another element covers is retried until the timeout, each retry with another
    /// scroll alignment, and never pressed; every refused setup gives the tab's input back.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_covered_element_is_never_pressed() {
        let mut tab = FakeTab::ready();
        tab.arm = Step::always(ROUND_TRIP, Ok(ArmAnswer::Intercepted(OVERLAY.to_string())));
        let (result, _, delivery) = run(&tab, &options(SELECTOR), 1000.0).await;

        assert_eq!(result, timeout_error(1000.0));
        assert!(
            !tab.calls().iter().any(|call| call.starts_with("press")),
            "{:?}",
            tab.calls()
        );
        assert_eq!(delivery, Delivery::NotSent);
        let scrolls: BTreeSet<String> = tab
            .calls()
            .into_iter()
            .filter(|call| call.starts_with("scroll"))
            .collect();
        assert!(
            scrolls.len() > 1,
            "the retries switch scroll alignment: {scrolls:?}"
        );
        assert_eq!(
            tab.count("input"),
            tab.count("input released"),
            "an intercepted setup gives its turn back at once: {:?}",
            tab.calls()
        );
    }

    /// A disabled element is checked again until the timeout and never reaches input.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_disabled_element_is_never_pressed() {
        let mut tab = FakeTab::ready();
        tab.check_states = Step::always(STABLE, Ok(StatesCheck::Missing(ElementState::Enabled)));
        let (result, _, delivery) = run(&tab, &options(SELECTOR), 500.0).await;

        assert_eq!(result, timeout_error(500.0));
        assert!(tab.count("check_states") > 1, "{:?}", tab.calls());
        assert_eq!(
            tab.count("input"),
            0,
            "no input turn without a ready element"
        );
        assert_eq!(delivery, Delivery::NotSent);
    }

    #[tokio::test(start_paused = true)]
    async fn an_ok_click_presses_once_at_the_middle_of_the_element_and_delivers() {
        let tab = FakeTab::ready();
        let (result, _, delivery) = run(&tab, &options(SELECTOR), 30_000.0).await;

        assert_eq!(result, Ok(()));
        assert_eq!(delivery, Delivery::Done);
        assert_eq!(
            tab.calls(),
            [
                "describe",
                "resolve",
                "check_states",
                "scroll Protocol",
                "geometry",
                "input",
                "arm Mouse Point { x: 36.75, y: 18.5 }",
                "watch_navigations",
                "press (36.75, 18.5)",
                "released",
                "verdict",
                "disarmed",
                "input released",
                "settle",
            ],
            "the turn at the tab's input starts after every wait for the element and ends with \
             the verdict, before the navigation barrier"
        );
    }

    /// Under `force` nothing is armed, so the writer's copy is what holds the turn while it
    /// presses; the turn ends before the navigation barrier.
    #[tokio::test(start_paused = true)]
    async fn a_forced_click_holds_the_tab_input_while_it_presses() {
        let tab = FakeTab::ready();
        let mut forced = options(SELECTOR);
        forced.target.force = true;
        let (result, _, delivery) = run(&tab, &forced, 30_000.0).await;

        assert_eq!(result, Ok(()));
        assert_eq!(delivery, Delivery::Done);
        assert_eq!(
            tab.calls(),
            [
                "describe",
                "resolve",
                "scroll Protocol",
                "geometry",
                "input",
                "watch_navigations",
                "press (36.75, 18.5)",
                "released",
                "input released",
                "settle",
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_verified_block_after_input_presses_again() {
        let mut tab = FakeTab::ready();
        tab.verdict = Step::new(
            ROUND_TRIP,
            [Verdict::Blocked(OVERLAY.to_string()), Verdict::Done],
        );
        let (result, _, delivery) = run(&tab, &options(SELECTOR), 30_000.0).await;

        assert_eq!(result, Ok(()));
        assert_eq!(delivery, Delivery::Done);
        assert_eq!(tab.count("press (36.75, 18.5)"), 2);
        assert_eq!(
            tab.count("settle"),
            2,
            "Playwright's barrier runs after a blocked press too"
        );
        let turns: Vec<String> = tab
            .calls()
            .into_iter()
            .filter(|call| call.starts_with("input"))
            .collect();
        assert_eq!(
            turns,
            ["input", "input released", "input", "input released"],
            "each press has a turn of its own, given back before the retry"
        );
    }

    /// `noWaitAfter` skips only the navigation barrier. Playwright skips the verdict too, and so
    /// reports a press its interceptor blocked as a click.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_blocked_press_under_no_wait_after_presses_again() {
        let mut tab = FakeTab::ready();
        tab.verdict = Step::new(
            ROUND_TRIP,
            [Verdict::Blocked(OVERLAY.to_string()), Verdict::Done],
        );
        let mut no_wait = options(SELECTOR);
        no_wait.wait_after = WaitAfter::Disabled;
        let (result, _, delivery) = run(&tab, &no_wait, 30_000.0).await;

        assert_eq!(result, Ok(()));
        assert_eq!(delivery, Delivery::Done);
        assert_eq!(
            tab.count("press (36.75, 18.5)"),
            2,
            "the blocked press is pressed again: {:?}",
            tab.calls()
        );
        assert_eq!(tab.count("settle"), 0, "no navigation barrier");
    }

    /// A trial runs every check a click runs, the hit check included, and sends no input.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_trial_checks_the_hit_target_and_sends_no_input() {
        let tab = FakeTab::ready();
        let mut trial = options(SELECTOR);
        trial.target.trial = true;
        let (result, _, delivery) = run(&tab, &trial, 30_000.0).await;

        assert_eq!(result, Ok(()));
        assert_eq!(delivery, Delivery::NotSent);
        let calls = tab.calls();
        assert!(
            calls.iter().any(|call| call.starts_with("arm ")),
            "the hit check ran: {calls:?}"
        );
        assert!(
            !calls.iter().any(|call| call.starts_with("press")),
            "a trial presses nothing: {calls:?}"
        );
        assert_eq!(
            calls.last().map(String::as_str),
            Some("input released"),
            "{calls:?}"
        );
    }

    /// Under `force` a trial has nothing left to check: no interceptor and no input. Playwright's
    /// forced trial arms nothing and delivers a real click.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_forced_trial_sends_no_input() {
        let tab = FakeTab::ready();
        let mut forced = options(SELECTOR);
        forced.target.trial = true;
        forced.target.force = true;
        let (result, _, delivery) = run(&tab, &forced, 30_000.0).await;

        assert_eq!(result, Ok(()));
        assert_eq!(delivery, Delivery::NotSent);
        let calls = tab.calls();
        assert!(
            !calls
                .iter()
                .any(|call| call.starts_with("arm ") || call.starts_with("press")),
            "{calls:?}"
        );
        assert_eq!(tab.count("check_states"), 0, "force skips the state checks");
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_press_is_unknown_and_never_presses_again() {
        let mut tab = FakeTab::ready();
        tab.press = Step::always(ROUND_TRIP, Writes::Fail);
        let (result, _, delivery) = run(&tab, &options(SELECTOR), 30_000.0).await;

        assert_eq!(
            result,
            Err(PlaywrightError::error("Input.dispatchMouseEvent timed out").into())
        );
        assert_eq!(delivery, Delivery::Unknown);
        assert_eq!(tab.count("press (36.75, 18.5)"), 1);
        assert!(tab.calls().contains(&"disarmed".to_string()));
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_written_reads_as_the_tab_closing_and_not_sent() {
        let mut tab = FakeTab::ready();
        tab.press = Step::always(ROUND_TRIP, Writes::Nothing);
        let (result, _, delivery) = run(&tab, &options(SELECTOR), 30_000.0).await;

        assert_eq!(result, Err(PlaywrightError::target_closed().into()));
        assert_eq!(delivery, Delivery::NotSent);
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_that_stops_while_its_press_is_pending_reads_what_the_writer_did() {
        let mut tab = FakeTab::ready();
        tab.press = Step::always(Duration::from_secs(2), Writes::Ack);
        let progress = Progress::for_test(500.0);
        let result = click(&tab, &options(SELECTOR), &progress).await;
        let cell = progress.delivery().expect("an input call");

        assert_eq!(result, timeout_error(500.0));
        assert_eq!(
            cell.delivery(),
            Delivery::Unknown,
            "the writer is mid-press"
        );
        assert!(
            !tab.calls().contains(&"disarmed".to_string()),
            "the writer still holds the interceptor: {:?}",
            tab.calls()
        );
        assert!(
            !tab.calls().contains(&"input released".to_string()),
            "the stopped call let go of its copy only; the writer still holds the tab's input: \
             {:?}",
            tab.calls()
        );
        time::sleep(Duration::from_secs(2)).await;
        assert_eq!(
            cell.delivery(),
            Delivery::Done,
            "the ticket lived in the writer, which acknowledged after the call stopped"
        );
        let calls = tab.calls();
        assert_eq!(
            calls[calls.len() - 3..],
            ["released", "disarmed", "input released"],
            "disarmed, and the tab's input given back, only once the last event was answered"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_detached_element_resolves_again() {
        let mut tab = FakeTab::ready();
        tab.check_states = Step::new(
            STABLE,
            [Ok(StatesCheck::NotConnected), Ok(StatesCheck::Ready)],
        );
        let (result, _, _) = run(&tab, &options(SELECTOR), 30_000.0).await;

        assert_eq!(result, Ok(()));
        assert_eq!(tab.count("resolve"), 2);
        assert_eq!(tab.count("describe"), 1, "the locator is named once");
    }

    #[tokio::test(start_paused = true)]
    async fn an_interceptor_that_finds_the_element_detached_resolves_again() {
        let mut tab = FakeTab::ready();
        tab.arm = Step::new(
            ROUND_TRIP,
            [Ok(ArmAnswer::NotConnected), Ok(ArmAnswer::Armed)],
        );
        let (result, _, delivery) = run(&tab, &options(SELECTOR), 30_000.0).await;

        assert_eq!(result, Ok(()));
        assert_eq!(delivery, Delivery::Done);
        assert_eq!(tab.count("resolve"), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_retriable_page_error_resolves_again_silently() {
        let mut tab = FakeTab::ready();
        tab.geometry = Step::new(
            ROUND_TRIP,
            [
                Err(PageError::NotConnected),
                Ok(Geometry::Quads {
                    quads: vec![OK_QUAD],
                    viewport: (516.0, 475.0),
                }),
            ],
        );
        let (result, _, delivery) = run(&tab, &options(SELECTOR), 30_000.0).await;

        assert_eq!(result, Ok(()));
        assert_eq!(delivery, Delivery::Done);
        assert_eq!(tab.count("resolve"), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_preparation_retries_like_any_page_step() {
        let mut tab = FakeTab::ready();
        tab.prepare = Step::new(Duration::ZERO, [Err(PageError::NotConnected), Ok(())]);
        let (result, _, delivery) = run(&tab, &options(SELECTOR), 30_000.0).await;

        assert_eq!(result, Ok(()));
        assert_eq!(delivery, Delivery::Done);

        let mut tab = FakeTab::ready();
        tab.prepare = Step::always(Duration::ZERO, Err(PageError::Closed));
        let (result, _, delivery) = run(&tab, &options(SELECTOR), 30_000.0).await;
        assert_eq!(result, Err(PlaywrightError::target_closed().into()));
        assert_eq!(delivery, Delivery::NotSent);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_page_that_never_attaches_times_out_before_resolving() {
        let mut tab = FakeTab::ready();
        tab.describe = Step::always(ROUND_TRIP, Err(PageError::NotConnected));
        let (result, _, delivery) = run(&tab, &options(SELECTOR), 500.0).await;

        assert_eq!(result, timeout_error(500.0));
        assert_eq!(tab.count("resolve"), 0);
        assert_eq!(delivery, Delivery::NotSent);
    }

    #[tokio::test(start_paused = true)]
    async fn an_invalid_selector_fails_at_once_naming_it() {
        let mut tab = FakeTab::ready();
        tab.describe = Step::always(
            ROUND_TRIP,
            Ok(SelectorFacts {
                locator: "button >>".to_string(),
                invalid: Some("Unexpected token \">>\"".to_string()),
            }),
        );
        let (result, _, _) = run(&tab, &options("button >>"), 30_000.0).await;

        assert_eq!(
            result,
            Err(PlaywrightError::error("Unexpected token \">>\"").into())
        );
        assert_eq!(
            tab.count("resolve"),
            0,
            "nothing is resolved after a parse error"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_closed_tab_ends_the_call_with_target_closed() {
        let mut tab = FakeTab::ready();
        tab.check_states = Step::always(STABLE, Err(PageError::Closed));
        let (result, _, _) = run(&tab, &options(SELECTOR), 30_000.0).await;

        let Err(CallError::Playwright(error)) = result else {
            panic!("expected a Playwright error, got {result:?}");
        };
        assert_eq!(error.name, ErrorName::TargetClosedError);
    }

    #[tokio::test(start_paused = true)]
    async fn force_fails_at_once_on_an_invisible_element() {
        let mut tab = FakeTab::ready();
        tab.geometry = Step::always(ROUND_TRIP, Ok(Geometry::NoQuads));
        let mut forced = options(SELECTOR);
        forced.target.force = true;
        let (result, _, delivery) = run(&tab, &forced, 30_000.0).await;

        assert_eq!(
            result,
            Err(PlaywrightError::error("Element is not visible").into())
        );
        assert_eq!(tab.count("geometry"), 1, "no retry under force");
        assert_eq!(delivery, Delivery::NotSent);
    }

    #[tokio::test(start_paused = true)]
    async fn a_pending_navigation_is_waited_out_before_resolving() {
        let tab = FakeTab::ready();
        *tab.pending.lock().expect("pending") = Some("https://example.test/next".to_string());
        let (result, _, _) = run(&tab, &options(SELECTOR), 30_000.0).await;

        assert_eq!(result, Ok(()));
        assert_eq!(
            tab.calls()[0],
            "navigation_settled",
            "the navigation settles before anything is resolved: {:?}",
            tab.calls()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_disabled_wait_after_reads_the_verdict_without_a_navigation_barrier() {
        let mut tab = FakeTab::ready();
        tab.verdict = Step::new(
            ROUND_TRIP,
            [Verdict::Blocked(OVERLAY.into()), Verdict::Done],
        );
        let progress = Progress::for_test(30_000.0);
        let result = retry_pointer_action(
            &tab,
            &1,
            &PointerCall {
                target: &options(SELECTOR).target,
                action: PointerAction::Click {
                    click: MouseClick {
                        button: MouseButton::Left,
                        click_count: 1,
                        delay: None,
                    },
                    steps: 1,
                    wait_after: WaitAfter::Disabled,
                },
            },
            HeldModifiers::Keep,
            progress.delivery().expect("input"),
            &progress,
        )
        .await;
        assert!(matches!(result, Ok(ActionResult::Done(()))));
        assert_eq!(tab.count("verdict"), 2);
        assert_eq!(tab.count("watch_navigations"), 0);
        assert_eq!(tab.count("settle"), 0);
        assert_eq!(tab.count("input released"), 2);
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::Done
        );
    }

    #[test]
    fn options_refuse_what_neo_does_not_serve() {
        let decode = |params: serde_json::Value| {
            ClickOptions::from_params(serde_json::from_value(params).expect("click params"))
        };
        assert_eq!(
            decode(serde_json::json!({ "selector": "a", "steps": 1001 })),
            Err(PlaywrightError::error(
                "Neo does not support steps above 1000"
            ))
        );
        assert_eq!(
            decode(serde_json::json!({ "selector": "a", "clickCount": 1001 })),
            Err(PlaywrightError::error(
                "Neo does not support a clickCount above 1000"
            ))
        );
        let served = decode(serde_json::json!({
            "selector": "a", "modifiers": [], "steps": 1, "delay": -5, "clickCount": 2,
            "button": "right", "noWaitAfter": true, "scroll": "none",
        }))
        .expect("served");
        assert_eq!(
            served.click,
            MouseClick {
                button: MouseButton::Right,
                click_count: 2,
                delay: Some(Duration::ZERO),
            },
            "a negative delay is truthy in JS and sleeps 0"
        );
        assert!(
            served.wait_after == WaitAfter::Disabled
                && !served.target.scroll
                && !served.target.strict
        );
        let default = decode(serde_json::json!({ "selector": "a", "delay": 0 })).expect("served");
        assert_eq!(
            default.click,
            MouseClick {
                button: MouseButton::Left,
                click_count: 1,
                delay: None,
            },
            "a zero delay takes the pipelined path"
        );
    }

    // Tests invoke the same typed entry points as dispatch. This enum keeps comparisons
    // shared without reintroducing the old production option bag.
    #[derive(Debug, PartialEq)]
    enum TestAction {
        Click(ClickOptions),
        Dblclick(DblclickOptions),
        Hover(HoverOptions),
        Tap(TapOptions),
    }

    trait TestPointer {
        async fn invoke(&self, tab: &FakeTab, progress: &Progress) -> Result<(), CallError>;
    }
    impl TestPointer for ClickOptions {
        async fn invoke(&self, tab: &FakeTab, progress: &Progress) -> Result<(), CallError> {
            click(tab, self, progress).await
        }
    }
    impl TestPointer for TestAction {
        async fn invoke(&self, tab: &FakeTab, progress: &Progress) -> Result<(), CallError> {
            match self {
                Self::Click(options) => click(tab, options, progress).await,
                Self::Dblclick(options) => dblclick(tab, options, progress).await,
                Self::Hover(options) => hover(tab, options, progress).await,
                Self::Tap(options) => tap(tab, options, progress).await,
            }
        }
    }
    impl TestAction {
        fn modifiers(&self) -> Option<Vec<KeyboardModifier>> {
            match self {
                Self::Click(options) => options.modifiers.clone(),
                Self::Dblclick(options) => options.modifiers.clone(),
                Self::Hover(options) => options.modifiers.clone(),
                Self::Tap(options) => options.modifiers.clone(),
            }
        }
        fn steps(&self) -> Option<u32> {
            match self {
                Self::Click(options) => Some(options.steps),
                Self::Dblclick(options) => Some(options.steps),
                Self::Hover(_) | Self::Tap(_) => None,
            }
        }
    }

    fn decoded(method: &str, params: serde_json::Value) -> Result<TestAction, PlaywrightError> {
        match method {
            "dblclick" => {
                DblclickOptions::from_params(serde_json::from_value(params).expect("params"))
                    .map(TestAction::Dblclick)
            }
            "hover" => Ok(TestAction::Hover(HoverOptions::from_params(
                serde_json::from_value(params).expect("params"),
            ))),
            "tap" => Ok(TestAction::Tap(TapOptions::from_params(
                serde_json::from_value(params).expect("params"),
            ))),
            _ => ClickOptions::from_params(serde_json::from_value(params).expect("params"))
                .map(TestAction::Click),
        }
    }

    /// Each action's calls after the shared start (`describe`, `resolve`), so the three can be
    /// read side by side: what it waited for, what its interceptor judged, what it wrote, and
    /// that none of them waits for navigations.
    async fn served(method: &str) -> (Vec<String>, Vec<Vec<ElementState>>, Delivery) {
        let tab = FakeTab::ready();
        let options = decoded(
            method,
            serde_json::json!({ "selector": SELECTOR, "strict": true }),
        )
        .expect("served");
        let (result, _, delivery) = run(&tab, &options, 30_000.0).await;
        assert_eq!(result, Ok(()), "{method}");
        let states = tab.states.lock().expect("states").clone();
        (tab.calls()[2..].to_vec(), states, delivery)
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_hover_waits_for_visible_and_stable_and_writes_one_move() {
        let (calls, states, delivery) = served("hover").await;
        assert_eq!(states, [HOVER_STATES.to_vec()], "never waits for enabled");
        assert_eq!(
            calls,
            [
                "check_states",
                "scroll Protocol",
                "geometry",
                "input",
                "arm Hover Point { x: 36.75, y: 18.5 }",
                "move (36.75, 18.5)",
                "released",
                "verdict",
                "disarmed",
                "input released",
            ],
            "hover's interceptor judges its move; no navigation barrier"
        );
        assert_eq!(delivery, Delivery::Done);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_dblclick_presses_twice_without_a_navigation_barrier() {
        let (calls, states, delivery) = served("dblclick").await;
        assert_eq!(states, [CLICK_STATES.to_vec()]);
        assert!(calls.contains(&"arm Mouse Point { x: 36.75, y: 18.5 }".to_string()));
        assert!(
            calls.contains(&"press twice (36.75, 18.5)".to_string()),
            "{calls:?}"
        );
        assert!(
            !calls.contains(&"watch_navigations".to_string())
                && !calls.contains(&"settle".to_string()),
            "Playwright's dblclick waits for no navigation: {calls:?}"
        );
        assert_eq!(delivery, Delivery::Done);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_tap_arms_for_touches_and_writes_a_tap() {
        let (calls, states, delivery) = served("tap").await;
        assert_eq!(states, [CLICK_STATES.to_vec()], "tap waits for enabled");
        assert!(
            calls.contains(&"arm Tap Point { x: 36.75, y: 18.5 }".to_string()),
            "{calls:?}"
        );
        assert!(
            calls.contains(&"tap (36.75, 18.5)".to_string()),
            "{calls:?}"
        );
        assert!(!calls.contains(&"settle".to_string()));
        assert_eq!(delivery, Delivery::Done);
    }

    /// A hover whose move the interceptor blocked (the element was covered by then) moves again,
    /// as a blocked click presses again; a forced hover arms nothing.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_blocked_hover_moves_again_and_a_forced_one_arms_nothing() {
        let mut tab = FakeTab::ready();
        tab.verdict = Step::new(
            ROUND_TRIP,
            [Verdict::Blocked(OVERLAY.to_string()), Verdict::Done],
        );
        let hover = decoded("hover", serde_json::json!({ "selector": SELECTOR })).expect("served");
        let (result, _, delivery) = run(&tab, &hover, 30_000.0).await;
        assert_eq!(result, Ok(()));
        assert_eq!(delivery, Delivery::Done);
        assert_eq!(tab.count("move (36.75, 18.5)"), 2, "{:?}", tab.calls());

        let tab = FakeTab::ready();
        let forced = decoded(
            "hover",
            serde_json::json!({ "selector": SELECTOR, "force": true }),
        )
        .expect("served");
        let (result, _, _) = run(&tab, &forced, 30_000.0).await;
        assert_eq!(result, Ok(()));
        assert!(
            !tab.calls().iter().any(|call| call.starts_with("arm ")),
            "{:?}",
            tab.calls()
        );
    }

    /// `scroll: 'none'` reaches the states check, so an element in an offscreen frame is not
    /// scrolled into view there either; Playwright scrolls nothing under it.
    #[tokio::test(start_paused = true)]
    async fn tests_that_scroll_none_checks_states_without_scrolling_frames() {
        for method in ["click", "dblclick", "hover", "tap"] {
            let tab = FakeTab::ready();
            let options = decoded(
                method,
                serde_json::json!({ "selector": SELECTOR, "scroll": "none" }),
            )
            .expect("served");
            let (result, _, _) = run(&tab, &options, 30_000.0).await;
            assert_eq!(result, Ok(()), "{method}");
            let calls = tab.calls();
            assert!(
                calls.contains(&"check_states without scrolling".to_string())
                    && !calls.iter().any(|call| call.starts_with("scroll ")),
                "{method}: {calls:?}"
            );
        }
    }

    #[test]
    fn tests_that_every_pointer_action_serves_modifiers_and_steps() {
        for method in ["click", "dblclick", "hover", "tap"] {
            let options = decoded(
                method,
                serde_json::json!({ "selector": "a", "modifiers": ["Shift", "ControlOrMeta"] }),
            )
            .expect("served");
            assert_eq!(
                options.modifiers(),
                Some(vec![
                    KeyboardModifier::Shift,
                    KeyboardModifier::ControlOrMeta
                ]),
                "{method}"
            );
            let empty = decoded(
                method,
                serde_json::json!({ "selector": "a", "modifiers": [] }),
            )
            .expect("served");
            assert_eq!(
                empty.modifiers(),
                Some(Vec::new()),
                "{method}: [] releases them all"
            );
            let kept = decoded(method, serde_json::json!({ "selector": "a" })).expect("served");
            assert_eq!(
                kept.modifiers(),
                None,
                "{method}: no option keeps the keyboard's"
            );
        }
        for method in ["click", "dblclick"] {
            let options = decoded(method, serde_json::json!({ "selector": "a", "steps": 5 }))
                .expect("served");
            assert_eq!(options.steps(), Some(5), "{method}");
            let none = decoded(method, serde_json::json!({ "selector": "a", "steps": -3 }))
                .expect("served");
            assert_eq!(
                none.steps(),
                Some(0),
                "{method}: fewer than one step moves without an event"
            );
            assert_eq!(
                decoded(
                    method,
                    serde_json::json!({ "selector": "a", "steps": 1001 })
                ),
                Err(PlaywrightError::error(
                    "Neo does not support steps above 1000"
                )),
            );
        }
        let dblclick = decoded(
            "dblclick",
            serde_json::json!({ "selector": "a", "button": "middle", "delay": 10, "steps": 1 }),
        )
        .expect("served");
        let TestAction::Dblclick(dblclick) = dblclick else {
            panic!("double click decoder");
        };
        assert_eq!(
            MouseClick {
                button: dblclick.button,
                click_count: 2,
                delay: dblclick.delay
            },
            MouseClick {
                button: MouseButton::Middle,
                click_count: 2,
                delay: Some(Duration::from_millis(10)),
            }
        );
        assert!(!dblclick.target.strict);
    }

    /// Each planned event of `plan`: `keyUp Shift` for a key, `mousePressed modifiers=8` for a
    /// mouse event.
    fn plan_events(plan: &Plan) -> Vec<String> {
        plan.commands
            .iter()
            .filter(|command| command.method().is_some())
            .map(|command| {
                let event: Value = serde_json::from_str(command.params()).expect("params are JSON");
                let kind = event["type"].as_str().unwrap_or_default();
                if command.method() == Some("Input.dispatchKeyEvent") {
                    format!("{kind} {}", event["key"].as_str().unwrap_or_default())
                } else {
                    format!("{kind} modifiers={}", event["modifiers"])
                }
            })
            .collect()
    }

    /// Where each planned `mouseMoved` goes.
    fn plan_moves(plan: &Plan) -> Vec<(f64, f64)> {
        plan.commands
            .iter()
            .filter_map(|command| {
                let event: Value = serde_json::from_str(command.params()).ok()?;
                (event["type"] == "mouseMoved").then(|| {
                    (
                        event["x"].as_f64().unwrap_or_default(),
                        event["y"].as_f64().unwrap_or_default(),
                    )
                })
            })
            .collect()
    }

    /// Design section 4.3: a pointer action plans from the tab's devices. A Shift held by an
    /// earlier `keyboard.down` reaches a click; `modifiers: []` lifts it for the click and holds
    /// it again after; `ControlOrMeta` is the mac's Meta, swapped in and out in `kModifiers`
    /// order; `steps` start at the pointer's last position; the platform is read only when
    /// `modifiers` is given.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_pointer_action_plans_from_the_tabs_devices() {
        let tab = FakeTab::ready();
        let place_pointer = |tab: &FakeTab| {
            let mut devices = tab.devices.lock().expect("devices");
            devices.mouse.x = 0.75;
            devices.mouse.y = 0.5;
        };
        {
            let mut devices = tab.devices.lock().expect("devices");
            devices.keyboard.modifiers = keyboard_input::SHIFT;
            devices.keyboard.pressed = vec![("ShiftLeft", fake_owner())];
        }
        let click = |params: serde_json::Value| {
            let mut all = serde_json::json!({ "selector": SELECTOR });
            if let (Some(all), Some(params)) = (all.as_object_mut(), params.as_object()) {
                all.extend(params.clone());
            }
            decoded("click", all).expect("served")
        };

        let (result, _, delivery) = run(&tab, &click(serde_json::json!({})), 30_000.0).await;
        assert_eq!((result, delivery), (Ok(()), Delivery::Done));
        assert_eq!(tab.count("platform"), 0, "no modifiers, no platform");

        place_pointer(&tab);
        let options = click(serde_json::json!({ "modifiers": [], "steps": 2 }));
        let (result, _, delivery) = run(&tab, &options, 30_000.0).await;
        assert_eq!((result, delivery), (Ok(()), Delivery::Done));
        assert_eq!(tab.count("platform"), 1);

        let (result, _, delivery) = run(
            &tab,
            &click(serde_json::json!({ "modifiers": ["ControlOrMeta"] })),
            30_000.0,
        )
        .await;
        assert_eq!((result, delivery), (Ok(()), Delivery::Done));

        let plans = tab.plans.lock().expect("plans").clone();
        assert_eq!(plans.len(), 3);
        assert_eq!(
            plan_events(&plans[0]),
            [
                "mouseMoved modifiers=8",
                "mousePressed modifiers=8",
                "mouseReleased modifiers=8"
            ],
            "the held Shift reaches the click"
        );
        assert_eq!(
            plan_events(&plans[1]),
            [
                "keyUp Shift",
                "mouseMoved modifiers=0",
                "mouseMoved modifiers=0",
                "mousePressed modifiers=0",
                "mouseReleased modifiers=0",
                "rawKeyDown Shift"
            ],
            "modifiers: [] lifts Shift for the click only"
        );
        let moves = plan_moves(&plans[1]);
        let to = moves[1];
        assert_eq!(
            moves[0],
            (f64::midpoint(0.75, to.0), f64::midpoint(0.5, to.1)),
            "the first of two steps is halfway from the last position"
        );
        assert_eq!(
            plan_events(&plans[2]),
            [
                "rawKeyDown Meta",
                "keyUp Shift",
                "mouseMoved modifiers=4",
                "mousePressed modifiers=4",
                "mouseReleased modifiers=4",
                "keyUp Meta",
                "rawKeyDown Shift"
            ],
            "ControlOrMeta on mac, in kModifiers order both ways"
        );
        let keyboard = tab.devices.lock().expect("devices").keyboard.clone();
        assert_eq!(
            keyboard.modifiers,
            keyboard_input::SHIFT,
            "Shift is held again"
        );
        assert_eq!(keyboard.pressed, [("ShiftLeft", fake_owner())]);
    }

    /// Known review issue: `clickCount: 0` with `steps: 0` writes no event, as Playwright's
    /// does, and resolves: no writer runs, the delivery stays `NotSent`, and the pointer still
    /// moves to the click point.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_click_with_nothing_to_write_resolves_without_input() {
        let tab = FakeTab::ready();
        let options = decoded(
            "click",
            serde_json::json!({ "selector": SELECTOR, "clickCount": 0, "steps": 0 }),
        )
        .expect("served");
        let (result, log, delivery) = run(&tab, &options, 30_000.0).await;
        assert_eq!(result, Ok(()), "{log:?}");
        assert_eq!(delivery, Delivery::NotSent);
        assert!(tab.plans.lock().expect("plans").is_empty(), "no writer ran");
        let mouse = tab.devices.lock().expect("devices").mouse.clone();
        assert!(
            (mouse.x, mouse.y) != (0.0, 0.0),
            "the pointer moved: {mouse:?}"
        );
    }
}
