//! Neo: Playwright's `ElementHandle` commands on the run's element handles (`server/dom.ts`
//! `ElementHandle`). Each one acts on the handle's own node: nothing resolves a selector again
//! for a handle. `$`, `$$`, `$eval` and `$$eval` are `evaluate.rs`'s.
//!
//! ```text
//!  ElementHandle.<command> on handle@…, served on the frame the handle belongs to (dispatch.rs)
//!   the handle → its node adopted into that frame's utility world (evaluate::utility_element;
//!     a JSHandle is refused as Playwright's dispatcher refuses it, an unknown guid is closed)
//!   textContent innerText innerHTML  the frame reads (reads.rs, states.rs) on a [`Scoped`]
//!   getAttribute inputValue          tab, whose selectors resolve from the node, with the
//!   isChecked isEnabled isDisabled   selector `:scope`: Playwright reads a handle through its
//!   isEditable isVisible isHidden    frame as `{ selector: ':scope', scope: this }`
//!   boundingBox   the `neo:boundingBox` step on the node (element_ops.rs), or null
//!   ownerFrame    the node's document element → DOM.describeNode's frameId
//!   contentFrame  DOM.describeNode of a frame element → its frameId
//!                 → the frame's record (getters::frame_by_id), or null
//! ```
//!
//! A handle whose document is gone: the reads end with Playwright's destroyed-context error,
//! `isVisible` reads false and `isHidden` true (`Frame.isVisibleInternal` catches the error).
//! A detached node in its own document is still read, as Playwright's `:scope` engine matches the
//! scope element whether or not it is connected; a state of one (`isChecked`) ends with
//! `Element is not attached to the DOM`.

use std::future::{self, Future};
use std::sync::{Arc, OnceLock};

use neo_runtime_spec::injected::{
    AttributeMatch, ElementStateMatch, FormInputValueMatch, InjectedAriaSnapshotParams,
    InjectedMarkTargetParams, InjectedOwnerDocumentElementParams,
    InjectedScopedElementStateOnSelectorParams, InjectedScopedGetAttributeParams,
    InjectedScopedInnerHTMLParams, InjectedScopedInnerTextParams, InjectedScopedInputValueParams,
    InjectedScopedIsVisibleParams, InjectedScopedTextContentParams, ReadableElementState,
    TextMatch,
};
use neo_runtime_spec::serve::{
    ElementHandleBoundingBoxResult, ElementHandleCall, ElementHandleGetAttributeResult,
    ElementHandleInnerHTMLResult, ElementHandleInnerTextResult, ElementHandleInputValueResult,
    ElementHandleIsCheckedResult, ElementHandleIsDisabledResult, ElementHandleIsEditableResult,
    ElementHandleIsEnabledResult, ElementHandleIsHiddenResult, ElementHandleIsVisibleResult,
    ElementHandleTextContentResult, Rect,
};
use neo_runtime_spec::{InjectedCall, MethodMeta};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::check::{CheckAction, CheckOptions};
use crate::click::{
    self, ActionResult, ElementAction, PageError, PointerOptions, SelectorFacts, Stop,
    Tab as ClickTab, WaitAfter,
};
use crate::element_ops::{self, ElementOpsTab, Focus};
use crate::evaluate::{self, UtilityElement};
use crate::fill::{FillAction, FillOptions, SelectAction, SelectOptions};
use crate::frame_selector;
use crate::getters;
use crate::keyboard::KeyboardTab;
use crate::page::{FrameTab, ResolvedElement};
use crate::press::{PressAction, PressOptions};
use crate::progress::{CallError, DeliveryCell, PlaywrightError, Progress};
use crate::reads::{self, AriaRead, AttributeOptions, Matched, ReadOptions, ReadTab, SelectorTab};
use crate::states::{self, StateOptions, StateTab};
use crate::tabs::Tabs;

/// The selector each handle read resolves from the handle's node: the node itself.
///
/// Playwright: `server/dom.ts` `ElementHandle.textContent`: `return this._frame.textContent(progress, { selector: ':scope', scope: this });`
const SCOPE: &str = ":scope";

