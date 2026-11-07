//! Neo: Playwright's element operations, served without `ElementHandle`s. `Frame.focus`,
//! `Frame.blur` and `Frame.dispatchEvent` are Playwright's own commands; `scrollIntoViewIfNeeded`,
//! `selectText` and `boundingBox` are Neo commands on the frame (`neo:`), because Playwright's
//! client reaches them only through a handle (its `Locator._withElement`), which Neo does not
//! have yet. Rust keeps each whole wait, as for `neo:waitForURL`.
//!
//! ```text
//!  focus / blur           loop A (`click::retry_with_progress_if_not_connected`)
//!    resolve → the tab's input turn → focusElement / blurElement glue
//!      detached → resolve again
//!  dispatchEvent          `reads::wait_for_function_on_selector`, no input turn
//!    eventInit decoded here to plain JSON (anything else refused before any page call)
//!    → glue dispatchEvent {selector, strict, type, eventInit}, the first match marked
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

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::time::Duration;

use neo_runtime_spec::injected::{
    ElementState, InjectedBlurElementParams, InjectedDispatchEventParams, InjectedSelectTextParams,
    SelectTextOutcome, SelectTextOutcomeStatus,
};
use neo_runtime_spec::serve::{
    FrameBlurParams, FrameDispatchEventParams, FrameFocusParams, SerializedArgument,
    SerializedValue, SerializedValueV,
};
use neo_runtime_spec::{MethodFlag, MethodMeta};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value, json};
use tokio::sync::oneshot;

use crate::click::{
    self, ActionResult, ElementAction, NotReady, PageError, Resolved, Scroll, StatesCheck, Stop,
    Tab,
};
use crate::forms::{FocusOutcome, FormTab};
use crate::frame_selector;
use crate::page::{FrameTab, InputPermit, ResolvedElement};
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::reads::{self, Matched, SelectorTab};

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
/// neo-sdk's `NeoFrameCommands` and the wire tests. The timeout is in the call's metadata.
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

    /// The `dispatchEvent` glue: resolves and marks the first match and dispatches to it. `None`
    /// while nothing matches; otherwise the element's preview.
    fn dispatch_event(
        &self,
        selector: &str,
        strict: bool,
        event_type: &str,
        event_init: &str,
    ) -> impl Future<Output = Result<Option<String>, PageError>> + Send;
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
    click::retry_with_progress_if_not_connected(
        tab,
        &target.selector,
        target.strict,
        false,
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
    click::retry_with_progress_if_not_connected(
        tab,
        &target.selector,
        target.strict,
        false,
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

/// Neo: one `Frame.dispatchEvent`, its `eventInit` already decoded to plain JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DispatchOptions {
    pub(crate) selector: String,
    pub(crate) strict: bool,
    pub(crate) event_type: String,
    /// JSON text of the init dictionary.
    pub(crate) event_init: String,
}

/// Neo: decodes `Frame.dispatchEvent`'s params, refusing an `eventInit` Neo cannot pass before
/// any page call.
impl DispatchOptions {
    pub(crate) fn from_params(params: FrameDispatchEventParams) -> Result<Self, PlaywrightError> {
        let event_init = plain_event_init(&params.event_init)?;
        Ok(Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
            event_type: params.r#type,
            event_init: event_init.to_string(),
        })
    }
}

/// Deviation: `server/frames.ts` `Frame.dispatchEvent`: `{ mainWorld: true }` Playwright builds and dispatches the event in the page's main world from any serialized value, handles included; Neo dispatches from its isolated world, so `eventInit` takes plain values only (strings, finite numbers, booleans, null, arrays and plain objects) and anything else is refused before any page call.
const EVENT_INIT_UNSERVED: &str = "Neo's dispatchEvent takes plain eventInit values (strings, finite numbers, booleans, null, arrays and plain objects); this one holds";

