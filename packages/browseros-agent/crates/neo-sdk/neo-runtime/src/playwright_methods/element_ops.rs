//! Neo: Playwright's element operations, served without `ElementHandle`s. `Frame.focus`,
//! `Frame.blur` and `Frame.dispatchEvent` are Playwright's own commands; `scrollIntoViewIfNeeded`,
//! `selectText` and `boundingBox` are Neo commands on the frame (`neo:`), because Playwright's
//! client reaches them only through a handle (its `Locator._withElement`), which Neo does not
//! have yet. Rust keeps each whole wait, as for `neo:waitForURL`.
//!
//! ```text
//!  focus / blur           loop A (`retry::retry_with_progress_if_not_connected`)
//!    resolve → the tab's input turn → focusElement / blurElement glue
//!      detached → resolve again
//!  dispatchEvent          eval's main-world element callback (`evaluate::call_on_element_main`)
//!    eventInit as neo-quickjs-injected's `serializeArgument` left it, handles included; polls until the
//!    selector matches, then `InjectedScript.dispatchEvent` on the first match, in the page's
//!    own world, as Playwright's `mainWorld: true`
//!    a selector with an `aria-ref` (Deviation): loop A resolves it in the utility world, where
//!    Neo's snapshots keep their refs → adopted into its frame's main world as a handle of the
//!    run → the same callback on that handle with `:scope` (`ElementHandle.dispatchEvent`) →
//!    the handle released
//!  scrollIntoViewIfNeeded / selectText / boundingBox
//!    wait until the selector matches (strict) → one element, then on that element only:
//!      scroll   retry_action: stable → DOM.scrollIntoViewIfNeeded; not visible → retry
//!      select   retry_action: the input turn → selectText glue (visible unless force)
//!      box      DOM.getBoxModel's border quad, moved by the frame's offset, or null
//!    detached during the action → Error, never a second resolve
//! ```
//!
//! None of these writes `Input.*`, so none has a delivery cell; focus, blur and selectText still
//! take the tab's input turn, so they never land between another call's focus and its keys.

use browseros_cdp::CdpError;
use std::future::Future;

use neo_playwright_protocol::injected::{
    ElementState, InjectedBlurElementParams, InjectedSelectTextParams,
    InjectedSelectorElementParams, SelectTextOutcome, SelectTextOutcomeStatus,
};
use neo_playwright_protocol::serve::{FrameBlurParams, FrameFocusParams};
use neo_playwright_protocol::{MethodFlag, MethodMeta};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::oneshot;

use crate::browser_representations::page::{FrameTab, GroupGuard, InputPermit, ResolvedElement};
use crate::chromium_bridge::cdp::{self, Command};
use crate::chromium_bridge::frame_selector;

use crate::playwright_methods::evaluate::{
    self, CreatedHandle, EvalCall, MainCallback, SerializedArgument,
};
use crate::playwright_methods::forms::{FocusOutcome, FormTab};
use crate::playwright_methods::reads::SelectorTab;
use crate::playwright_methods::retry::{self, ActionResult, ElementAction, PreChecks, RetryAction};
use crate::playwright_methods::tab::{
    NotReady, PageError, Resolved, Scroll, StatesCheck, Stop, Tab,
};
use crate::progress::{CallError, PlaywrightError, Progress};

// ---------------------------------------------------------------------------------------------
// The Neo commands.
// ---------------------------------------------------------------------------------------------

/// Neo: the trace record of `neo:scrollIntoViewIfNeeded`, titled as Playwright's
/// `ElementHandle.scrollIntoViewIfNeeded`.
pub(crate) static SCROLL_INTO_VIEW_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "neo:scrollIntoViewIfNeeded",
    title: Some("Scroll into view"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot],
};

/// Neo: the trace record of `neo:selectText`, titled as Playwright's `ElementHandle.selectText`.
pub(crate) static SELECT_TEXT_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "neo:selectText",
    title: Some("Select text"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot],
};

/// Neo: the trace record of `neo:boundingBox`, titled as Playwright's `ElementHandle.boundingBox`.
pub(crate) static BOUNDING_BOX_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "neo:boundingBox",
    title: Some("Get bounding box"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[],
};