/// Neo: one `ElementHandle` command, on the run's handle `guid`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ElementCommand {
    pub(crate) guid: String,
    pub(crate) call: ElementHandleCall,
}

/// Routes a command on a handle guid that `ElementHandle` serves, with its metainfo; `None` for
/// a method it does not serve, which the `JSHandle` commands answer (`evaluate::route_handle`).
pub(crate) fn route(
    guid: &str,
    method: &str,
    params: Option<Value>,
) -> Option<(
    Option<&'static MethodMeta>,
    Result<ElementCommand, PlaywrightError>,
)> {
    use neo_runtime_spec::CallDecodeError;
    match ElementHandleCall::decode(method, params) {
        Ok(call) => Some((
            Some(call.meta()),
            Ok(ElementCommand {
                guid: guid.to_string(),
                call,
            }),
        )),
        Err(CallDecodeError::UnknownMethod { .. }) => None,
        Err(error) => {
            let meta = match &error {
                CallDecodeError::InvalidParams { meta, .. } => Some(*meta),
                CallDecodeError::UnknownMethod { .. } => None,
            };
            Some((meta, Err(PlaywrightError::error(error.to_string()))))
        }
    }
}

/// Serves one `ElementHandle` command on `tab`, the frame the handle belongs to, and encodes
/// its reply.
///
/// # Errors
///
/// The call's [`CallError`]: `TargetClosedError` for a handle the run does not have, Playwright's
/// mismatching-dispatcher error for a `JSHandle`, and each command's own errors.
pub(crate) async fn serve(
    tabs: &Tabs,
    tab: &FrameTab,
    command: ElementCommand,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let ElementCommand { guid, call } = command;
    let method = call.meta().method;
    let node = progress
        .race(evaluate::utility_element(
            tab,
            &guid,
            method,
            progress.handles(),
        ))
        .await??;
    match call {
        ElementHandleCall::IsVisible(_) | ElementHandleCall::IsHidden(_) => {
            let visible = match &node {
                Ok(node) => {
                    states::is_visible(
                        &Scoped { tab, node },
                        &StateOptions::new(SCOPE.to_string(), None),
                        progress,
                    )
                    .await?
                }
                // Playwright: `server/frames.ts` `Frame.isVisibleInternal`: `return false;`
                Err(PageError::NotConnected) => false,
                Err(error) => return Err(evaluate::page_error(error.clone()).into()),
            };
            if matches!(call, ElementHandleCall::IsHidden(_)) {
                // Playwright: `server/frames.ts` `Frame.isHidden`: `return !(await this.isVisible(progress, target));`
                return encode(&ElementHandleIsHiddenResult { value: !visible });
            }
            encode(&ElementHandleIsVisibleResult { value: visible })
        }
        call @ (ElementHandleCall::Click(_)
        | ElementHandleCall::Dblclick(_)
        | ElementHandleCall::Hover(_)
        | ElementHandleCall::Tap(_)
        | ElementHandleCall::Check(_)
        | ElementHandleCall::Uncheck(_)
        | ElementHandleCall::Fill(_)
        | ElementHandleCall::Press(_)
        | ElementHandleCall::Type(_)
        | ElementHandleCall::SelectOption(_)
        | ElementHandleCall::SelectText(_)
        | ElementHandleCall::Focus(_)
        | ElementHandleCall::ScrollIntoViewIfNeeded(_)) => {
            let node = match node {
                Ok(node) => node,
                Err(PageError::Closed) => return Err(PlaywrightError::target_closed().into()),
                // Playwright: `server/dom.ts` `ElementHandle.evaluateInUtility`: `return 'error:notconnected';`
                Err(PageError::NotConnected) => {
                    return Err(PlaywrightError::error(ELEMENT_NOT_ATTACHED).into());
                }
                Err(error) => return Err(evaluate::page_error(error).into()),
            };
            act(tab, &node, call, progress).await
        }
        ElementHandleCall::BoundingBox(_) => {
            let value = match &node {
                Ok(node) => bounding_box(tab, node, progress).await?,
                Err(PageError::Closed) => return Err(PlaywrightError::target_closed().into()),
                // Playwright: `server/chromium/crPage.ts` `FrameSession._getBoundingBox`: `const result = await this._client._sendMayFail('DOM.getBoxModel', {`
                Err(_) => None,
            };
            encode(&ElementHandleBoundingBoxResult { value })
        }
        call => {
            let node = node.map_err(evaluate::page_error)?;
            read(tabs, tab, &node, call, progress).await
        }
    }
}