/// Neo: the plain JSON of a serialized `eventInit`, or the refusal naming what it holds.
fn plain_event_init(argument: &SerializedArgument) -> Result<Value, PlaywrightError> {
    // The wire's handle channels decode to nothing until the evaluation seam serves handles;
    // from then on, a handle in `eventInit` is refused here.
    if !argument.handles.is_empty() {
        return Err(PlaywrightError::error(format!(
            "{EVENT_INIT_UNSERVED} a JSHandle"
        )));
    }
    plain_value(&argument.value, &mut Refs::default())
        .map(|value| value.unwrap_or(Value::Null))
        .map_err(|kind| PlaywrightError::error(format!("{EVENT_INIT_UNSERVED} {kind}")))
}

/// Deviation: `protocol/src/serializers.ts` `innerSerializeValue`: `return { ref: id };` Playwright's serializer writes a value met again as a `ref` to its first occurrence and its server hands the refs on, so any number of references to one value dispatches; Neo hands `eventInit` to its isolated world as JSON, which writes a repeated value out in full each time (a chain of values each holding the next twice doubles at every level), so one `eventInit` expands to at most this many values and a larger one is refused, naming the limit, before any page call.
const EXPANDED_VALUES_MAX: usize = 100_000;

/// Neo: the refusal of an `eventInit` past [`EXPANDED_VALUES_MAX`], which it names.
const EXPANSION_REFUSED: &str =
    "more than 100000 values once each repeated value is written out in full (Neo's limit)";

/// Neo: the arrays and objects decoded so far, by their serialized `id`, the ones still open (an
/// ancestor of the value being decoded), and how many values the expansion has written. A `ref`
/// to a finished value is Playwright's shared value, written again; a `ref` to an open one is a
/// cycle, which JSON cannot carry.
#[derive(Default)]
struct Refs {
    done: HashMap<i64, Value>,
    open: HashSet<i64>,
    written: usize,
}

impl Refs {
    /// Counts `values` more written values, refusing an expansion past [`EXPANDED_VALUES_MAX`].
    fn write(&mut self, values: usize) -> Result<(), &'static str> {
        self.written = self.written.saturating_add(values);
        if self.written > EXPANDED_VALUES_MAX {
            return Err(EXPANSION_REFUSED);
        }
        Ok(())
    }
}

/// Neo: a decoded array or object, closed and kept under its `id` for later `ref`s.
fn finish(value: Value, id: Option<i64>, refs: &mut Refs) -> Value {
    if let Some(id) = id {
        refs.open.remove(&id);
        refs.done.insert(id, value.clone());
    }
    value
}

/// Neo: how many JSON values `value` holds, itself included.
fn size_of_value(value: &Value) -> usize {
    let mut size = 0usize;
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        size = size.saturating_add(1);
        match value {
            Value::Array(items) => stack.extend(items),
            Value::Object(members) => stack.extend(members.values()),
            _ => {}
        }
    }
    size
}

/// Neo: one serialized value as plain JSON: `None` for `undefined` (an object drops that key, as
/// an init dictionary reads a missing member; JSON has no `undefined` array item, so an array
/// holding one is refused), or what it holds that JSON cannot carry.
fn plain_value(value: &SerializedValue, refs: &mut Refs) -> Result<Option<Value>, &'static str> {
    refs.write(1)?;
    if let Some(number) = value.n {
        return Number::from_f64(number)
            .map(|number| Some(Value::Number(number)))
            .ok_or("a number that is not finite");
    }
    if let Some(flag) = value.b {
        return Ok(Some(Value::Bool(flag)));
    }
    if let Some(text) = &value.s {
        return Ok(Some(Value::String(text.clone())));
    }
    if let Some(special) = &value.v {
        return match special {
            SerializedValueV::Undefined => Ok(None),
            SerializedValueV::Null => Ok(Some(Value::Null)),
            SerializedValueV::Minus0 => Ok(Number::from_f64(-0.0).map(Value::Number)),
            SerializedValueV::NaN
            | SerializedValueV::Infinity
            | SerializedValueV::MinusInfinity => Err("a number that is not finite"),
        };
    }
    if let Some(items) = &value.a {
        refs.open.extend(value.id);
        let mut array = Vec::with_capacity(items.len());
        for item in items {
            array.push(plain_value(item, refs)?.ok_or("an array holding undefined")?);
        }
        return Ok(Some(finish(Value::Array(array), value.id, refs)));
    }
    if let Some(members) = &value.o {
        refs.open.extend(value.id);
        let mut object = Map::new();
        for member in members {
            if let Some(item) = plain_value(&member.v, refs)? {
                object.insert(member.k.clone(), item);
            }
        }
        return Ok(Some(finish(Value::Object(object), value.id, refs)));
    }
    if let Some(id) = value.r#ref {
        if refs.open.contains(&id) {
            return Err("a circular reference");
        }
        let shared = refs
            .done
            .get(&id)
            .cloned()
            .ok_or("a reference to a value it does not hold")?;
        // The `ref` itself was counted above, as the root of the value it writes out again.
        refs.write(size_of_value(&shared).saturating_sub(1))?;
        return Ok(Some(shared));
    }
    Err(if value.d.is_some() {
        "a Date"
    } else if value.u.is_some() {
        "a URL"
    } else if value.bi.is_some() {
        "a BigInt"
    } else if value.r.is_some() {
        "a RegExp"
    } else if value.e.is_some() {
        "an Error"
    } else if value.ta.is_some() {
        "a typed array"
    } else if value.h.is_some() {
        "a JSHandle"
    } else if value.r#fn.is_some() {
        "a function"
    } else {
        "a value of an unknown kind"
    })
}

