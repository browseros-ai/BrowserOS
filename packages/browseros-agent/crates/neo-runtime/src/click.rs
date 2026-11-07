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
//! ```text
//!  pointer_action(tab, options, progress)                log: waiting for <locator>
//!   loop A  waits 0,20,50,100,100,500…  (entries > timeout/5 dropped; the last repeats)
//!   │ prepare the tab (this page's first click enables its domains; then a no-op)
//!   │ unless force: wait out a pending main-frame navigation
//!   │ first time only: describe → name the locator │ invalid selector → Error, at once
//!   │ resolve(selector, strict) → element? │ throws strict violation → Error, at once
//!   │ loop B  waits 0,0,20,100,100,500… on the same element (uncapped)
//!   │ │ unless force: the navigation pre-check again
//!   │ │ states (hover: visible, stable; the others: + enabled, non-inert) → scroll
//!   │ │   (mode = retry % 4) → geometry → point
//!   │ │ wait for the tab's input turn → arm the hit-target interceptor for the action's input
//!   │ │   not ready (not visible, outside viewport, missing state, intercepted) → log, retry B
//!   │ │   not connected → "element was detached from the DOM, retrying", next loop A
//!   │ └ input: begin ticket → move, click or tap (detached writer) → verdict, turn over →
//!   │     navigation barrier (click without noWaitAfter only)
//!   │     a verified block (verdict names another element) → ticket back to NotSent, retry B
//!   │     anything else after the press is success or a terminal error, never a retry
//! ```
//!
//! [`Tab`] is the seam between this policy and CDP: one method per step of Playwright's own
//! attempt. `page::PageSession` implements it over CDP and the glue; the tests implement it with
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
//! - A CDP failure after a press was written ends the call with `Error` and delivery `Unknown`;
//!   Playwright's loop A would retry it and might press again.
//! - A `clickCount` above [`MAX_CLICK_COUNT`] and `steps` above [`MAX_STEPS`] are refused
//!   before any work, with no call log.
//! - Pointer actions on one tab take turns at its input ([`Tab::input`]), across every run of the
//!   `Runtime`. Playwright lets a second click's setup replace the first one's interceptor
//!   (`setupHitTargetInterceptor`'s own comment), so overlapping clicks judge each other's events.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use neo_runtime_spec::injected::{ElementState, HitTargetAction, Point};
use neo_runtime_spec::serve::{
    FrameClickParams, FrameClickParamsButton, FrameClickParamsModifiersItem,
    FrameClickParamsScroll, FrameDblclickParams, FrameDblclickParamsButton,
    FrameDblclickParamsModifiersItem, FrameDblclickParamsScroll, FrameHoverParams,
    FrameHoverParamsModifiersItem, FrameHoverParamsScroll, FrameTapParams,
    FrameTapParamsModifiersItem, FrameTapParamsScroll, Point as WirePoint,
};

use crate::input_devices::{self, Devices, EffectSink, Plan};
use crate::keyboard::{self, KeyboardModifier, KeyboardTab, Platform};
use crate::progress::{
    AckedInput, CallError, DeliveryCell, InputTicket, PlaywrightError, Progress,
};

// ---------------------------------------------------------------------------------------------
// The seam: what click needs from a tab, in domain terms.
// ---------------------------------------------------------------------------------------------

/// What click needs from its target tab. Each method is one Playwright step: one page call, one
/// CDP command, or one wait. Every returned future is cancel-safe: dropping it in
/// [`Progress::race`] loses only the answer. Implemented by `Arc<PageSession>` (`page.rs`) and
/// by the tests' `FakeTab`.
pub(crate) trait Tab: Send + Sync {
    /// One resolved element, held for one loop-A iteration. Dropping it releases it (the real
    /// one owns a CDP object group and releases it on the run's tracker).
    type Element: Send + Sync;
    /// An armed hit-target interceptor. Dropping one without a successful [`Tab::verdict`]
    /// disarms it in the background, so every path out of an attempt disarms it: a leaked
    /// armed interceptor would keep swallowing mismatched clicks in the user's tab, because the
    /// injected script's slot outlives the run.
    ///
    /// It has one owner at a time, from the setup's send to the stop's answer: [`Tab::arm`]'s
    /// task until a waiting caller takes it, the attempt until it presses, the writer until
    /// the last mouse event is acknowledged ([`Tab::press`]), then the attempt again for the
    /// verdict. Whichever owner goes away drops it, so a stop never lands before input it was
    /// armed to judge. It holds a copy of its click's input turn until the stop has answered.
    type Armed: Send;
    /// A turn at the tab's input ([`Tab::input`]). Copies go to everything that operates the
    /// click's input: the interceptor's setup and the interceptor it arms, and the writer. The
    /// next click's turn starts only once the last copy drops, so a call that stops early never
    /// frees the tab under its own writer or its interceptor's stop.
    type Input: Devices;
    /// Navigation requests recorded from the moment [`Tab::watch_navigations`] returns.
    type Navigations: Send;

    /// Prepares this page's domains and watcher once, on its first click or beforeunload call.
    /// Later calls find it done. Playwright's page is
    /// ready before any call, so this comes after the call names its locator, and a failure
    /// retries like any page step.
    fn prepare(&self) -> impl Future<Output = Result<(), PageError>> + Send;

    /// `describeSelector`: the `waiting for` text, or the parse error.
    fn describe(
        &self,
        selector: &str,
    ) -> impl Future<Output = Result<SelectorFacts, PageError>> + Send;

    /// `resolve`: the first match with the match count and its preview, `None` while nothing
    /// matches. A strict violation is `PageError::Threw` with V8's `Error: strict mode …`.
    fn resolve(
        &self,
        selector: &str,
        strict: bool,
    ) -> impl Future<Output = Result<Option<Resolved<Self::Element>>, PageError>> + Send;

    /// `checkElementStates` with `frameVisible` inherited from the selector's iframe hop
    /// owners (`true` when the selector has no frame hops). With `scroll` (not Playwright's
    /// `scroll: 'none'`), an element in a child frame first has its frames' owners scrolled
    /// into view, so an offscreen frame paints and `stable` can resolve.
    fn check_states(
        &self,
        element: &Self::Element,
        states: &[ElementState],
        scroll: bool,
    ) -> impl Future<Output = Result<StatesCheck, PageError>> + Send;

    /// [`Scroll::Protocol`]: CDP `DOM.scrollIntoViewIfNeeded` (`position` as a zero rect); the
    /// other modes: the `scrollIntoView` glue. Returns why it could not scroll, if so.
    fn scroll(
        &self,
        element: &Self::Element,
        scroll: Scroll,
        position: Option<&Point>,
    ) -> impl Future<Output = Result<Option<NotReady>, PageError>> + Send;

    /// The raw facts the click point is computed from: `DOM.getContentQuads` with the
    /// `viewportSize` glue, or for `position`, `DOM.getBoxModel` with `getElementBorderWidth`.
    fn geometry(
        &self,
        element: &Self::Element,
        position: Option<&Point>,
        force: bool,
    ) -> impl Future<Output = Result<Geometry, PageError>> + Send;

    /// Waits for a turn at the tab's input: its one mouse and its document's one hit-target
    /// slot, which every call and every run on the tab share. Without turns, one click's setup
    /// replaces another's armed interceptor, and their events are judged by the wrong one. An
    /// attempt waits only after every wait for its element, so a click waiting for content never
    /// holds up the click that would produce it. Turns go in arrival order; a wait dropped before
    /// its turn holds nothing.
    fn input(&self) -> impl Future<Output = Self::Input> + Send;

    /// `setupHitTargetInterceptor(element, action, hit_point, false)`: the hit check at
    /// `hit_point`, then the interceptor armed for the coming input, judging the events of
    /// `action` (a hover's `mousemove`, a tap's touches, a click's presses). The page installs the
    /// interceptor before it answers, so once the setup is sent its answer is always read, on a
    /// task of its own: a caller that stopped waiting leaves an `Armed` nobody takes, which
    /// disarms as it drops. A future dropped before its first poll sends nothing. `input` is
    /// held until the answer, and an armed interceptor keeps it until its stop has answered.
    fn arm(
        &self,
        element: &Self::Element,
        hit_point: Point,
        action: HitTargetAction,
        input: Self::Input,
    ) -> impl Future<Output = Result<Arming<Self::Armed>, PageError>> + Send;