/// The node's border box in the main frame's CSS pixels, `None` without one: the
/// `neo:boundingBox` step on the adopted node, which a node gone also answers `None`.
async fn bounding_box(
    tab: &FrameTab,
    node: &UtilityElement,
    progress: &Progress,
) -> Result<Option<Rect>, CallError> {
    // Playwright's box reads no frame element: no owners.
    let element = ResolvedElement::adopted(
        node.element.clone(),
        Arc::clone(&node.group),
        Arc::clone(&node.frame),
        Vec::new(),
    );
    Ok(
        match progress
            .race(ElementOpsTab::bounding_box(tab, &element))
            .await?
        {
            Ok(rect) => rect.map(|rect| Rect {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            }),
            Err(PageError::Closed) => return Err(PlaywrightError::target_closed().into()),
            Err(_) => None,
        },
    )
}

/// Every command but the visibility reads, on the adopted node.
async fn read(
    tabs: &Tabs,
    tab: &FrameTab,
    node: &UtilityElement,
    call: ElementHandleCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let scoped = &Scoped { tab, node };
    let scope = || ReadOptions {
        selector: SCOPE.to_string(),
        strict: false,
    };
    let state = |state| async move {
        states::element_state(
            scoped,
            &StateOptions::new(SCOPE.to_string(), None),
            state,
            progress,
        )
        .await
    };
    match call {
        ElementHandleCall::TextContent(_) => {
            let read = reads::text_content(scoped, &scope(), progress).await?;
            encode(&ElementHandleTextContentResult { value: read.value })
        }
        ElementHandleCall::InnerText(_) => {
            let read = reads::inner_text(scoped, &scope(), progress).await?;
            encode(&ElementHandleInnerTextResult { value: read.value })
        }
        ElementHandleCall::InnerHTML(_) => {
            let read = reads::inner_html(scoped, &scope(), progress).await?;
            encode(&ElementHandleInnerHTMLResult { value: read.value })
        }
        ElementHandleCall::GetAttribute(params) => {
            let options = AttributeOptions {
                selector: SCOPE.to_string(),
                strict: false,
                name: params.name,
            };
            let read = reads::get_attribute(scoped, &options, progress).await?;
            encode(&ElementHandleGetAttributeResult { value: read.value })
        }
        ElementHandleCall::InputValue(_) => {
            let value = input_value(scoped, progress).await?;
            encode(&ElementHandleInputValueResult { value })
        }
        ElementHandleCall::IsChecked(_) => encode(&ElementHandleIsCheckedResult {
            value: state(ReadableElementState::Checked).await?,
        }),
        ElementHandleCall::IsDisabled(_) => encode(&ElementHandleIsDisabledResult {
            value: state(ReadableElementState::Disabled).await?,
        }),
        ElementHandleCall::IsEditable(_) => encode(&ElementHandleIsEditableResult {
            value: state(ReadableElementState::Editable).await?,
        }),
        ElementHandleCall::IsEnabled(_) => encode(&ElementHandleIsEnabledResult {
            value: state(ReadableElementState::Enabled).await?,
        }),
        ElementHandleCall::OwnerFrame(_) => {
            let frame = owner_frame(tabs, tab, node, progress).await?;
            Ok(Some(frame.map_or_else(
                || json!({}),
                |frame| json!({ "frame": frame }),
            )))
        }
        ElementHandleCall::ContentFrame(_) => {
            let frame = content_frame(tabs, tab, node, progress).await?;
            Ok(Some(frame.map_or_else(
                || json!({}),
                |frame| json!({ "frame": frame }),
            )))
        }
        _ => Err(PlaywrightError::error(
            "neo-runtime: the actions, boundingBox and the visibility reads are served by `serve`",
        )
        .into()),
    }
}