/// Serves `Frame.dispatchEvent`: waits for the first match, then dispatches once.
///
/// Playwright: `server/frames.ts` `Frame.dispatchEvent`: `injectedScript.dispatchEvent(element, data.type, data.eventInit);`
pub(crate) async fn dispatch_event<T: ElementOpsTab>(
    tab: &T,
    options: &DispatchOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    reads::wait_for_function_on_selector(tab, &options.selector, progress, || async {
        let preview = tab
            .dispatch_event(
                &options.selector,
                options.strict,
                &options.event_type,
                &options.event_init,
            )
            .await?;
        Ok(preview.map(|preview| Matched { value: (), preview }))
    })
    .await
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
/// with loop A's backoff. Its object group lives as long as the returned value.
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
    let mut waits = click::LoopAWaits::new(progress.timeout_ms());
    loop {
        let wait = waits.next_wait();
        if !wait.is_zero() {
            progress.wait(wait).await?;
        }
        let attempt: Result<Option<Resolved<T::Element>>, Stop> = async {
            progress.race(Tab::prepare(tab)).await??;
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

/// Playwright: `server/dom.ts` `ElementHandle._retryAction`: `const waitTime = [0, 20, 100, 100, 500];`
const RETRY_WAITS_MS: [u64; 5] = [0, 20, 100, 100, 500];

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
    action_name: &str,
    force: bool,
    mut attempt: F,
) -> Result<(), CallError>
where
    T: Tab,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Attempt, CallError>>,
{
    let mut retry: usize = 0;
    loop {
        if retry == 0 {
            progress.log(format!("attempting {action_name} action"));
        } else {
            progress.log(format!("retrying {action_name} action"));
            let wait = RETRY_WAITS_MS[(retry - 1).min(RETRY_WAITS_MS.len() - 1)];
            if wait > 0 {
                progress.log(format!("  waiting {wait}ms"));
                progress.wait(Duration::from_millis(wait)).await?;
            }
        }
        if !force {
            click::wait_out_pending_navigation(tab, progress)
                .await
                .map_err(|stop| match stop {
                    Stop::End(error) => error,
                    Stop::Retry => PlaywrightError::error(ELEMENT_NOT_ATTACHED).into(),
                })?;
        }
        let result = attempt().await?;
        retry = retry.saturating_add(1);
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
            .send(
                "DOM.getBoxModel",
                json!({ "objectId": element.handle().id().object_id() }),
            )
            .await;
        let model = match model {
            Ok(model) => model,
            Err(error) => {
                // Playwright's `_sendMayFail`: no box model is no box, unless the tab is gone.
                let error = element.frame.host.classify(&error);
                return match error {
                    PageError::Closed => Err(error),
                    _ => Ok(None),
                };
            }
        };
        let Some(border) = border_rect(&model["model"]["border"]) else {
            return Err(PageError::Contract(
                "DOM.getBoxModel answered no border quad".to_string(),
            ));
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

    async fn dispatch_event(
        &self,
        selector: &str,
        strict: bool,
        event_type: &str,
        event_init: &str,
    ) -> Result<Option<String>, PageError> {
        let Some(located) = frame_selector::locate(self, selector, strict).await? else {
            return Ok(None);
        };
        let params = InjectedDispatchEventParams {
            selector: located.selector,
            strict,
            r#type: event_type.to_string(),
            event_init: event_init.to_string(),
        };
        Ok(located.frame.call(None, &params).await?.preview)
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
fn border_rect(quad: &Value) -> Option<Rect> {
    let points: Vec<f64> = quad
        .as_array()?
        .iter()
        .map(Value::as_f64)
        .collect::<Option<_>>()?;
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
    //! The pure steps: `eventInit` decoding and the border box. The retries and the page side
    //! are covered by the real-browser tests (`tests/element_ops_e2e.rs`, neo-sdk's parity).

    use super::*;

    fn argument(value: serde_json::Value) -> SerializedArgument {
        SerializedArgument {
            value: serde_json::from_value(value).expect("a serialized value"),
            handles: Vec::new(),
        }
    }

    #[test]
    fn tests_that_plain_event_init_values_become_json() {
        let init = argument(json!({ "o": [
            { "k": "bubbles", "v": { "b": false } },
            { "k": "clientX", "v": { "n": 12.5 } },
            { "k": "detail", "v": { "o": [{ "k": "list", "v": { "a": [{ "s": "x" }, { "v": "null" }], "id": 2 } }], "id": 3 } },
            { "k": "skipped", "v": { "v": "undefined" } },
        ], "id": 1 }));
        assert_eq!(
            plain_event_init(&init).expect("plain"),
            json!({ "bubbles": false, "clientX": 12.5, "detail": { "list": ["x", null] } })
        );
    }

    #[test]
    fn tests_that_values_json_cannot_carry_are_refused_by_kind() {
        for (wire, kind) in [
            (json!({ "d": "2020-01-01T00:00:00.000Z" }), "a Date"),
            (json!({ "v": "NaN" }), "a number that is not finite"),
            (json!({ "bi": "1" }), "a BigInt"),
            (json!({ "h": 0 }), "a JSHandle"),
        ] {
            let init = argument(json!({ "o": [{ "k": "detail", "v": wire }], "id": 1 }));
            let error = plain_event_init(&init).expect_err("refused");
            assert!(error.message.ends_with(kind), "{}", error.message);
        }
    }

    #[test]
    fn tests_that_a_shared_value_is_written_again_and_only_a_cycle_is_refused() {
        // Playwright's ids in visit order: the init 1, `touches` 2, the touch 3, then the two
        // other lists, whose only item is the touch already met.
        let shared = argument(json!({ "o": [
            { "k": "touches", "v": { "a": [{ "o": [{ "k": "identifier", "v": { "n": 7 } }], "id": 3 }], "id": 2 } },
            { "k": "targetTouches", "v": { "a": [{ "ref": 3 }], "id": 4 } },
            { "k": "changedTouches", "v": { "a": [{ "ref": 3 }], "id": 5 } },
        ], "id": 1 }));
        let touch = json!([{ "identifier": 7.0 }]);
        assert_eq!(
            plain_event_init(&shared).expect("a shared touch"),
            json!({ "touches": touch, "targetTouches": touch, "changedTouches": touch })
        );
        for (wire, kind) in [
            (
                json!({ "o": [{ "k": "detail", "v": { "ref": 1 } }], "id": 1 }),
                "a circular reference",
            ),
            (
                json!({ "o": [{ "k": "detail", "v": { "ref": 9 } }], "id": 1 }),
                "a reference to a value it does not hold",
            ),
        ] {
            let error = plain_event_init(&argument(wire)).expect_err("refused");
            assert!(error.message.ends_with(kind), "{}", error.message);
        }
    }

    #[test]
    fn tests_that_an_expansion_past_neos_bound_is_refused() {
        // Each level holds the one below twice, the second time as a `ref`: 2^20 values.
        let mut level = json!({ "o": [], "id": 1 });
        for id in 2..=21 {
            level = json!({ "o": [
                { "k": "x", "v": level },
                { "k": "y", "v": { "ref": id - 1 } },
            ], "id": id });
        }
        let error = plain_event_init(&argument(level)).expect_err("too large");
        assert!(
            error.message.ends_with(EXPANSION_REFUSED),
            "{}",
            error.message
        );
        assert!(EXPANSION_REFUSED.contains(&EXPANDED_VALUES_MAX.to_string()));
    }

    #[test]
    fn tests_that_many_references_to_one_value_expand_up_to_the_limit() {
        // Playwright's `Array(n).fill(shared)`: the first item in full, every other a `ref`.
        let shallow = |n: usize| {
            let mut items = vec![json!({ "o": [{ "k": "leaf", "v": { "b": true } }], "id": 2 })];
            items.extend((1..n).map(|_| json!({ "ref": 2 })));
            argument(json!({ "a": items, "id": 1 }))
        };
        let expanded = plain_event_init(&shallow(10_000)).expect("within the limit");
        let items = expanded.as_array().expect("an array");
        assert_eq!(items.len(), 10_000);
        assert!(items.iter().all(|item| item == &json!({ "leaf": true })));
        // The review's 60,000 references expand past 100,000 values; Playwright dispatches them.
        let error = plain_event_init(&shallow(60_000)).expect_err("past the limit");
        assert!(
            error.message.ends_with(EXPANSION_REFUSED),
            "{}",
            error.message
        );
    }

    #[test]
    fn tests_that_each_expanded_value_counts_once_against_the_limit() {
        // `{ detail: Array(n).fill(shared) }` writes 2 + 2n values: the init object, the array,
        // and each copy's object and boolean.
        let detail = |n: usize| {
            let mut items = vec![json!({ "o": [{ "k": "leaf", "v": { "b": true } }], "id": 3 })];
            items.extend((1..n).map(|_| json!({ "ref": 3 })));
            argument(json!({ "o": [{ "k": "detail", "v": { "a": items, "id": 2 } }], "id": 1 }))
        };
        // The second review's 40,000 references: 80,002 values.
        let expanded = plain_event_init(&detail(40_000)).expect("80,002 values");
        assert_eq!(expanded["detail"].as_array().map(Vec::len), Some(40_000));
        // Exactly at the limit, and one copy past it.
        assert!(plain_event_init(&detail(49_999)).is_ok(), "100,000 values");
        let error = plain_event_init(&detail(50_000)).expect_err("100,002 values");
        assert!(
            error.message.ends_with(EXPANSION_REFUSED),
            "{}",
            error.message
        );
    }

    #[test]
    fn tests_that_minus_zero_keeps_its_sign_and_undefined_array_items_are_refused() {
        let init = argument(json!({ "o": [{ "k": "detail", "v": { "v": "-0" } }], "id": 1 }));
        let detail = plain_event_init(&init).expect("plain")["detail"].as_f64();
        assert!(detail.is_some_and(|zero| zero == 0.0 && zero.is_sign_negative()));
        let holes = argument(json!({ "o": [{ "k": "detail", "v": {
            "a": [{ "n": 1 }, { "v": "undefined" }], "id": 2 } }], "id": 1 }));
        assert!(
            plain_event_init(&holes)
                .expect_err("an undefined item")
                .message
                .ends_with("an array holding undefined")
        );
        assert_eq!(
            plain_event_init(&argument(json!({ "v": "undefined" }))).expect("undefined"),
            Value::Null
        );
    }

    #[test]
    fn tests_that_a_border_quad_becomes_its_bounding_rect() {
        assert_eq!(
            border_rect(&json!([10.0, 20.0, 110.0, 20.0, 110.0, 70.0, 10.0, 70.0])),
            Some(Rect {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 50.0
            })
        );
        assert_eq!(border_rect(&json!([1.0, 2.0])), None);
    }
}