    /// Starts writing the action's input (a move, a click's mouse events, or a tap's touches) on a
    /// detached writer task *before it returns*, and gives back a future of the writer's report.
    /// The ticket moves into the writer, which marks it as it goes, so delivery stays true however
    /// the caller ends: dropping the returned future (the call stopped) stops the wait, never the
    /// writes. A press is always followed by its release, a touch start by its end, and a `delay`
    /// is cut short. Starting eagerly matters: a future that started the writer only when first
    /// polled could be dropped unpolled by a [`Progress::race`] that had already lost, leaving a
    /// begun ticket nobody marks.
    ///
    /// The armed interceptor moves into the writer too, and comes back with the report once
    /// every event is acknowledged. A caller that stopped waiting never gets it back, and the
    /// writer drops it only then: disarming any earlier would let the release of a press it
    /// blocked, and the page's `click`, through to whatever is under the pointer. The writer
    /// holds `input` until its last event is answered, which under `force`, with nothing armed,
    /// is all that keeps the next click from pressing at once.
    fn press(
        &self,
        plan: Plan,
        ticket: InputTicket,
        armed: Option<Self::Armed>,
        input: Self::Input,
    ) -> impl Future<Output = (PressReport, Option<Self::Armed>)> + Send;

    /// Disarms `armed` and reports what it saw: `stopHitTargetInterceptor`, raced against a
    /// dialog opening or a navigation being requested since it was armed (Playwright's
    /// `raceAgainstEvaluationStallingEvents`), either of which reads as `Done`, as does any
    /// error (Playwright's `.catch(() => 'done')`). Never fails.
    fn verdict(&self, armed: Self::Armed) -> impl Future<Output = Verdict> + Send;

    /// Playwright's pre-check: the URL of a main-frame navigation requested and not yet
    /// committed, finished in the document, or stopped.
    fn pending_navigation(&self) -> Option<String>;

    /// Resolves when no navigation is pending, with the URL it committed to, if any.
    fn navigation_settled(&self) -> impl Future<Output = Result<Option<String>, PageError>> + Send;

    /// Starts recording navigation requests. Called before `press`, so none of the input's own
    /// requests can be missed.
    fn watch_navigations(&self) -> Self::Navigations;

    /// Playwright's input epilogue (`waitForSignalsCreatedBy`): a flush after which every
    /// navigation the input requested has been reported, then a wait until each requested
    /// main-frame navigation commits or stops. Returns the URLs committed, for the log.
    fn settle_after_input(
        &self,
        watched: Self::Navigations,
    ) -> impl Future<Output = Result<Vec<String>, PageError>> + Send;
}

/// What `describeSelector` learned, once per call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectorFacts {
    /// `asLocator('javascript', selector)`, for the first log line.
    pub(crate) locator: String,
    /// The parse error's message (no `Error: `), which ends the call at once.
    pub(crate) invalid: Option<String>,
}

/// The first element a selector matched.
#[derive(Debug)]
pub(crate) struct Resolved<E> {
    pub(crate) element: E,
    /// How many elements matched; above one only without strict mode.
    pub(crate) count: u32,
    /// `InjectedScript.previewNode` of the element.
    pub(crate) preview: String,
}

/// `checkElementStates`' answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StatesCheck {
    Ready,
    NotConnected,
    Missing(ElementState),
}

/// Raw facts for the click point; [`click_point`] turns them into Playwright's point.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Geometry {
    /// `getContentQuads` or `getBoxModel` failed or found nothing: `error:notvisible`
    /// (Playwright's `_sendMayFail`).
    NoQuads,
    Quads {
        quads: Vec<[f64; 8]>,
        viewport: (f64, f64),
    },
    /// With `position`: the border box quad and the element's border widths.
    Offset {
        border_box: [f64; 8],
        border: (f64, f64),
    },
}

/// Why an attempt did not reach input: Playwright's `PerformActionResult` minus `done`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotReady {
    /// `error:notconnected`: back to loop A to resolve again.
    NotConnected,
    NotVisible,
    NotInViewport,
    Missing(ElementState),
    /// The hit target is another element: its preview.
    Intercepted(String),
}

/// `setupHitTargetInterceptor`'s answer.
#[derive(Debug)]
pub(crate) enum Arming<A> {
    Armed(A),
    NotConnected,
    /// The preliminary hit test found another element: its preview.
    Intercepted(String),
}

/// What the armed interceptor saw at the first trusted pointer event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The event hit the element, or no event arrived, or a dialog or navigation made reading
    /// it moot, or reading it failed: Playwright's `'done'`.
    Done,
    /// The event hit another element, and the interceptor blocked it and every later event.
    Blocked(String),
}

/// How far the writer got. The ticket's state already says the same; this is what the call
/// task acts on.
#[derive(Debug)]
pub(crate) enum PressReport {
    /// Every event that was written was acknowledged.
    Acked(AckedInput),
    /// Nothing was written (the writer called `InputTicket::unsent`).
    NothingWritten,
    /// Something may have been written, then CDP failed; delivery stays `Unknown`. `closed`
    /// when the session or the connection is gone, which reads as the tab closing.
    Failed { closed: bool, message: String },
}

/// Why a page step could not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PageError {
    /// Any protocol error other than a closed session: the context, document or object is
    /// gone, or the frame is mid-commit. Playwright retries these ("Execution context was
    /// destroyed" and kin), so does Neo, without matching error text.
    NotConnected,
    /// The page threw: `exceptionDetails.exception.description` (`Error: <message>` for
    /// Playwright's stackless errors). Ends the call (`JavaScriptErrorInEvaluate`).
    Threw(String),
    /// The tab closed or the session is gone.
    Closed,
    /// The page answered outside the spec: a bundle/spec mismatch. Ends the call.
    Contract(String),
}

/// A click's mouse events, decided before any is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MouseClick {
    pub(crate) button: MouseButton,
    pub(crate) click_count: u32,
    /// Playwright's `delay` when truthy: the sequential path, sleeping between press and
    /// release (zero for a negative delay, as `setTimeout` treats it).
    pub(crate) delay: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MouseButton {
    Left,
    Right,
    Middle,
}

/// What a pointer action writes at its point, decided before anything is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PointerInput {
    /// Playwright: `server/dom.ts` `ElementHandle._hover`: `perform: (progress, point) => this._page.mouse.move(progress, point.x, point.y),`
    Move,
    /// Playwright: `server/dom.ts` `ElementHandle._click`: `perform: (progress, point) => this._page.mouse.click(progress, point.x, point.y, options),`
    Click(MouseClick),
    /// Playwright: `server/dom.ts` `ElementHandle._tap`: `perform: (progress, point) => this._page.touchscreen.tap(progress, point.x, point.y),`
    Tap,
    /// Playwright: `server/frames.ts` `Frame.dragAndDrop`: `await this._page.mouse.down(progress);`
    MoveAndDown,
    /// Playwright: `server/frames.ts` `Frame.dragAndDrop`: `await this._page.mouse.up(progress);`
    MoveAndUp,
}

/// Playwright's `PointerAction` (`dom.ts`): the action loop B performs once the element is
/// ready. Its name goes into the call log; [`PointerAction::states`] are what it waits for,
/// [`PointerAction::hit_target`] what its interceptor judges, and [`PointerAction::input`] what
/// its writer sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PointerAction {
    Click(MouseClick),
    /// A click of `clickCount: 2` under its own name; its [`MouseClick`] always counts two.
    Dblclick(MouseClick),
    Hover,
    Tap,
    /// `dragAndDrop`'s first half, on the source: a move, then a left press.
    MoveAndDown,
    /// `dragAndDrop`'s second half, on the target: a move in `steps`, then the release, which
    /// drops when the press started a drag.
    MoveAndUp,
}

impl PointerAction {
    /// `action.name`, as the call log says it (`attempting hover action`).
    /// Playwright: `server/dom.ts` `ElementHandle._retryPointerAction`: `return await this._retryAction(progress, action.name, async (progress, retry) => {`
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Click(_) => "click",
            Self::Dblclick(_) => "dblclick",
            Self::Hover => "hover",
            Self::Tap => "tap",
            Self::MoveAndDown => "move and down",
            Self::MoveAndUp => "move and up",
        }
    }

    /// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `const elementStates: ElementState[] = action.waitForEnabled ? ['visible', 'enabled', 'stable', 'non-inert'] : ['visible', 'stable'];`
    fn states(self) -> &'static [ElementState] {
        match self {
            Self::Hover | Self::MoveAndDown | Self::MoveAndUp => &HOVER_STATES,
            Self::Click(_) | Self::Dblclick(_) | Self::Tap => &CLICK_STATES,
        }
    }

    /// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `action.waitForEnabled ? 'visible, enabled and stable' : 'visible and stable'`
    fn states_text(self) -> &'static str {
        match self {
            Self::Hover | Self::MoveAndDown | Self::MoveAndUp => "visible and stable",
            Self::Click(_) | Self::Dblclick(_) | Self::Tap => "visible, enabled and stable",
        }
    }

    /// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `const actionType = action.name === 'move and up' ? 'drag' : ((action.name === 'hover' || action.name === 'tap') ? action.name : 'mouse');`
    pub(crate) fn hit_target(self) -> HitTargetAction {
        match self {
            Self::Hover => HitTargetAction::Hover,
            Self::Tap => HitTargetAction::Tap,
            Self::MoveAndUp => HitTargetAction::Drag,
            Self::Click(_) | Self::Dblclick(_) | Self::MoveAndDown => HitTargetAction::Mouse,
        }
    }

    pub(crate) fn input(self) -> PointerInput {
        match self {
            Self::Click(click) | Self::Dblclick(click) => PointerInput::Click(click),
            Self::Hover => PointerInput::Move,
            Self::Tap => PointerInput::Tap,
            Self::MoveAndDown => PointerInput::MoveAndDown,
            Self::MoveAndUp => PointerInput::MoveAndUp,
        }
    }
}