/// One action on the node: the per-element step of the same frame command, once, on the node,
/// as Playwright's `ElementHandle.click` runs `_click` with no selector to resolve again. The
/// command's params are the frame command's with no selector, so the frame command's own
/// decoding gives its defaults and refusals, before anything reaches the page as Playwright's
/// validator refuses them.
async fn act(
    tab: &FrameTab,
    node: &UtilityElement,
    call: ElementHandleCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    match call {
        ElementHandleCall::Click(params) => {
            let options = PointerOptions::from_click(frame_params(&params)?)?;
            pointer(tab, node, options, progress).await
        }
        ElementHandleCall::Dblclick(params) => {
            let options = PointerOptions::from_dblclick(frame_params(&params)?)?;
            pointer(tab, node, options, progress).await
        }
        ElementHandleCall::Hover(params) => {
            let options = PointerOptions::from_hover(frame_params(&params)?);
            pointer(tab, node, options, progress).await
        }
        ElementHandleCall::Tap(params) => {
            let options = PointerOptions::from_tap(frame_params(&params)?);
            pointer(tab, node, options, progress).await
        }
        call => form(tab, node, call, progress).await,
    }
}

/// The form and focus actions on the node.
async fn form(
    tab: &FrameTab,
    node: &UtilityElement,
    call: ElementHandleCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    match call {
        ElementHandleCall::Check(params) => {
            let options = CheckOptions::from_check(frame_params(&params)?);
            let action = CheckAction {
                options: &options,
                delivery: delivery(progress)?,
            };
            on_element(tab, node, &action, progress).await?;
            Ok(None)
        }
        ElementHandleCall::Uncheck(params) => {
            let options = CheckOptions::from_uncheck(frame_params(&params)?);
            let action = CheckAction {
                options: &options,
                delivery: delivery(progress)?,
            };
            on_element(tab, node, &action, progress).await?;
            Ok(None)
        }
        ElementHandleCall::Fill(params) => {
            let options = FillOptions::from_params(frame_params(&params)?);
            let action = FillAction {
                options: &options,
                delivery: delivery(progress)?,
                platform: OnceLock::new(),
            };
            on_element(tab, node, &action, progress).await?;
            Ok(None)
        }
        ElementHandleCall::Press(params) => {
            let options = PressOptions::from_params(frame_params(&params)?);
            press(tab, node, &options, progress).await
        }
        ElementHandleCall::Type(params) => {
            let options = PressOptions::from_type(frame_params(&params)?);
            press(tab, node, &options, progress).await
        }
        ElementHandleCall::SelectOption(params) => {
            let guids = params
                .elements
                .iter()
                .flatten()
                .map(|element| element.guid.clone())
                .collect();
            let options = SelectOptions::from_params(frame_params(&params)?).with_elements(guids);
            // Checked before any work, as Playwright's dispatcher checks `elements` channels
            // (`fill::select_option`).
            let elements = options
                .elements
                .iter()
                .enumerate()
                .map(|(index, guid)| progress.handles().select_element(index, guid))
                .collect::<Result<Vec<_>, _>>()?;
            let action = SelectAction {
                options: &options,
                elements: &elements,
                delivery: delivery(progress)?,
            };
            let values = on_element(tab, node, &action, progress).await?;
            Ok(Some(json!({ "values": values })))
        }
        // Playwright marks no target for these two.
        ElementHandleCall::SelectText(params) => {
            prepare(tab, progress).await?;
            let force = params.force.unwrap_or(false);
            let element = adopted(tab, node, progress).await?;
            element_ops::select_text_element(tab, &element, force, progress).await?;
            Ok(None)
        }
        ElementHandleCall::ScrollIntoViewIfNeeded(_) => {
            prepare(tab, progress).await?;
            let element = adopted(tab, node, progress).await?;
            element_ops::scroll_element(tab, &element, progress).await?;
            Ok(None)
        }
        ElementHandleCall::Focus(_) => {
            on_element(tab, node, &Focus::Focus, progress).await?;
            Ok(None)
        }
        _ => Err(PlaywrightError::error("neo-runtime: not an ElementHandle action").into()),
    }
}