/// Neo: the params of `neo:scrollIntoViewIfNeeded` and `neo:boundingBox`, kept in step with
/// neo-quickjs-injected's `NeoFrameCommands` and the wire tests. The timeout is in the call's
/// metadata.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NeoElementParams {
    pub(crate) selector: String,
    pub(crate) strict: Option<bool>,
}

/// Neo: the params of `neo:selectText`, `force` as Playwright's `ElementHandle.selectText` takes
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NeoSelectTextParams {
    pub(crate) selector: String,
    pub(crate) strict: Option<bool>,
    pub(crate) force: Option<bool>,
}

/// Neo: what `neo:boundingBox` answers, as Playwright's `ElementHandleBoundingBoxResult`: no
/// `value` when the element has no box.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct BoundingBoxResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) value: Option<Rect>,
}

/// Neo: a border box in the main frame's CSS pixels, Playwright's `Rect`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct Rect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

// ---------------------------------------------------------------------------------------------
// The seam.
// ---------------------------------------------------------------------------------------------

/// Neo: the page steps of the element operations that click's and the forms' seams do not have.
/// Mutating steps start a detached page task and hold the input turn until the page answers,
/// as the forms' steps do; every returned future is cancel-safe.
pub(crate) trait ElementOpsTab: FormTab + SelectorTab {
    /// The `blurElement` glue on one resolved element.
    fn blur(
        &self,
        element: &Self::Element,
        input: Self::Input,
    ) -> impl Future<Output = Result<FocusOutcome, PageError>> + Send;

    /// The `selectText` glue on one resolved element: visible unless `force`.
    fn select_text(
        &self,
        element: &Self::Element,
        force: bool,
        input: Self::Input,
    ) -> impl Future<Output = Result<SelectTextOutcome, PageError>> + Send;

    /// `DOM.getBoxModel`'s border box in the main frame's CSS pixels, `None` when the element has
    /// no box (it is not rendered).
    fn bounding_box(
        &self,
        element: &Self::Element,
    ) -> impl Future<Output = Result<Option<Rect>, PageError>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Focus and blur.
// ---------------------------------------------------------------------------------------------

/// Neo: one `Frame.focus`, `Frame.blur` or Neo element command's target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ElementTarget {
    pub(crate) selector: String,
    /// Locators send `strict: true`; page and frame calls default to non-strict.
    pub(crate) strict: bool,
}

/// Neo: decodes each command's `{selector, strict?}`.
impl ElementTarget {
    pub(crate) fn from_focus(params: FrameFocusParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
        }
    }

    pub(crate) fn from_blur(params: FrameBlurParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
        }
    }

    /// `locator.boundingBox` and `scrollIntoViewIfNeeded` always send `strict: true`.
    pub(crate) fn from_neo(params: NeoElementParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(true),
        }
    }
}

/// Serves `Frame.focus`: loop A, then focus the element.
///
/// Playwright: `server/frames.ts` `Frame.focus`: `(progress, handle) => handle._focus(progress)`
pub(crate) async fn focus<T: ElementOpsTab>(
    tab: &T,
    target: &ElementTarget,
    progress: &Progress,
) -> Result<(), CallError> {
    retry::retry_with_progress_if_not_connected(
        tab,
        &target.selector,
        target.strict,
        PreChecks::Run,
        progress,
        &Focus::Focus,
    )
    .await
}

/// Serves `Frame.blur`: loop A, then blur the element.
///
/// Playwright: `server/frames.ts` `Frame.blur`: `(progress, handle) => handle._blur(progress)`
pub(crate) async fn blur<T: ElementOpsTab>(
    tab: &T,
    target: &ElementTarget,
    progress: &Progress,
) -> Result<(), CallError> {
    retry::retry_with_progress_if_not_connected(
        tab,
        &target.selector,
        target.strict,
        PreChecks::Run,
        progress,
        &Focus::Blur,
    )
    .await
}

/// Neo: which of the two focus steps loop A runs on its element.
enum Focus {
    Focus,
    Blur,
}