/// The scroll of one loop-B attempt, by retry index (`dom.ts` `_retryPointerAction`): CDP
/// first, then the page's `scrollIntoView` with `end`, `center`, `start`, which gets an element
/// out from under a sticky header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scroll {
    Protocol,
    End,
    Center,
    Start,
}

const SCROLLS: [Scroll; 4] = [Scroll::Protocol, Scroll::End, Scroll::Center, Scroll::Start];

/// Loop A's backoff after its immediate first try (`frames.ts` `retryWithProgressAndBackoff`).
const LOOP_A_BACKOFF_MS: [u32; 5] = [20, 50, 100, 100, 500];
/// Loop B's waits by retry index, the last repeating, never capped (`dom.ts` `_retryAction`).
const LOOP_B_WAITS_MS: [u64; 5] = [0, 20, 100, 100, 500];
/// The states every pointer action but hover waits for, in Playwright's order.
/// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `['visible', 'enabled', 'stable', 'non-inert']`
const CLICK_STATES: [ElementState; 4] = [
    ElementState::Visible,
    ElementState::Enabled,
    ElementState::Stable,
    ElementState::NonInert,
];
/// Hover's states: it never waits for an enabled element (`waitForEnabled: false`).
/// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `['visible', 'stable']`
const HOVER_STATES: [ElementState; 2] = [ElementState::Visible, ElementState::Stable];
/// The longest URL Playwright's navigation pre-check prints.
const LOGGED_URL_CHARS: usize = 200;
/// Deviation: `server/input.ts` `Mouse.click`: `for (let cc = 1; cc <= clickCount; ++cc) {` Playwright has no cap; every click is two events Neo plans before writing, so a script's `clickCount: 1e9` must not take the server's memory with it.
const MAX_CLICK_COUNT: u32 = 1000;
/// Deviation: `server/input.ts` `Mouse.move`: `for (let i = 1; i <= steps; i++) {` Playwright has no cap; every step is one event Neo plans before writing, so `steps: 1e9` must not take the server's memory with it.
const MAX_STEPS: u32 = 1000;

// ---------------------------------------------------------------------------------------------
// The algorithm.
// ---------------------------------------------------------------------------------------------

/// One pointer action's options, decoded once from Playwright's params by the dispatcher's
/// route.
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is one of Playwright's independent pointer-action options"
)]
pub(crate) struct PointerOptions {
    pub(crate) selector: String,
    /// `strict: true` from a locator; absent (non-strict) from the `page`/`frame` forms.
    pub(crate) strict: bool,
    pub(crate) force: bool,
    /// `scroll: 'none'` skips scrolling (the log lines stay, as in Playwright).
    pub(crate) scroll: bool,
    /// Playwright's `waitAfter: true`: wait for the navigations the input requested. Only a
    /// click without `noWaitAfter` has it; `dblclick`, `hover` and `tap` are `'disabled'`, as are
    /// check and uncheck. The verdict is read either way.
    pub(crate) wait_after: bool,
    pub(crate) position: Option<Point>,
    pub(crate) action: PointerAction,
    pub(crate) trial: bool,
    /// Playwright's `modifiers`: the modifiers held during the input, pressed and released
    /// around it and restored after (`ensureModifiers`); `None` keeps the keyboard's, and an
    /// empty list releases every held one for the input.
    pub(crate) modifiers: Option<Vec<KeyboardModifier>>,
    /// Playwright's `steps`: the pointer moves in this many steps from its last position.
    pub(crate) steps: u32,
    /// The browser's platform, read by [`pointer_action`] when `modifiers` are given: it decides
    /// `ControlOrMeta` and the mac editing commands of the modifier keys.
    pub(crate) platform: Option<Platform>,
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

impl PointerOptions {
    /// Reads `Frame.click`'s params. Refuses before any work, so the reply has no log, more than
    /// [`MAX_CLICK_COUNT`] clicks and more than [`MAX_STEPS`] steps.
    pub(crate) fn from_click(params: FrameClickParams) -> Result<Self, PlaywrightError> {
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
        let steps = steps_of(steps)?;
        let click_count = click_count_of(click_count)?;
        let button = match button.unwrap_or(FrameClickParamsButton::Left) {
            FrameClickParamsButton::Left => MouseButton::Left,
            FrameClickParamsButton::Right => MouseButton::Right,
            FrameClickParamsButton::Middle => MouseButton::Middle,
        };
        Ok(Self {
            selector,
            strict: strict.unwrap_or(false),
            force: force.unwrap_or(false),
            scroll: scroll != Some(FrameClickParamsScroll::None),
            wait_after: !no_wait_after.unwrap_or(false),
            position: position.as_ref().map(point),
            action: PointerAction::Click(MouseClick {
                button,
                click_count,
                delay: click_delay(delay),
            }),
            trial: trial.unwrap_or(false),
            modifiers: keyboard_modifiers!(modifiers, FrameClickParamsModifiersItem),
            steps,
            platform: None,
        })
    }

    /// Reads `Frame.dblclick`'s params: a click of two with its `button` and `delay`. The
    /// protocol has no `noWaitAfter` for it (the client drops the option), and Playwright never
    /// waits for its navigations. Refuses `steps` as [`Self::from_click`] does.
    pub(crate) fn from_dblclick(params: FrameDblclickParams) -> Result<Self, PlaywrightError> {
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
        let steps = steps_of(steps)?;
        let button = match button.unwrap_or(FrameDblclickParamsButton::Left) {
            FrameDblclickParamsButton::Left => MouseButton::Left,
            FrameDblclickParamsButton::Right => MouseButton::Right,
            FrameDblclickParamsButton::Middle => MouseButton::Middle,
        };
        Ok(Self {
            selector,
            strict: strict.unwrap_or(false),
            force: force.unwrap_or(false),
            scroll: scroll != Some(FrameDblclickParamsScroll::None),
            wait_after: false,
            position: position.as_ref().map(point),
            // Playwright: `server/dom.ts` `ElementHandle._dblclick`: `{ ...options, clickCount: 2 }`
            action: PointerAction::Dblclick(MouseClick {
                button,
                click_count: 2,
                delay: click_delay(delay),
            }),
            trial: trial.unwrap_or(false),
            modifiers: keyboard_modifiers!(modifiers, FrameDblclickParamsModifiersItem),
            steps,
            platform: None,
        })
    }

    /// Reads `Frame.hover`'s params: one move, waiting for a visible and stable element only.
    pub(crate) fn from_hover(params: FrameHoverParams) -> Self {
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
            selector,
            strict: strict.unwrap_or(false),
            force: force.unwrap_or(false),
            scroll: scroll != Some(FrameHoverParamsScroll::None),
            wait_after: false,
            position: position.as_ref().map(point),
            action: PointerAction::Hover,
            trial: trial.unwrap_or(false),
            modifiers: keyboard_modifiers!(modifiers, FrameHoverParamsModifiersItem),
            // Playwright: `server/input.ts` `Mouse.move`: `const { steps = 1 } = options;`
            steps: 1,
            platform: None,
        }
    }

    /// Reads `Frame.tap`'s params: a touch start and end at the point.
    pub(crate) fn from_tap(params: FrameTapParams) -> Self {
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
            selector,
            strict: strict.unwrap_or(false),
            force: force.unwrap_or(false),
            scroll: scroll != Some(FrameTapParamsScroll::None),
            wait_after: false,
            position: position.as_ref().map(point),
            action: PointerAction::Tap,
            trial: trial.unwrap_or(false),
            modifiers: keyboard_modifiers!(modifiers, FrameTapParamsModifiersItem),
            steps: 1,
            platform: None,
        }
    }
}