/// `press` and `type` on the node, through the frame's press step.
async fn press(
    tab: &FrameTab,
    node: &UtilityElement,
    options: &PressOptions,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let action = PressAction {
        options,
        delivery: delivery(progress)?,
        platform: OnceLock::new(),
    };
    on_element(tab, node, &action, progress).await?;
    Ok(None)
}

/// A pointer action on the node: its target marked, then loop B
/// (`click::retry_pointer_action`), the browser's platform read first for `modifiers` as
/// `click::pointer_action` reads it.
async fn pointer(
    tab: &FrameTab,
    node: &UtilityElement,
    options: PointerOptions,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let delivery = delivery(progress)?;
    mark(node, progress).await?;
    prepare(tab, progress).await?;
    let options = if options.modifiers.is_some() {
        let platform = match progress.race(KeyboardTab::platform(tab)).await? {
            Ok(platform) => platform,
            Err(PageError::Closed) => return Err(PlaywrightError::target_closed().into()),
            Err(_) => {
                return Err(PlaywrightError::error(
                    "neo-runtime: the browser's platform could not be read",
                )
                .into());
            }
        };
        PointerOptions {
            platform: Some(platform),
            ..options
        }
    } else {
        options
    };
    let wait_after = if options.wait_after {
        WaitAfter::Enabled
    } else {
        WaitAfter::Disabled
    };
    let element = adopted(tab, node, progress).await?;
    finish(
        click::retry_pointer_action(tab, &element, &options, wait_after, progress, delivery).await,
    )?;
    Ok(None)
}

/// A frame action's per-element step, once on the node: its target marked and its page made ready,
/// as loop A makes it.
async fn on_element<A: ElementAction<FrameTab>>(
    tab: &FrameTab,
    node: &UtilityElement,
    action: &A,
    progress: &Progress,
) -> Result<A::Output, CallError> {
    mark(node, progress).await?;
    prepare(tab, progress).await?;
    let element = adopted(tab, node, progress).await?;
    finish(action.act(tab, &element, progress).await)
}

/// The node as the per-element steps take it, with its frame's frame elements from the main
/// frame down, as Playwright's `_checkFrameIsHitTarget` walks the element's own frame up to the
/// main frame: without them a pointer step would hit-test a child frame at the main frame's
/// point. A frame gone since the node was adopted has taken the node with it.
///
/// Playwright: `server/dom.ts` `ElementHandle._checkFrameIsHitTarget`: `while (frame.parentFrame()) {`
async fn adopted(
    tab: &FrameTab,
    node: &UtilityElement,
    progress: &Progress,
) -> Result<ResolvedElement, CallError> {
    let owners = match progress
        .race(frame_selector::direct_owners(tab, &node.frame.id))
        .await?
    {
        Ok(owners) => owners,
        Err(PageError::Closed) => return Err(PlaywrightError::target_closed().into()),
        Err(PageError::NotConnected | PageError::Threw(_)) => {
            return Err(PlaywrightError::error(ELEMENT_NOT_ATTACHED).into());
        }
        Err(PageError::Contract(message)) => return Err(PlaywrightError::error(message).into()),
    };
    Ok(ResolvedElement::adopted(
        node.element.clone(),
        Arc::clone(&node.group),
        Arc::clone(&node.frame),
        owners,
    ))
}

/// The target marks an action makes before it acts, which a node that is gone only skips, as
/// Playwright drops `evaluateInUtility`'s `'error:notconnected'` there.
///
/// Playwright: `server/dom.ts` `ElementHandle._markAsTargetElement`: `injected.markTargetElements(new Set([node as Node as Element]));`
async fn mark(node: &UtilityElement, progress: &Progress) -> Result<(), CallError> {
    let marked = progress
        .race(node.frame.call(
            Some(node.group.group()),
            &InjectedMarkTargetParams {
                node: node.element.clone(),
            },
        ))
        .await?;
    match marked {
        Ok(_) | Err(PageError::NotConnected) => Ok(()),
        Err(error) => Err(evaluate::page_error(error).into()),
    }
}