/// Playwright: `server/dom.ts` `ElementHandle._focus`: `injected.focusNode(node, resetSelectionIfNotFocused)`
impl<T: ElementOpsTab> ElementAction<T> for Focus {
    type Output = ();

    async fn act(
        &self,
        tab: &T,
        element: &T::Element,
        progress: &Progress,
    ) -> Result<ActionResult<()>, Stop> {
        // The turn is taken after the element waits, and dropped with the answer.
        let input = progress.race(tab.input()).await?;
        let outcome = match self {
            Self::Focus => progress.race(tab.focus(element, false, input)).await??,
            Self::Blur => progress.race(tab.blur(element, input)).await??,
        };
        Ok(match outcome {
            FocusOutcome::Done => ActionResult::Done(()),
            FocusOutcome::ErrorNotconnected => ActionResult::NotConnected,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// dispatchEvent.
// ---------------------------------------------------------------------------------------------

/// Neo: `Frame.dispatchEvent`'s params as neo-quickjs-injected sends them: Playwright's, with
/// `eventInit` already in the page's format and its handles named by guid
/// (`neo-quickjs-injected/src/js-handle.ts` `serializeArgument`). The generated
/// `FrameDispatchEventParams` types `eventInit` in the protocol's format, so the frame route
/// decodes these instead (`method_router.rs` `route_frame_extension`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DispatchEventParams {
    selector: String,
    #[serde(default)]
    strict: Option<bool>,
    r#type: String,
    event_init: SerializedArgument,
}

/// Neo: one `Frame.dispatchEvent`.
#[derive(Debug)]
pub(crate) struct DispatchOptions {
    pub(crate) selector: String,
    pub(crate) strict: bool,
    pub(crate) event_type: String,
    /// The event's init dictionary as Playwright's server hands it to the page.
    pub(crate) event_init: SerializedArgument,
}

impl From<DispatchEventParams> for DispatchOptions {
    fn from(params: DispatchEventParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
            event_type: params.r#type,
            event_init: params.event_init,
        }
    }
}

/// Playwright: `server/frames.ts` `Frame.dispatchEvent`: `}, { type, eventInit }, { mainWorld: true });`
///
/// Serves `Frame.dispatchEvent`: waits until the selector matches, then dispatches once on the
/// first match, built and sent in the page's own world by eval's element callback, with any
/// handles `eventInit` holds. A selector with an `aria-ref` is resolved in the utility world
/// first ([`dispatch_on_aria_ref`]).
pub(crate) async fn dispatch_event(
    page: &FrameTab,
    options: &DispatchOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    if frame_selector::has_aria_ref(&options.selector) {
        return dispatch_on_aria_ref(page, options, progress).await;
    }
    evaluate::call_on_element_main(
        page,
        None,
        &options.selector,
        options.strict,
        MainCallback::DispatchEvent,
        &[json!(options.event_type)],
        &options.event_init,
        progress.handles(),
        progress,
    )
    .await
}

/// Deviation: `server/frames.ts` `Frame.dispatchEvent`: `}, { type, eventInit }, { mainWorld: true });` Playwright resolves the selector in the page's own world, whose injected script holds no `ai` snapshot, so a selector with an `aria-ref` matches nothing there and the call waits out its timeout. Neo resolves such a selector in the utility world, as Playwright's `click` and `locator.evaluate` resolve it, adopts the match into its frame's main world and dispatches there as `ElementHandle.dispatchEvent` does, so the event reaches the element the ref names, in a hidden frame too.
///
/// Loop A until the first match resolves there (strict when asked); then eval's element callback
/// on the run's handle of that element, with `:scope` (Neo's `ElementHandle.dispatchEvent`),
/// and the handle is released. The element's main-world object lives in an object group of this
/// call from its adoption on, so success, error, timeout and abort all release it.
async fn dispatch_on_aria_ref(
    page: &FrameTab,
    options: &DispatchOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    // Playwright names the locator before any page call; Neo learns its text from the page.
    progress.name_locator(&options.selector);
    let mut locator = None;
    let mut waits = retry::LoopAWaits::new(progress.timeout_ms());
    let (element, owner) = loop {
        let wait = waits.next_wait();
        if !wait.is_zero() {
            progress.wait(wait).await?;
        }
        match ref_attempt(page, options, progress, &mut locator).await {
            Ok(Some(element)) => break element,
            Ok(None) | Err(Stop::Retry) => {}
            Err(Stop::End(error)) => return Err(error),
        }
    };
    let dispatched = evaluate::serve(
        page,
        EvalCall::DispatchEvent {
            guid: element.guid.clone(),
            kind: options.event_type.clone(),
            event_init: options.event_init.clone(),
        },
        progress,
    )
    .await;
    // The callback named its own `:scope` locator; the call's log names the agent's.
    if let Some(locator) = &locator {
        progress.name_locator(locator);
    }
    // The handle was this call's own: released as `JSHandle.dispose` releases one, then its group.
    let _released = evaluate::serve(page, EvalCall::Dispose { guid: element.guid }, progress).await;
    drop(owner);
    dispatched.map(|_| ())
}

/// Neo: one attempt of [`dispatch_on_aria_ref`]: the selector's first match in the utility world,
/// unmarked, as `Frame.querySelector` finds it, adopted into its frame's main world in a group of
/// this call, whose guard comes back with the run's handle of it; `None` to poll again.
async fn ref_attempt(
    page: &FrameTab,
    options: &DispatchOptions,
    progress: &Progress,
    locator: &mut Option<String>,
) -> Result<Option<(CreatedHandle, GroupGuard)>, Stop> {
    progress.race(Tab::prepare(page)).await??;
    if locator.is_none() {
        let facts = progress
            .race(Tab::describe(page, &options.selector))
            .await??;
        progress.name_locator(&facts.locator);
        *locator = Some(facts.locator);
        // Playwright parses the selector before its first page call and fails at once.
        if let Some(message) = facts.invalid {
            return Err(PlaywrightError::error(message).into());
        }
    }
    let Some(located) = progress
        .race(frame_selector::locate(
            page,
            &options.selector,
            options.strict,
        ))
        .await??
    else {
        return Ok(None);
    };
    let group = page.group(&located.frame, "dispatch-ref");
    let found = progress
        .race(located.frame.call(
            Some(group.group()),
            &InjectedSelectorElementParams {
                selector: located.selector,
                strict: options.strict,
            },
        ))
        .await??;
    let (Some(element), Some(preview)) = (found.element, found.preview) else {
        return Ok(None);
    };
    // Chromium allocates the object in this call's group: a timeout or abort that wins before the
    // reply drops the guard, which releases the group, so no adoption outlives the call.
    let owner = page.group(&located.frame, "dispatch-ref-main");
    let (document, object) = progress
        .race(
            located
                .frame
                .main
                .adopt(&located.frame, &element, owner.group()),
        )
        .await??;
    let created = evaluate::adopted_element_handle(
        page,
        &located.frame,
        document,
        object,
        &preview,
        progress.handles(),
    )?;
    Ok(Some((created, owner)))
}

// ---------------------------------------------------------------------------------------------
// The element commands: one element, then one action on it.
// ---------------------------------------------------------------------------------------------

/// Serves `neo:scrollIntoViewIfNeeded`: the element, then stable and scrolled into view.
///
/// Playwright: `server/dom.ts` `ElementHandle._waitAndScrollIntoViewIfNeeded`: `return await this._scrollRectIntoViewIfNeeded(progress);`
pub(crate) async fn scroll_into_view_if_needed<T: ElementOpsTab>(
    tab: &T,
    target: &ElementTarget,
    progress: &Progress,
) -> Result<(), CallError> {
    let resolved = attached(tab, target, progress).await?;
    let element = &resolved.element;
    retry_action(tab, progress, "scroll into view", false, || async {
        progress.log("  waiting for element to be stable");
        let checked = progress
            .race(tab.check_states(element, &[ElementState::Stable], true))
            .await?;
        match checked {
            Ok(StatesCheck::Ready) => {}
            Ok(StatesCheck::NotConnected) => return Ok(Attempt::NotConnected),
            Ok(StatesCheck::Missing(state)) => return Ok(Attempt::Missing(state)),
            Err(error) => return page_attempt(error),
        }
        let scrolled = progress
            .race(tab.scroll(element, Scroll::Protocol, None))
            .await?;
        match scrolled {
            Ok(None) => Ok(Attempt::Done),
            Ok(Some(NotReady::NotConnected)) => Ok(Attempt::NotConnected),
            Ok(Some(NotReady::Missing(state))) => Ok(Attempt::Missing(state)),
            Ok(Some(_)) => Ok(Attempt::NotVisible),
            Err(error) => page_attempt(error),
        }
    })
    .await
}

/// Serves `neo:selectText`: the element, then its text selected, visible unless `force`.
///
/// Playwright: `server/dom.ts` `ElementHandle.selectText`: `progress.log('  waiting for element to be visible');`
pub(crate) async fn select_text<T: ElementOpsTab>(
    tab: &T,
    target: &ElementTarget,
    force: bool,
    progress: &Progress,
) -> Result<(), CallError> {
    let resolved = attached(tab, target, progress).await?;
    let element = &resolved.element;
    retry_action(tab, progress, "selectText", force, || async {
        if !force {
            progress.log("  waiting for element to be visible");
        }
        let input = progress.race(tab.input()).await?;
        match progress
            .race(tab.select_text(element, force, input))
            .await?
        {
            Ok(SelectTextOutcome::Status(SelectTextOutcomeStatus::Done)) => Ok(Attempt::Done),
            Ok(SelectTextOutcome::Status(SelectTextOutcomeStatus::ErrorNotconnected)) => {
                Ok(Attempt::NotConnected)
            }
            Ok(SelectTextOutcome::Missing(missing)) => Ok(Attempt::Missing(missing.missing_state)),
            Err(error) => page_attempt(error),
        }
    })
    .await
}

/// Serves `neo:boundingBox`: the element's border box, or `None` without one.
///
/// Playwright: `client/locator.ts` `Locator.boundingBox`: `return await this._withElement(h => h.boundingBox(), { timeout: options?.timeout, signal: options?.signal });`
pub(crate) async fn bounding_box<T: ElementOpsTab>(
    tab: &T,
    target: &ElementTarget,
    progress: &Progress,
) -> Result<BoundingBoxResult, CallError> {
    let resolved = attached(tab, target, progress).await?;
    let value = progress
        .race(tab.bounding_box(&resolved.element))
        .await?
        .map_err(Stop::from);
    match value {
        Ok(value) => Ok(BoundingBoxResult { value }),
        // The element went away between its match and its box: Playwright's handle has no box.
        Err(Stop::Retry) => Ok(BoundingBoxResult { value: None }),
        Err(Stop::End(error)) => Err(error),
    }
}

/// The element a handle method runs on: the selector's match once it is attached, waited for
/// with loop A's backoff, each attempt first running the page's action pre-checks (a pending
/// navigation, the locator handlers), as Playwright's served `waitForSelector` does. Its object
/// group lives as long as the returned value.
///
/// Playwright: `server/frames.ts` `Frame.waitForSelector`: `await this._page.performActionPreChecks(progress);`
///
/// Playwright: `client/locator.ts` `Locator._withElement`: `const result = await this._frame._channel.waitForSelector({ selector: this._selector, strict: true, state: 'attached' }, { signal: options.signal, timeout });`
/// Playwright: `server/frames.ts` `Frame._retryWithProgressIfNotConnected`: `markTargets: 'all'`
async fn attached<T: ElementOpsTab>(
    tab: &T,
    target: &ElementTarget,
    progress: &Progress,
) -> Result<Resolved<T::Element>, CallError> {
    progress.name_locator(&target.selector);
    let mut named = false;
    let mut waits = retry::LoopAWaits::new(progress.timeout_ms());
    loop {
        let wait = waits.next_wait();
        if !wait.is_zero() {
            progress.wait(wait).await?;
        }
        let attempt: Result<Option<Resolved<T::Element>>, Stop> = async {
            progress.race(Tab::prepare(tab)).await??;
            retry::wait_out_pending_navigation(tab, progress).await?;
            if !named {
                let facts = progress
                    .race(Tab::describe(tab, &target.selector))
                    .await??;
                progress.name_locator(&facts.locator);
                named = true;
                if let Some(message) = facts.invalid {
                    return Err(PlaywrightError::error(message).into());
                }
            }
            Ok(progress
                .race(tab.resolve(&target.selector, target.strict))
                .await??)
        }
        .await;
        match attempt {
            Ok(Some(resolved)) => {
                progress.log(format!("  locator resolved to {}", resolved.preview));
                return Ok(resolved);
            }
            Ok(None) | Err(Stop::Retry) => {}
            Err(Stop::End(error)) => return Err(error),
        }
    }
}

/// Neo: how one attempt of a handle action ended, Playwright's `PerformActionResult` reduced to
/// what scroll and selectText answer.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Attempt {
    Done,
    NotConnected,
    NotVisible,
    Missing(ElementState),
}

/// Playwright: `server/dom.ts` `throwElementIsNotAttached`: `'Element is not attached to the DOM'`
const ELEMENT_NOT_ATTACHED: &str = "Element is not attached to the DOM";

/// Playwright: `server/dom.ts` `ElementHandle._retryAction`: `'Element is not visible'`
const ELEMENT_NOT_VISIBLE: &str = "Element is not visible";

/// Playwright's loop B for a handle action: attempts on the same element with growing waits
/// until it is done, the element detaches (Playwright's handle throws), or the call's deadline.
/// Unless `force`, each attempt first waits out a pending navigation.
///
/// Playwright: `server/dom.ts` `ElementHandle._retryAction`: `if (result === 'error:notvisible') {`
async fn retry_action<T, F, Fut>(
    tab: &T,
    progress: &Progress,
    action_name: &'static str,
    force: bool,
    mut attempt: F,
) -> Result<(), CallError>
where
    T: Tab,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Attempt, CallError>>,
{
    let mut retry = RetryAction::new(action_name);
    loop {
        retry
            .before(tab, PreChecks::unless_forced(force), progress)
            .await
            .map_err(|stop| match stop {
                Stop::End(error) => error,
                Stop::Retry => PlaywrightError::error(ELEMENT_NOT_ATTACHED).into(),
            })?;
        let result = attempt().await?;
        match result {
            Attempt::Done => return Ok(()),
            Attempt::NotConnected => {
                return Err(PlaywrightError::error(ELEMENT_NOT_ATTACHED).into());
            }
            Attempt::NotVisible if force => {
                return Err(PlaywrightError::error(ELEMENT_NOT_VISIBLE).into());
            }
            Attempt::NotVisible => progress.log("  element is not visible"),
            Attempt::Missing(ElementState::NonInert) => progress.log("  element is inert"),
            Attempt::Missing(state) => {
                progress.log(format!("  element is not {}", state_name(state)));
            }
        }
    }
}

/// Neo: a page step's failure inside a handle action. A lost document is the element detaching
/// (Playwright's handle then throws its detached error); the rest end the call as page reads do.
fn page_attempt(error: PageError) -> Result<Attempt, CallError> {
    match Stop::from(error) {
        Stop::Retry => Ok(Attempt::NotConnected),
        Stop::End(error) => Err(error),
    }
}

/// Neo: an element state as Playwright's log names it (`ElementState`'s string).
fn state_name(state: ElementState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// The tab.
// ---------------------------------------------------------------------------------------------

/// Neo: the element operations over the selected frame's utility world. Each mutating step runs
/// on the page's tracker so the turn and the element's group stay held until the page answers.
impl ElementOpsTab for FrameTab {
    fn blur(
        &self,
        element: &ResolvedElement,
        input: InputPermit,
    ) -> impl Future<Output = Result<FocusOutcome, PageError>> + Send {
        let element = element.clone();
        detached(self, async move {
            let _input = input;
            let params = InjectedBlurElementParams {
                element: element.handle().clone(),
            };
            let answer = element.frame.call(None, &params).await;
            drop(element);
            answer.map(|answer| answer.outcome)
        })
    }

    fn select_text(
        &self,
        element: &ResolvedElement,
        force: bool,
        input: InputPermit,
    ) -> impl Future<Output = Result<SelectTextOutcome, PageError>> + Send {
        let element = element.clone();
        detached(self, async move {
            let _input = input;
            let params = InjectedSelectTextParams {
                element: element.handle().clone(),
                force,
                // Playwright: `server/dom.ts` `ElementHandle.selectText`: `{ force: options.force, frameVisible: true }`
                frame_visible: true,
            };
            let answer = element.frame.call(None, &params).await;
            drop(element);
            answer.map(|answer| answer.outcome)
        })
    }

    /// Playwright: `server/chromium/crPage.ts` `FrameSession._getBoundingBox`: `const quad = result.model.border;`
    async fn bounding_box(&self, element: &ResolvedElement) -> Result<Option<Rect>, PageError> {
        let model = element
            .frame
            .host
            .send_command(cdp::dom::GetBoxModel {
                object_id: Some(element.handle().id().object_id().to_string()),
                ..cdp::dom::GetBoxModel::default()
            })
            .await;
        let model = match model {
            Ok(model) => model,
            Err(CdpError::Json(message)) => {
                return Err(PageError::Contract(format!(
                    "{} answered: {message}",
                    cdp::dom::GetBoxModel::METHOD
                )));
            }
            Err(error) => {
                // Playwright's `_sendMayFail`: no box model is no box, unless the tab is gone.
                let error = element.frame.host.classify(&error);
                return match error {
                    PageError::Closed => Err(error),
                    _ => Ok(None),
                };
            }
        };
        let Some(border) = border_rect(&model.model.border) else {
            return Err(PageError::Contract(format!(
                "{} answered no border quad",
                cdp::dom::GetBoxModel::METHOD
            )));
        };
        // Playwright: `server/chromium/crPage.ts` `FrameSession._getBoundingBox`: `const position = await this._framePosition();`
        // Each out-of-process hop adds its iframe's border-box origin, as Playwright places
        // the box; input keeps aiming by the content box (`Frames::offset`).
        let Some((dx, dy)) = self.frames.border_offset(&element.frame.host).await? else {
            return Ok(None);
        };
        Ok(Some(Rect {
            x: border.x + dx,
            y: border.y + dy,
            ..border
        }))
    }
}

/// Neo: starts `work` before returning, as the forms' page steps do, so the turn and the element
/// it holds reach the page's answer even when the caller stops waiting.
fn detached<R: Send + 'static>(
    page: &FrameTab,
    work: impl Future<Output = Result<R, PageError>> + Send + 'static,
) -> impl Future<Output = Result<R, PageError>> + Send {
    let (sender, receiver) = oneshot::channel();
    page.spawn(async move {
        let _waiting = sender.send(work.await);
    });
    async move { receiver.await.unwrap_or(Err(PageError::Closed)) }
}

/// Playwright: `server/chromium/crPage.ts` `FrameSession._getBoundingBox`: `const x = Math.min(quad[0], quad[2], quad[4], quad[6]);`
fn border_rect(points: &[f64]) -> Option<Rect> {
    if points.len() != 8 {
        return None;
    }
    let xs = [points[0], points[2], points[4], points[6]];
    let ys = [points[1], points[3], points[5], points[7]];
    let x = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let y = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let width = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max) - x;
    let height = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max) - y;
    Some(Rect {
        x,
        y,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    //! The pure step: the border box. The retries and the page side are covered by the
    //! real-browser tests (`tests/element_ops_e2e.rs`, neo-quickjs-injected's parity).

    use super::*;

    #[test]
    fn tests_that_a_border_quad_becomes_its_bounding_rect() {
        assert_eq!(
            border_rect(&[10.0, 20.0, 110.0, 20.0, 110.0, 70.0, 10.0, 70.0]),
            Some(Rect {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 50.0
            })
        );
        assert_eq!(border_rect(&[1.0, 2.0]), None);
    }
}