/// Playwright's `steps`, default 1: no steps or fewer moves the pointer without an event, as
/// Playwright's loop does; more than [`MAX_STEPS`] is refused before any work.
pub(crate) fn steps_of(steps: Option<i64>) -> Result<u32, PlaywrightError> {
    // Playwright: `server/input.ts` `Mouse.move`: `const { steps = 1 } = options;`
    let steps = u32::try_from(steps.unwrap_or(1).max(0)).unwrap_or(u32::MAX);
    if steps > MAX_STEPS {
        return Err(PlaywrightError::error(format!(
            "Neo does not support steps above {MAX_STEPS}"
        )));
    }
    Ok(steps)
}

/// A click's `clickCount`, default 1: JS's `Math.max(0, n)` loop bound, so zero or fewer clicks
/// write only the move; more than [`MAX_CLICK_COUNT`] is refused before any work.
/// Playwright: `server/input.ts` `Mouse.click`: `const { delay = null, clickCount = 1, steps } = options;`
pub(crate) fn click_count_of(click_count: Option<i64>) -> Result<u32, PlaywrightError> {
    let click_count = u32::try_from(click_count.unwrap_or(1).max(0)).unwrap_or(u32::MAX);
    if click_count > MAX_CLICK_COUNT {
        return Err(PlaywrightError::error(format!(
            "Neo does not support a clickCount above {MAX_CLICK_COUNT}"
        )));
    }
    Ok(click_count)
}

/// `if (delay)`: zero takes the pipelined path; any other number the sequential one, sleeping
/// zero for a negative delay, as `setTimeout` treats it.
/// Playwright: `server/input.ts` `Mouse.click`: `if (delay) {`
pub(crate) fn click_delay(delay: Option<f64>) -> Option<Duration> {
    delay
        .filter(|delay| *delay != 0.0)
        .map(|delay| Duration::try_from_secs_f64(delay.max(0.0) / 1000.0).unwrap_or(Duration::MAX))
}

/// The wire's `Point` and the page call's are two generated types of one shape.
/// Neo: a copy between two generated types, which no script can see.
pub(crate) fn point(position: &WirePoint) -> Point {
    Point {
        x: position.x,
        y: position.y,
    }
}

/// Why one loop-A iteration ended without its action.
#[derive(Debug)]
pub(crate) enum Stop {
    /// A page step answered [`PageError::NotConnected`]: resolve again, as Playwright retries
    /// every error its `isNonRetriableError` does not name.
    Retry,
    /// The call ends with this error.
    End(CallError),
}

impl From<CallError> for Stop {
    fn from(error: CallError) -> Self {
        Self::End(error)
    }
}

impl From<PlaywrightError> for Stop {
    fn from(error: PlaywrightError) -> Self {
        Self::End(error.into())
    }
}

impl From<PageError> for Stop {
    fn from(error: PageError) -> Self {
        match error {
            PageError::NotConnected => Self::Retry,
            PageError::Threw(message) | PageError::Contract(message) => {
                PlaywrightError::error(message).into()
            }
            PageError::Closed => PlaywrightError::target_closed().into(),
        }
    }
}

/// Serves `Frame.click`, `Frame.dblclick`, `Frame.hover` and `Frame.tap` (`frames.ts`: loop A
/// around `dom.ts` `_click`, `_dblclick`, `_hover`, `_tap`). `Ok(())` is Playwright's `{id}`
/// reply; the call log and delivery are in `progress`.
pub(crate) async fn pointer_action<T: KeyboardTab>(
    tab: &T,
    options: &PointerOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    // Every pointer action is an input method in the spec, so dispatch always gives it a cell.
    let Some(delivery) = progress.delivery() else {
        return Err(PlaywrightError::error(format!(
            "neo-runtime: Frame.{} has no delivery cell",
            options.action.name()
        ))
        .into());
    };
    // `modifiers` name `ControlOrMeta` by the browser's platform, read once per call.
    let with_platform;
    let options = if options.modifiers.is_some() {
        let platform = progress
            .race(tab.platform())
            .await?
            .map_err(platform_error)?;
        with_platform = PointerOptions {
            platform: Some(platform),
            ..options.clone()
        };
        &with_platform
    } else {
        options
    };
    retry_with_progress_if_not_connected(
        tab,
        &options.selector,
        options.strict,
        options.force,
        progress,
        &PointerElementAction { options, delivery },
    )
    .await
}

/// A per-element action inside Playwright's loop A. It returns `NotConnected` only when the
/// page proved the element detached; transient page errors use `Stop::Retry` without that log.
/// The resolved handle lives until `act` returns, including across its loop-B retries.
pub(crate) trait ElementAction<T: Tab>: Sync {
    type Output;

    fn act(
        &self,
        tab: &T,
        element: &T::Element,
        progress: &Progress,
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, Stop>> + Send;
}

/// How one per-element action ended for loop A.
pub(crate) enum ActionResult<R> {
    Done(R),
    NotConnected,
}

struct PointerElementAction<'a> {
    options: &'a PointerOptions,
    delivery: &'a Arc<DeliveryCell>,
}

impl<T: Tab> ElementAction<T> for PointerElementAction<'_> {
    type Output = ();

    async fn act(
        &self,
        tab: &T,
        element: &T::Element,
        progress: &Progress,
    ) -> Result<ActionResult<()>, Stop> {
        let wait_after = if self.options.wait_after {
            WaitAfter::Enabled
        } else {
            WaitAfter::Disabled
        };
        retry_pointer_action(
            tab,
            element,
            self.options,
            wait_after,
            progress,
            self.delivery,
        )
        .await
    }
}