/// What a per-element action came to, for a handle: there is no selector to resolve again, so
/// an element that proved detached, or a page step that lost it, ends the call.
/// Playwright: `server/dom.ts` `throwRetargetableDOMError`: `throwElementIsNotAttached();`
fn finish<R>(result: Result<ActionResult<R>, Stop>) -> Result<R, CallError> {
    match result {
        Ok(ActionResult::Done(value)) => Ok(value),
        Ok(ActionResult::NotConnected) | Err(Stop::Retry) => {
            Err(PlaywrightError::error(ELEMENT_NOT_ATTACHED).into())
        }
        Err(Stop::End(error)) => Err(error),
    }
}

/// The page's domains and watcher, which the actions' steps need, as loop A prepares them.
async fn prepare(tab: &FrameTab, progress: &Progress) -> Result<(), CallError> {
    match progress.race(ClickTab::prepare(tab)).await? {
        Ok(()) => Ok(()),
        Err(PageError::Closed) => Err(PlaywrightError::target_closed().into()),
        Err(PageError::NotConnected) => Err(PlaywrightError::error(ELEMENT_NOT_ATTACHED).into()),
        Err(PageError::Threw(message) | PageError::Contract(message)) => {
            Err(PlaywrightError::error(message).into())
        }
    }
}

/// The input method's delivery cell, which dispatch gives every action of the spec.
fn delivery(progress: &Progress) -> Result<&Arc<DeliveryCell>, CallError> {
    progress.delivery().ok_or_else(|| {
        PlaywrightError::error("neo-runtime: an ElementHandle action has no delivery cell").into()
    })
}

/// The frame command's params for an element command's: Playwright's wire shape of the one is the
/// other's with the frame's `selector`, which the per-element steps never read.
fn frame_params<E: Serialize, F: DeserializeOwned>(params: &E) -> Result<F, CallError> {
    let mut value = serde_json::to_value(params).map_err(|error| {
        PlaywrightError::error(format!(
            "neo-runtime: encoding an ElementHandle action: {error}"
        ))
    })?;
    value["selector"] = json!("");
    serde_json::from_value(value).map_err(|error| {
        PlaywrightError::error(format!(
            "neo-runtime: an ElementHandle action's params: {error}"
        ))
        .into()
    })
}

/// `Frame.inputValue` of `:scope` on the scoped tab: the frame's `_waitForFunctionOnSelector` loop, as
/// Playwright's `ElementHandle.inputValue` runs its frame's.
///
/// Playwright: `server/dom.ts` `ElementHandle.inputValue`: `return this._frame.inputValue(progress, { selector: ':scope', scope: this });`
async fn input_value(scoped: &Scoped<'_>, progress: &Progress) -> Result<String, CallError> {
    reads::wait_for_function_on_selector(scoped, SCOPE, progress, || async {
        let params = InjectedScopedInputValueParams {
            selector: SCOPE.to_string(),
            strict: false,
            root: scoped.node.element.clone(),
        };
        let matched = scoped.call(&params).await?.matched;
        Ok(matched.map(|FormInputValueMatch { value, preview }| Matched { value, preview }))
    })
    .await
}

/// The record of the frame that owns the node, `None` when its document has no element or no
/// page of the context has that frame: the handle's page first, then every other live page of
/// the context, as Playwright looks through the context's pages for an adopted node. A page the
/// run holds no session on yet gets one, as the getters attach one: Playwright's context knows
/// every page, the run only the pages it reached.
///
/// Playwright: `server/dom.ts` `ElementHandle._ownerFrame`: `for (const page of this._page.browserContext.pages()) {`
async fn owner_frame(
    tabs: &Tabs,
    tab: &FrameTab,
    node: &UtilityElement,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let document = progress
        .race(node.frame.call(
            Some(node.group.group()),
            &InjectedOwnerDocumentElementParams {
                node: node.element.clone(),
            },
        ))
        .await?
        .map_err(evaluate::page_error)?;
    let Some(element) = document.element else {
        return Ok(None);
    };
    let Some(frame_id) = progress
        .race(described_frame(node, element.id().object_id()))
        .await??
    else {
        return Ok(None);
    };
    let own = tabs
        .attached()
        .into_iter()
        .find(|(_, page)| Arc::ptr_eq(page, &tab.page))
        .map(|(entry, _)| entry);
    if let Some(entry) = &own
        && let Some(record) = progress
            .race(getters::frame_by_id(tabs, entry, &frame_id))
            .await??
    {
        return Ok(Some(record));
    }
    for entry in progress.race(tabs.live_entries()).await?? {
        if own.as_ref().is_some_and(|own| Arc::ptr_eq(own, &entry)) {
            continue;
        }
        // A page that closed, or cannot be read, since the listing has no frame to give, as a
        // page gone from Playwright's context is no longer among its pages.
        if let Ok(Some(record)) = progress
            .race(getters::frame_by_id(tabs, &entry, &frame_id))
            .await?
        {
            return Ok(Some(record));
        }
    }
    Ok(None)
}

/// The record of the frame a frame element shows, `None` for any other element or a frame the
/// handle's page does not have.
///
/// Playwright: `server/dom.ts` `ElementHandle._contentFrame`: `const isFrameElement = throwRetargetableDOMError(await this._isIframeElement());`
async fn content_frame(
    tabs: &Tabs,
    tab: &FrameTab,
    node: &UtilityElement,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let described = progress
        .race(describe(node, node.element.id().object_id()))
        .await?;
    let described = match described {
        Ok(described) => described,
        // Playwright: `server/dom.ts` `throwRetargetableDOMError`: `throwElementIsNotAttached();`
        Err(PageError::NotConnected) => {
            return Err(PlaywrightError::error(ELEMENT_NOT_ATTACHED).into());
        }
        Err(error) => return Err(evaluate::page_error(error).into()),
    };
    // Playwright: `server/dom.ts` `ElementHandle._isIframeElement`: `return this.evaluateInUtility(([injected, node]) => node && (node.nodeName === 'IFRAME' || node.nodeName === 'FRAME'), {});`
    let name = described["node"]["nodeName"].as_str().unwrap_or_default();
    if name != "IFRAME" && name != "FRAME" {
        return Ok(None);
    }
    // Playwright: `server/chromium/crPage.ts` `FrameSession._getContentFrame`: `if (!nodeInfo || typeof nodeInfo.node.frameId !== 'string')`
    let Some(frame_id) = described["node"]["frameId"].as_str() else {
        return Ok(None);
    };
    let Some((entry, _)) = tabs
        .attached()
        .into_iter()
        .find(|(_, page)| Arc::ptr_eq(page, &tab.page))
    else {
        return Ok(None);
    };
    Ok(progress
        .race(getters::frame_by_id(tabs, &entry, frame_id))
        .await??)
}

/// `DOM.describeNode`'s `frameId` for `object_id`, `None` when CDP names none.
///
/// Playwright: `server/chromium/crPage.ts` `FrameSession._getOwnerFrame`: `const frameId = nodeInfo && typeof nodeInfo.node.frameId === 'string' ?`
async fn described_frame(
    node: &UtilityElement,
    object_id: &str,
) -> Result<Option<String>, CallError> {
    let described = describe(node, object_id)
        .await
        .map_err(evaluate::page_error)?;
    Ok(described["node"]["frameId"].as_str().map(str::to_string))
}

/// `DOM.describeNode` of one object of the node's session.
async fn describe(node: &UtilityElement, object_id: &str) -> Result<Value, PageError> {
    node.frame
        .host
        .send("DOM.describeNode", json!({ "objectId": object_id }))
        .await
        .map_err(|error| node.frame.host.classify(&error))
}

/// Playwright: `server/dom.ts` `throwElementIsNotAttached`: `'Element is not attached to the DOM'`
const ELEMENT_NOT_ATTACHED: &str = "Element is not attached to the DOM";