/// Playwright's `_retryWithProgressIfNotConnected`, shared by pointer and form actions. Naming,
/// backoff, preparation, pre-checks, description and resolution stay in exactly that order.
/// The element and its object group drop after each iteration, as Playwright disposes its handle.
pub(crate) async fn retry_with_progress_if_not_connected<T: Tab, A: ElementAction<T>>(
    tab: &T,
    selector: &str,
    strict: bool,
    force: bool,
    progress: &Progress,
    action: &A,
) -> Result<A::Output, CallError> {
    // Playwright names the locator before any page call. Neo learns its text from the page, so
    // the line reads the raw selector until `describe` answers (Playwright's own fallback).
    progress.name_locator(selector);
    let mut named = false;
    let mut waits = LoopAWaits::new(progress.timeout_ms());
    loop {
        let wait = waits.next_wait();
        if !wait.is_zero() {
            progress.wait(wait).await?;
        }
        let iteration = async {
            progress.race(tab.prepare()).await??;
            if !force {
                wait_out_pending_navigation(tab, progress).await?;
            }
            if !named {
                let facts = progress.race(tab.describe(selector)).await??;
                progress.name_locator(&facts.locator);
                named = true;
                // Playwright parses the selector before its first page call and fails at once.
                if let Some(message) = facts.invalid {
                    return Err(PlaywrightError::error(message).into());
                }
            }
            let resolved = progress.race(tab.resolve(selector, strict)).await??;
            let Some(resolved) = resolved else {
                return Ok(None);
            };
            progress.log(resolved_line(resolved.count, &resolved.preview));
            match action.act(tab, &resolved.element, progress).await? {
                ActionResult::Done(value) => Ok(Some(value)),
                ActionResult::NotConnected => {
                    progress.log("element was detached from the DOM, retrying");
                    Ok(None)
                }
            }
        };
        match iteration.await {
            Ok(Some(value)) => return Ok(value),
            Ok(None) | Err(Stop::Retry) => {}
            Err(Stop::End(error)) => return Err(error),
        }
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
    options: &PointerOptions,
    wait_after: WaitAfter,
    progress: &Progress,
    delivery: &Arc<DeliveryCell>,
) -> Result<ActionResult<()>, Stop> {
    let trial = if options.trial { " (trial run)" } else { "" };
    let name = options.action.name();
    let mut retry = 0;
    loop {
        if retry == 0 {
            progress.log(format!("attempting {name} action{trial}"));
        } else {
            progress.log(format!("retrying {name} action{trial}"));
            let wait = loop_b_wait(retry);
            if !wait.is_zero() {
                progress.log(format!("  waiting {}ms", wait.as_millis()));
                progress.wait(wait).await?;
            }
        }
        if !options.force {
            wait_out_pending_navigation(tab, progress).await?;
        }
        let reason =
            match attempt(tab, element, retry, options, wait_after, progress, delivery).await? {
                Attempt::Done => return Ok(ActionResult::Done(())),
                Attempt::NotReady(NotReady::NotConnected) => return Ok(ActionResult::NotConnected),
                Attempt::NotReady(reason) => reason,
            };
        retry += 1;
        if options.force {
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
        if let Some(line) = not_ready_line(&reason) {
            progress.log(line);
        }
    }
}

/// One attempt (`_performPointerAction`), retry index `retry`: states, scroll, point, arm, and
/// then [`act`]. The `waiting for …` line goes out before the page call, so a call that times
/// out inside `checkElementStates` ends its log exactly as Playwright's does.
async fn attempt<T: Tab>(
    tab: &T,
    element: &T::Element,
    retry: usize,
    options: &PointerOptions,
    wait_after: WaitAfter,
    progress: &Progress,
    delivery: &Arc<DeliveryCell>,
) -> Result<Attempt, Stop> {
    let action = options.action;
    if !options.force {
        progress.log(format!(
            "  waiting for element to be {}",
            action.states_text()
        ));
        match progress
            .race(tab.check_states(element, action.states(), options.scroll))
            .await??
        {
            StatesCheck::Ready => {}
            StatesCheck::NotConnected => return Ok(Attempt::NotReady(NotReady::NotConnected)),
            StatesCheck::Missing(state) => {
                return Ok(Attempt::NotReady(NotReady::Missing(state)));
            }
        }
        progress.log(format!("  element is {}", action.states_text()));
    }

    progress.log("  scrolling into view if needed");
    if options.scroll {
        let scroll = SCROLLS[retry % SCROLLS.len()];
        let scrolled = progress
            .race(tab.scroll(element, scroll, options.position.as_ref()))
            .await??;
        if let Some(reason) = scrolled {
            return Ok(Attempt::NotReady(reason));
        }
    }
    progress.log("  done scrolling");

    let geometry = progress
        .race(tab.geometry(element, options.position.as_ref(), options.force))
        .await??;
    let point = match click_point(&geometry, options.position.as_ref()) {
        Ok(point) => round_point(&point),
        Err(reason) => return Ok(Attempt::NotReady(reason)),
    };

    if options.force {
        progress.log("  forcing action");
    }
    // The input phase runs from here, after every wait for the element, to the verdict. Inside
    // it the preliminary hit check, the input and the stop all see this action's own
    // interceptor.
    let input = progress.race(tab.input()).await?;
    let armed = if options.force {
        None
    } else {
        match progress
            .race(tab.arm(element, point.clone(), action.hit_target(), input.clone()))
            .await??
        {
            Arming::Armed(armed) => Some(armed),
            Arming::NotConnected => return Ok(Attempt::NotReady(NotReady::NotConnected)),
            Arming::Intercepted(target) => {
                return Ok(Attempt::NotReady(NotReady::Intercepted(target)));
            }
        }
    };
    if options.trial {
        // Deviation: `server/dom.ts` `ElementHandle._performPointerAction`: `await action.perform(progress, point);` Playwright's trial still performs the input into an interceptor that blocks every event it judges, which the page's earlier listeners and the hover state still see; Neo's trial ends at its checks and sends no input.
        // Dropping the interceptor disarms it, and the tab's input comes back once that stop has
        // answered.
        drop(armed);
        progress.log(format!("  trial {} action done", action.name()));
        return Ok(Attempt::Done);
    }
    act(
        tab, point, armed, input, options, wait_after, progress, delivery,
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
    options: &PointerOptions,
    wait_after: WaitAfter,
    progress: &Progress,
    delivery: &Arc<DeliveryCell>,
) -> Result<Attempt, Stop> {
    let name = options.action.name();
    // Planned under the turn, from the tab's devices as they are now: the held modifiers and
    // buttons, and the pointer where the last input left it.
    let plan = plan_pointer_input(&input, &point, options)?;
    let watched = (wait_after == WaitAfter::Enabled).then(|| tab.watch_navigations());
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
        let ticket = if options.action == PointerAction::MoveAndUp {
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
    options: &PointerOptions,
) -> Result<Plan, Stop> {
    let mut simulated = input.snapshot()?;
    let owner = input.owner();
    let mut plan = Plan::default();
    let target = match (&options.modifiers, options.platform) {
        (Some(modifiers), Some(platform)) => {
            Some((keyboard::modifiers_mask(modifiers, platform), platform))
        }
        (Some(_), None) => {
            return Err(PlaywrightError::error(
                "neo-runtime: a pointer action's modifiers came without the platform",
            )
            .into());
        }
        (None, _) => None,
    };
    let changes = target.map(|(mask, platform)| {
        keyboard::plan_ensure_modifiers(&mut plan, &mut simulated, mask, platform, &owner)
    });
    let to = (point.x, point.y);
    match options.action.input() {
        PointerInput::Move => {
            input_devices::plan_free_move(&mut plan, &mut simulated, to, options.steps);
        }
        PointerInput::MoveAndDown => {
            // Playwright: `server/frames.ts` `Frame.dragAndDrop`: `await this._page.mouse.move(progress, point.x, point.y);`
            input_devices::plan_free_move(&mut plan, &mut simulated, to, 1);
            input_devices::plan_down(&mut plan, &mut simulated, MouseButton::Left, 1, &owner);
        }
        PointerInput::MoveAndUp => {
            // Playwright: `server/frames.ts` `Frame.dragAndDrop`: `await this._page.mouse.move(progress, point.x, point.y, { steps: options.steps });`
            input_devices::plan_free_move(&mut plan, &mut simulated, to, options.steps);
            input_devices::plan_up(&mut plan, &mut simulated, MouseButton::Left, 1, None);
        }
        PointerInput::Click(click) => {
            input_devices::plan_click(&mut plan, &mut simulated, to, &click, options.steps, &owner);
        }
        PointerInput::Tap => input_devices::plan_tap(&mut plan, &mut simulated, to),
    }
    if let (Some((_, platform)), Some(changes)) = (target, changes) {
        keyboard::plan_restore_modifiers(&mut plan, &mut simulated, &changes, platform, &owner);
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

/// `performActionPreChecks`' navigation half (`page.ts` `_performWaitForNavigationCheck`):
/// when a requested main-frame navigation has not settled, log, wait for it, and log where it
/// went.
pub(crate) async fn wait_out_pending_navigation<T: Tab>(
    tab: &T,
    progress: &Progress,
) -> Result<(), Stop> {
    let Some(url) = tab.pending_navigation() else {
        return Ok(());
    };
    let to = if url.is_empty() {
        String::new()
    } else {
        format!(" \"{}\"", trim_with_ellipsis(&url, LOGGED_URL_CHARS))
    };
    progress.log(format!("  waiting for{to} navigation to finish..."));
    if let Some(committed) = progress.race(tab.navigation_settled()).await?? {
        progress.log(format!(
            "  navigated to \"{}\"",
            trim_with_ellipsis(&committed, LOGGED_URL_CHARS)
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Pure policy, unit-tested without a browser.
// ---------------------------------------------------------------------------------------------

/// Loop A's waits for a call timeout in ms (`0` = none): `0`, then [`LOOP_A_BACKOFF_MS`] with
/// trailing entries over `timeout / 5` dropped, the last repeating forever. A 500 ms timeout
/// gives 0, 20, 50, 100, 100, 100….
pub(crate) struct LoopAWaits {
    waits_ms: Vec<u32>,
    index: usize,
}

impl LoopAWaits {
    pub(crate) fn new(timeout_ms: f64) -> Self {
        let mut backoff = LOOP_A_BACKOFF_MS.to_vec();
        // Playwright's `if (progress.timeout)`: a negative timeout caps too, dropping every entry.
        if timeout_ms != 0.0 {
            while backoff
                .last()
                .is_some_and(|&wait| f64::from(wait) > timeout_ms / 5.0)
            {
                backoff.pop();
            }
        }
        let mut waits_ms = vec![0];
        waits_ms.extend(backoff);
        Self { waits_ms, index: 0 }
    }

    pub(crate) fn next_wait(&mut self) -> Duration {
        let last = self.waits_ms.len() - 1;
        let wait = self.waits_ms[self.index.min(last)];
        self.index += 1;
        Duration::from_millis(u64::from(wait))
    }
}

/// Loop B's wait before retry `retry` (≥ 1): `LOOP_B_WAITS_MS[min(retry - 1, 4)]`. Retry 1
/// waits 0 ms and logs no wait line.
pub(crate) fn loop_b_wait(retry: usize) -> Duration {
    let index = retry.saturating_sub(1).min(LOOP_B_WAITS_MS.len() - 1);
    Duration::from_millis(LOOP_B_WAITS_MS[index])
}

/// Playwright's point from raw geometry (`dom.ts` `_clickablePoint`, `_offsetPoint`): clip
/// each quad to the viewport, keep quads whose area is over 0.99, take the middle of the first
/// survivor; no quads → `NotVisible`, none left → `NotInViewport`. For `Offset`, the border
/// box's top-left plus the border widths plus `position`.
fn click_point(geometry: &Geometry, position: Option<&Point>) -> Result<Point, NotReady> {
    match geometry {
        Geometry::NoQuads => Err(NotReady::NotVisible),
        Geometry::Quads { quads, .. } if quads.is_empty() => Err(NotReady::NotVisible),
        Geometry::Quads {
            quads,
            viewport: (width, height),
        } => quads
            .iter()
            .map(|quad| {
                let mut clipped = *quad;
                for (index, value) in clipped.iter_mut().enumerate() {
                    let bound = if index % 2 == 0 { *width } else { *height };
                    *value = value.max(0.0).min(bound);
                }
                clipped
            })
            .find(|quad| quad_area(quad) > 0.99)
            .map(|quad| quad_middle(&quad))
            .ok_or(NotReady::NotInViewport),
        Geometry::Offset {
            border_box,
            border: (left, top),
        } => {
            let (offset_x, offset_y) = position.map_or((0.0, 0.0), |point| (point.x, point.y));
            let x = [border_box[0], border_box[2], border_box[4], border_box[6]]
                .into_iter()
                .fold(f64::INFINITY, f64::min);
            let y = [border_box[1], border_box[3], border_box[5], border_box[7]]
                .into_iter()
                .fold(f64::INFINITY, f64::min);
            Ok(Point {
                x: x + left + offset_x,
                y: y + top + offset_y,
            })
        }
    }
}

/// `computeQuadArea`: the shoelace sum over the four corners, in Playwright's order.
fn quad_area(quad: &[f64; 8]) -> f64 {
    let mut area = 0.0;
    for corner in 0..4 {
        let next = (corner + 1) % 4;
        let (x1, y1) = (quad[corner * 2], quad[corner * 2 + 1]);
        let (x2, y2) = (quad[next * 2], quad[next * 2 + 1]);
        area += (x1 * y2 - x2 * y1) / 2.0;
    }
    area.abs()
}

/// `quadMiddlePoint`: each corner's quarter summed in order, so the float result is
/// Playwright's to the last bit.
fn quad_middle(quad: &[f64; 8]) -> Point {
    let mut point = Point { x: 0.0, y: 0.0 };
    for corner in 0..4 {
        point.x += quad[corner * 2] / 4.0;
        point.y += quad[corner * 2 + 1] / 4.0;
    }
    point
}

/// `roundPoint`: truncation to two decimals, `(x * 100 | 0) / 100`.
fn round_point(point: &Point) -> Point {
    Point {
        x: f64::from(to_int32(point.x * 100.0)) / 100.0,
        y: f64::from(to_int32(point.y * 100.0)) / 100.0,
    }
}

/// JS's `ToInt32`, which `| 0` applies: truncate, wrap modulo 2^32, read as signed.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "ToInt32 is defined as exactly this wrap; the value is in [0, 2^32) before the casts"
)]
fn to_int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let wrapped = value.trunc().rem_euclid(4_294_967_296.0);
    (wrapped as u32) as i32
}

/// `  locator resolved to <preview>`, or for a non-strict multi-match
/// `  locator resolved to N elements. Proceeding with the first one: <preview>`.
fn resolved_line(count: u32, preview: &str) -> String {
    if count > 1 {
        format!("  locator resolved to {count} elements. Proceeding with the first one: {preview}")
    } else {
        format!("  locator resolved to {preview}")
    }
}

/// The line loop B logs for `reason`; `None` for `NotConnected`, which leaves loop B.
pub(crate) fn not_ready_line(reason: &NotReady) -> Option<String> {
    match reason {
        NotReady::NotConnected => None,
        NotReady::NotVisible => Some("  element is not visible".to_string()),
        NotReady::NotInViewport => Some("  element is outside of the viewport".to_string()),
        NotReady::Missing(ElementState::NonInert) => Some("  element is inert".to_string()),
        NotReady::Missing(state) => Some(format!("  element is not {}", state_name(*state))),
        NotReady::Intercepted(target) => Some(format!("  {target} intercepts pointer events")),
    }
}

/// An element state as Playwright spells it.
fn state_name(state: ElementState) -> &'static str {
    match state {
        ElementState::Visible => "visible",
        ElementState::Hidden => "hidden",
        ElementState::Enabled => "enabled",
        ElementState::Disabled => "disabled",
        ElementState::Editable => "editable",
        ElementState::Checked => "checked",
        ElementState::Unchecked => "unchecked",
        ElementState::Indeterminate => "indeterminate",
        ElementState::Stable => "stable",
        ElementState::NonInert => "non-inert",
    }
}

/// Playwright's `trimStringWithEllipsis`: unchanged within `cap` UTF-16 units, else the first
/// `cap - 1` code points and `…`.
fn trim_with_ellipsis(input: &str, cap: usize) -> String {
    if input.encode_utf16().count() <= cap {
        return input.to_string();
    }
    if input.chars().count() <= cap {
        return input.to_string();
    }
    let mut trimmed: String = input.chars().take(cap - 1).collect();
    trimmed.push('\u{2026}');
    trimmed
}

#[cfg(test)]
pub(crate) mod tests {
    //! Policy tests against [`FakeTab`], under paused tokio time, so every wait is exact and no
    //! test depends on the machine's speed. They check what a click calls, in which order, and
    //! what it delivers; never the call log's wording or how many attempts fit in a timeout.
    //! `checkElementStates` takes 33 ms when `stable` is checked (two animation frames at 60 Hz,
    //! `injectedScript.ts` `_checkElementIsStable` with `stableRafCount: 1`); every other step
    //! takes 1 ms, a loopback CDP round trip.

    use std::collections::{BTreeSet, VecDeque};
    use std::future;
    use std::sync::Mutex;

    use tokio::sync::oneshot;
    use tokio::time;
    use tokio::time::Instant;

    use browseros_cdp::SessionId;
    use serde_json::Value;

    use super::*;
    use crate::forms::tests::FormSteps;
    use crate::input_devices::{Effect, EffectSink, InputDevices, Owner};
    use crate::progress::ErrorName;
    use crate::run::Delivery;

    const ROUND_TRIP: Duration = Duration::from_millis(1);
    const STABLE: Duration = Duration::from_millis(33);
    const SELECTOR: &str = "internal:role=button[name=\"Submit\"i]";
    const LOCATOR: &str = "getByRole('button', { name: 'Submit' })";
    const PREVIEW: &str = "<button>Submit</button>";
    const OVERLAY: &str = "<div id=\"overlay\"></div>";
    const OK_QUAD: [f64; 8] = [8.0, 8.0, 65.5, 8.0, 65.5, 29.0, 8.0, 29.0];

    /// One scripted step: each call takes `duration` and pops the next answer; the last answer
    /// repeats.
    pub(crate) struct Step<T> {
        pub(crate) duration: Duration,
        answers: Mutex<VecDeque<T>>,
    }

    impl<T: Clone> Step<T> {
        pub(crate) fn new(duration: Duration, answers: impl IntoIterator<Item = T>) -> Self {
            Self {
                duration,
                answers: Mutex::new(answers.into_iter().collect()),
            }
        }

        pub(crate) fn always(duration: Duration, answer: T) -> Self {
            Self::new(duration, [answer])
        }

        pub(crate) fn next(&self) -> T {
            let mut answers = self.answers.lock().expect("a fake step's lock");
            if answers.len() > 1 {
                answers.pop_front().expect("checked non-empty")
            } else {
                answers.front().cloned().expect("a fake step has an answer")
            }
        }

        pub(crate) async fn answer(&self) -> T {
            let answer = self.next();
            time::sleep(self.duration).await;
            answer
        }
    }

    /// The writer's scripted outcome.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Writes {
        Ack,
        Nothing,
        Fail,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub(crate) enum ArmAnswer {
        Armed,
        NotConnected,
        Intercepted(String),
    }

    /// A dropped armed interceptor, so tests see every path disarm. It holds a copy of its
    /// click's input turn, as the real one does until its stop has answered.
    pub(crate) struct FakeArmed {
        calls: Arc<Mutex<Vec<String>>>,
        _input: FakeInput,
    }

    impl Drop for FakeArmed {
        fn drop(&mut self) {
            self.calls
                .lock()
                .expect("calls")
                .push("disarmed".to_string());
        }
    }

    /// A turn at the fake tab's input, shared by its copies like the real one. The tab records
    /// `input` when it grants a turn and `input released` when the turn's last copy drops, so
    /// tests see where each input phase starts and ends.
    #[derive(Clone)]
    pub(crate) struct FakeInput {
        _turn: Arc<FakeTurn>,
        /// The fake tab's devices, which the turn reaches as the real one reaches its tab's.
        devices: Arc<Mutex<InputDevices>>,
    }

    /// The run that holds every fake turn.
    pub(crate) fn fake_owner() -> Owner {
        SessionId("RUN".to_string())
    }

    impl EffectSink for FakeInput {
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

    impl Devices for FakeInput {
        fn snapshot(&self) -> Result<InputDevices, PageError> {
            Ok(self.devices.lock().expect("devices").clone())
        }

        fn owner(&self) -> Owner {
            fake_owner()
        }
    }

    /// What a plan writes, as the fake tab records it: `tap (x, y)`, `press (x, y)`, `press twice
    /// (x, y)` or `move (x, y)`, at the plan's last pointer point.
    pub(crate) fn plan_summary(plan: &Plan) -> String {
        let events: Vec<Value> = plan
            .commands
            .iter()
            .filter(|command| command.method.is_some())
            .map(|command| serde_json::from_str(&command.params).expect("params are JSON"))
            .collect();
        let kind = |event: &Value| event["type"].as_str().unwrap_or_default().to_string();
        if let Some(start) = events.iter().find(|event| kind(event) == "touchStart") {
            let at = &start["touchPoints"][0];
            return format!("tap ({}, {})", at["x"], at["y"]);
        }
        let presses = events
            .iter()
            .filter(|event| kind(event) == "mousePressed")
            .count();
        let verb = match presses {
            0 => "move",
            2 => "press twice",
            _ => "press",
        };
        let last = events
            .iter()
            .rev()
            .find(|event| kind(event).starts_with("mouse"))
            .map_or((0.0, 0.0), |event| {
                (
                    event["x"].as_f64().unwrap_or_default(),
                    event["y"].as_f64().unwrap_or_default(),
                )
            });
        format!("{verb} ({}, {})", last.0, last.1)
    }

    struct FakeTurn(Arc<Mutex<Vec<String>>>);

    impl Drop for FakeTurn {
        fn drop(&mut self) {
            self.0
                .lock()
                .expect("calls")
                .push("input released".to_string());
        }
    }

    /// A scripted tab. Every page step records its name in `calls`.
    pub(crate) struct FakeTab {
        pub(crate) forms: FormSteps,
        pub(crate) platform: Step<Result<Platform, PageError>>,
        /// Instant, like every preparation after the run's first.
        pub(crate) prepare: Step<Result<(), PageError>>,
        pub(crate) describe: Step<Result<SelectorFacts, PageError>>,
        pub(crate) resolve: Step<Result<Option<(u32, String)>, PageError>>,
        pub(crate) check_states: Step<Result<StatesCheck, PageError>>,
        pub(crate) scroll: Step<Result<Option<NotReady>, PageError>>,
        pub(crate) geometry: Step<Result<Geometry, PageError>>,
        pub(crate) arm: Step<Result<ArmAnswer, PageError>>,
        pub(crate) press: Step<Writes>,
        pub(crate) verdict: Step<Verdict>,
        pub(crate) settle: Step<Result<Vec<String>, PageError>>,
        pub(crate) pending: Mutex<Option<String>>,
        pub(crate) calls: Arc<Mutex<Vec<String>>>,
        /// The states each `check_states` asked for, in call order.
        pub(crate) states: Mutex<Vec<Vec<ElementState>>>,
        /// The tab's devices, shared with every turn it grants.
        pub(crate) devices: Arc<Mutex<InputDevices>>,
        /// Every plan `press` was given, in call order.
        pub(crate) plans: Mutex<Vec<Plan>>,
    }

    impl FakeTab {
        /// The `ok` page: one button that every step finds ready.
        pub(crate) fn ready() -> Self {
            Self {
                forms: FormSteps::ready(),
                platform: Step::always(ROUND_TRIP, Ok(Platform::Mac)),
                prepare: Step::always(Duration::ZERO, Ok(())),
                describe: Step::always(
                    ROUND_TRIP,
                    Ok(SelectorFacts {
                        locator: LOCATOR.to_string(),
                        invalid: None,
                    }),
                ),
                resolve: Step::always(ROUND_TRIP, Ok(Some((1, PREVIEW.to_string())))),
                check_states: Step::always(STABLE, Ok(StatesCheck::Ready)),
                scroll: Step::always(ROUND_TRIP, Ok(None)),
                geometry: Step::always(
                    ROUND_TRIP,
                    Ok(Geometry::Quads {
                        quads: vec![OK_QUAD],
                        viewport: (516.0, 475.0),
                    }),
                ),
                arm: Step::always(ROUND_TRIP, Ok(ArmAnswer::Armed)),
                press: Step::always(ROUND_TRIP, Writes::Ack),
                verdict: Step::always(ROUND_TRIP, Verdict::Done),
                settle: Step::always(ROUND_TRIP, Ok(Vec::new())),
                pending: Mutex::new(None),
                calls: Arc::default(),
                states: Mutex::default(),
                devices: Arc::default(),
                plans: Mutex::default(),
            }
        }

        pub(crate) fn record(&self, call: impl Into<String>) {
            self.calls.lock().expect("calls").push(call.into());
        }

        pub(crate) fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("calls").clone()
        }

        pub(crate) fn count(&self, name: &str) -> usize {
            self.calls().iter().filter(|call| *call == name).count()
        }
    }

    impl Tab for FakeTab {
        type Element = u32;
        type Armed = FakeArmed;
        type Input = FakeInput;
        type Navigations = ();

        async fn prepare(&self) -> Result<(), PageError> {
            self.prepare.answer().await
        }

        async fn describe(&self, _selector: &str) -> Result<SelectorFacts, PageError> {
            self.record("describe");
            self.describe.answer().await
        }

        async fn resolve(
            &self,
            _selector: &str,
            _strict: bool,
        ) -> Result<Option<Resolved<u32>>, PageError> {
            self.record("resolve");
            let answer = self.resolve.answer().await?;
            Ok(answer.map(|(count, preview)| Resolved {
                element: 1,
                count,
                preview,
            }))
        }

        async fn check_states(
            &self,
            _element: &u32,
            states: &[ElementState],
            scroll: bool,
        ) -> Result<StatesCheck, PageError> {
            self.states.lock().expect("states").push(states.to_vec());
            self.record(if scroll {
                "check_states"
            } else {
                "check_states without scrolling"
            });
            self.check_states.answer().await
        }

        async fn scroll(
            &self,
            _element: &u32,
            scroll: Scroll,
            _position: Option<&Point>,
        ) -> Result<Option<NotReady>, PageError> {
            self.record(format!("scroll {scroll:?}"));
            self.scroll.answer().await
        }

        async fn geometry(
            &self,
            _element: &u32,
            _position: Option<&Point>,
            _force: bool,
        ) -> Result<Geometry, PageError> {
            self.record("geometry");
            self.geometry.answer().await
        }

        /// Forms can script a blocked input wait; click tests grant their turns at once.
        async fn input(&self) -> FakeInput {
            if !self.forms.input_wait.is_zero() {
                time::sleep(self.forms.input_wait).await;
            }
            self.record("input");
            FakeInput {
                _turn: Arc::new(FakeTurn(Arc::clone(&self.calls))),
                devices: Arc::clone(&self.devices),
            }
        }

        async fn arm(
            &self,
            _element: &u32,
            hit_point: Point,
            action: HitTargetAction,
            input: FakeInput,
        ) -> Result<Arming<FakeArmed>, PageError> {
            self.record(format!("arm {action:?} {hit_point:?}"));
            Ok(match self.arm.answer().await? {
                ArmAnswer::Armed => Arming::Armed(FakeArmed {
                    calls: Arc::clone(&self.calls),
                    _input: input,
                }),
                ArmAnswer::NotConnected => Arming::NotConnected,
                ArmAnswer::Intercepted(target) => Arming::Intercepted(target),
            })
        }

        fn press(
            &self,
            plan: Plan,
            ticket: InputTicket,
            armed: Option<FakeArmed>,
            input: FakeInput,
        ) -> impl Future<Output = (PressReport, Option<FakeArmed>)> + Send {
            self.record(plan_summary(&plan));
            self.plans.lock().expect("plans").push(plan.clone());
            let writes = self.press.next();
            let duration = self.press.duration;
            let calls = Arc::clone(&self.calls);
            let (report, receiver) = oneshot::channel();
            // Detached, like the real writer: it marks the ticket whether or not anyone waits,
            // and holds the interceptor until its last event is answered.
            tokio::spawn(async move {
                time::sleep(duration).await;
                calls.lock().expect("calls").push("released".to_string());
                let outcome = match writes {
                    Writes::Ack => {
                        // A perfect writer: every command written and answered.
                        for command in &plan.commands {
                            input.apply(&command.effect, true);
                        }
                        PressReport::Acked(ticket.acked())
                    }
                    Writes::Nothing => {
                        ticket.unsent();
                        PressReport::NothingWritten
                    }
                    Writes::Fail => {
                        drop(ticket);
                        PressReport::Failed {
                            closed: false,
                            message: "Input.dispatchMouseEvent timed out".to_string(),
                        }
                    }
                };
                // Done writing: the writer's copy of the turn goes, as the real writer's does.
                drop(input);
                let _waiting = report.send((outcome, armed));
            });
            async move {
                receiver.await.unwrap_or_else(|_| {
                    let failed = PressReport::Failed {
                        closed: false,
                        message: "the fake writer ended".to_string(),
                    };
                    (failed, None)
                })
            }
        }

        async fn verdict(&self, armed: FakeArmed) -> Verdict {
            self.record("verdict");
            let verdict = self.verdict.answer().await;
            drop(armed);
            verdict
        }

        fn pending_navigation(&self) -> Option<String> {
            self.pending.lock().expect("pending").clone()
        }

        async fn navigation_settled(&self) -> Result<Option<String>, PageError> {
            self.record("navigation_settled");
            time::sleep(ROUND_TRIP).await;
            Ok(self.pending.lock().expect("pending").take())
        }

        fn watch_navigations(&self) {
            self.record("watch_navigations");
        }

        async fn settle_after_input(&self, (): ()) -> Result<Vec<String>, PageError> {
            self.record("settle");
            self.settle.answer().await
        }
    }

    fn options(selector: &str) -> PointerOptions {
        PointerOptions::from_click(
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
        options: &PointerOptions,
        timeout_ms: f64,
    ) -> (Result<(), CallError>, Vec<String>, Delivery) {
        let progress = Progress::for_test(timeout_ms);
        let result = pointer_action(tab, options, &progress).await;
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
        forced.force = true;
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
        no_wait.wait_after = false;
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
        trial.trial = true;
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
        forced.trial = true;
        forced.force = true;
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
        let result = pointer_action(&tab, &options(SELECTOR), &progress).await;
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
        forced.force = true;
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
            &options(SELECTOR),
            WaitAfter::Disabled,
            &progress,
            progress.delivery().expect("input"),
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

    struct ReadAction(Mutex<usize>);

    impl ElementAction<FakeTab> for ReadAction {
        type Output = String;
        fn act(
            &self,
            tab: &FakeTab,
            _element: &u32,
            _progress: &Progress,
        ) -> impl Future<Output = Result<ActionResult<String>, Stop>> + Send {
            tab.record("read action");
            let mut attempts = self.0.lock().expect("attempts");
            *attempts += 1;
            future::ready(if *attempts == 1 {
                Ok(ActionResult::NotConnected)
            } else {
                Ok(ActionResult::Done("value".into()))
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_loop_a_returns_an_action_value_and_re_resolves_detached_elements() {
        let tab = FakeTab::ready();
        let progress = Progress::for_test(30_000.0);
        let result = retry_with_progress_if_not_connected(
            &tab,
            SELECTOR,
            true,
            false,
            &progress,
            &ReadAction(Mutex::new(0)),
        )
        .await;
        assert_eq!(result, Ok("value".into()));
        assert_eq!(tab.count("describe"), 1);
        assert_eq!(tab.count("resolve"), 2);
        assert_eq!(tab.count("read action"), 2);
        assert!(
            progress
                .folded_log()
                .contains(&"  - element was detached from the DOM, retrying".to_string())
        );
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::NotSent
        );
    }

    #[test]
    fn options_refuse_what_neo_does_not_serve() {
        let decode = |params: serde_json::Value| {
            PointerOptions::from_click(serde_json::from_value(params).expect("click params"))
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
            served.action,
            PointerAction::Click(MouseClick {
                button: MouseButton::Right,
                click_count: 2,
                delay: Some(Duration::ZERO),
            }),
            "a negative delay is truthy in JS and sleeps 0"
        );
        assert!(!served.wait_after && !served.scroll && !served.strict);
        let default = decode(serde_json::json!({ "selector": "a", "delay": 0 })).expect("served");
        assert_eq!(
            default.action.input(),
            PointerInput::Click(MouseClick {
                button: MouseButton::Left,
                click_count: 1,
                delay: None,
            }),
            "a zero delay takes the pipelined path"
        );
    }

    fn decoded(method: &str, params: serde_json::Value) -> Result<PointerOptions, PlaywrightError> {
        match method {
            "dblclick" => {
                PointerOptions::from_dblclick(serde_json::from_value(params).expect("params"))
            }
            "hover" => Ok(PointerOptions::from_hover(
                serde_json::from_value(params).expect("params"),
            )),
            "tap" => Ok(PointerOptions::from_tap(
                serde_json::from_value(params).expect("params"),
            )),
            _ => PointerOptions::from_click(serde_json::from_value(params).expect("params")),
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
                options.modifiers,
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
                empty.modifiers,
                Some(Vec::new()),
                "{method}: [] releases them all"
            );
            let kept = decoded(method, serde_json::json!({ "selector": "a" })).expect("served");
            assert_eq!(
                kept.modifiers, None,
                "{method}: no option keeps the keyboard's"
            );
        }
        for method in ["click", "dblclick"] {
            let options = decoded(method, serde_json::json!({ "selector": "a", "steps": 5 }))
                .expect("served");
            assert_eq!(options.steps, 5, "{method}");
            let none = decoded(method, serde_json::json!({ "selector": "a", "steps": -3 }))
                .expect("served");
            assert_eq!(
                none.steps, 0,
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
        assert_eq!(
            dblclick.action.input(),
            PointerInput::Click(MouseClick {
                button: MouseButton::Middle,
                click_count: 2,
                delay: Some(Duration::from_millis(10)),
            })
        );
        assert!(!dblclick.wait_after && !dblclick.strict);
    }

    /// Each planned event of `plan`: `keyUp Shift` for a key, `mousePressed modifiers=8` for a
    /// mouse event.
    fn plan_events(plan: &Plan) -> Vec<String> {
        plan.commands
            .iter()
            .filter(|command| command.method.is_some())
            .map(|command| {
                let event: Value = serde_json::from_str(&command.params).expect("params are JSON");
                let kind = event["type"].as_str().unwrap_or_default();
                if command.method == Some("Input.dispatchKeyEvent") {
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
                let event: Value = serde_json::from_str(&command.params).ok()?;
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
            devices.keyboard.modifiers = keyboard::SHIFT;
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
        assert_eq!(keyboard.modifiers, keyboard::SHIFT, "Shift is held again");
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

    #[test]
    fn the_click_point_is_the_middle_of_the_visible_part() {
        let quads = |quads: Vec<[f64; 8]>| Geometry::Quads {
            quads,
            viewport: (516.0, 475.0),
        };
        assert_eq!(
            click_point(&quads(vec![OK_QUAD]), None),
            Ok(Point { x: 36.75, y: 18.5 }),
            "the middle of a quad inside the viewport"
        );
        // Half the button hangs off the left edge: the clipped quad's middle.
        assert_eq!(
            click_point(
                &quads(vec![[-20.0, 8.0, 20.0, 8.0, 20.0, 28.0, -20.0, 28.0]]),
                None
            ),
            Ok(Point { x: 10.0, y: 18.0 })
        );
        assert_eq!(
            click_point(
                &quads(vec![[600.0, 8.0, 700.0, 8.0, 700.0, 28.0, 600.0, 28.0]]),
                None
            ),
            Err(NotReady::NotInViewport)
        );
        assert_eq!(
            click_point(&quads(vec![[8.0, 8.0, 9.0, 8.0, 9.0, 8.5, 8.0, 8.5]]), None),
            Err(NotReady::NotInViewport),
            "a 0.5 px² quad is under Playwright's 0.99 floor"
        );
        assert_eq!(
            click_point(&quads(Vec::new()), None),
            Err(NotReady::NotVisible)
        );
        assert_eq!(
            click_point(&Geometry::NoQuads, None),
            Err(NotReady::NotVisible)
        );
        assert_eq!(
            click_point(
                &Geometry::Offset {
                    border_box: OK_QUAD,
                    border: (2.0, 3.0),
                },
                Some(&Point { x: 5.0, y: 6.0 })
            ),
            Ok(Point { x: 15.0, y: 17.0 })
        );
    }
}