fn encode<R: Serialize>(result: &R) -> Result<Option<Value>, CallError> {
    serde_json::to_value(result).map(Some).map_err(|error| {
        PlaywrightError::error(format!(
            "neo-runtime: encoding an ElementHandle result: {error}"
        ))
        .into()
    })
}

/// The frame tab the frame reads take, with every selector resolved from one adopted node: a
/// Playwright `SelectorTarget` with a `scope`. Only the reads an element handle has are served.
struct Scoped<'a> {
    tab: &'a FrameTab,
    node: &'a UtilityElement,
}

impl Scoped<'_> {
    /// One page call from the node. The node's document gone is Playwright's destroyed-context
    /// error, which its handle reads race (`raceAgainstContextDestroyed`), rather than a poll.
    async fn call<C: InjectedCall + Sync>(&self, params: &C) -> Result<C::Result, PageError> {
        match self.node.frame.call(None, params).await {
            Err(PageError::NotConnected) => {
                Err(PageError::Threw(evaluate::CONTEXT_DESTROYED.to_string()))
            }
            result => result,
        }
    }
}

impl SelectorTab for Scoped<'_> {
    async fn prepare(&self) -> Result<(), PageError> {
        SelectorTab::prepare(self.tab).await
    }

    async fn describe(&self, selector: &str) -> Result<SelectorFacts, PageError> {
        SelectorTab::describe(self.tab, selector).await
    }
}

impl ReadTab for Scoped<'_> {
    async fn text_content(
        &self,
        selector: &str,
        strict: bool,
    ) -> Result<Option<TextMatch>, PageError> {
        let params = InjectedScopedTextContentParams {
            selector: selector.to_string(),
            strict,
            root: self.node.element.clone(),
        };
        Ok(self.call(&params).await?.matched)
    }

    async fn inner_text(
        &self,
        selector: &str,
        strict: bool,
    ) -> Result<Option<TextMatch>, PageError> {
        let params = InjectedScopedInnerTextParams {
            selector: selector.to_string(),
            strict,
            root: self.node.element.clone(),
        };
        Ok(self.call(&params).await?.matched)
    }

    async fn inner_html(
        &self,
        selector: &str,
        strict: bool,
    ) -> Result<Option<TextMatch>, PageError> {
        let params = InjectedScopedInnerHTMLParams {
            selector: selector.to_string(),
            strict,
            root: self.node.element.clone(),
        };
        Ok(self.call(&params).await?.matched)
    }

    async fn get_attribute(
        &self,
        selector: &str,
        strict: bool,
        name: &str,
    ) -> Result<Option<AttributeMatch>, PageError> {
        let params = InjectedScopedGetAttributeParams {
            selector: selector.to_string(),
            strict,
            name: name.to_string(),
            root: self.node.element.clone(),
        };
        Ok(self.call(&params).await?.matched)
    }

    fn aria_snapshot(
        &self,
        _params: &InjectedAriaSnapshotParams,
        _stitching: bool,
    ) -> impl Future<Output = Result<AriaRead, PageError>> + Send {
        future::ready(Err(PageError::Contract(
            "neo-runtime: an element handle has no ariaSnapshot".to_string(),
        )))
    }
}

impl StateTab for Scoped<'_> {
    /// A node gone reads false, as the frame's visibility reads take a lost context (Playwright's
    /// `isVisibleInternal` catch), so this call keeps CDP's answer.
    async fn is_visible(&self, selector: &str, strict: bool) -> Result<bool, PageError> {
        let params = InjectedScopedIsVisibleParams {
            selector: selector.to_string(),
            strict,
            // The handle's own frame, which no frame hop led to.
            frame_visible: true,
            root: self.node.element.clone(),
        };
        Ok(self.node.frame.call(None, &params).await?.value)
    }

    async fn element_state(
        &self,
        selector: &str,
        strict: bool,
        state: ReadableElementState,
    ) -> Result<Option<ElementStateMatch>, PageError> {
        let params = InjectedScopedElementStateOnSelectorParams {
            selector: selector.to_string(),
            strict,
            state,
            root: self.node.element.clone(),
        };
        Ok(self.call(&params).await?.matched)
    }
}
