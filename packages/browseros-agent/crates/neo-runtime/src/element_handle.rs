//! Neo: Playwright's `ElementHandle` commands on the run's element handles (`server/dom.ts`
//! `ElementHandle`). Each one acts on the handle's own node: nothing resolves a selector again
//! for a handle. `$`, `$$`, `$eval` and `$$eval` are `evaluate.rs`'s.
//!
//! ```text
//!  ElementHandle.<command> on handle@…, served on the frame the handle belongs to (dispatch.rs)
//!   the handle → its node adopted into that frame's utility world (evaluate::utility_element;
//!     a JSHandle is refused as Playwright's dispatcher refuses it, an unknown guid is closed)
//!   textContent innerText innerHTML  dom's frame reads (reads.rs, states.rs) on a [`Scoped`]
//!   getAttribute inputValue          tab, whose selectors resolve from the node, with the
//!   isChecked isEnabled isDisabled   selector `:scope`: Playwright reads a handle through its
//!   isEditable isVisible isHidden    frame as `{ selector: ':scope', scope: this }`
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

use std::sync::Arc;

use neo_runtime_spec::injected::{
    AttributeMatch, ElementStateMatch, FormInputValueMatch, InjectedAriaSnapshotParams,
    InjectedOwnerDocumentElementParams, InjectedScopedElementStateOnSelectorParams,
    InjectedScopedGetAttributeParams, InjectedScopedInnerHTMLParams,
    InjectedScopedInnerTextParams, InjectedScopedInputValueParams, InjectedScopedIsVisibleParams,
    InjectedScopedTextContentParams, ReadableElementState, TextMatch,
};
use neo_runtime_spec::serve::{
    ElementHandleCall, ElementHandleGetAttributeResult, ElementHandleInnerHTMLResult,
    ElementHandleInnerTextResult, ElementHandleInputValueResult, ElementHandleIsCheckedResult,
    ElementHandleIsDisabledResult, ElementHandleIsEditableResult, ElementHandleIsEnabledResult,
    ElementHandleIsHiddenResult, ElementHandleIsVisibleResult, ElementHandleTextContentResult,
};
use neo_runtime_spec::{InjectedCall, MethodMeta};
use serde::Serialize;
use serde_json::{Value, json};

use crate::click::{PageError, SelectorFacts};
use crate::evaluate::{self, UtilityElement};
use crate::getters;
use crate::page::FrameTab;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::reads::{self, AriaRead, AttributeOptions, Matched, ReadOptions, ReadTab, SelectorTab};
use crate::states::{self, StateOptions, StateTab};
use crate::tabs::Tabs;

/// The selector each handle read resolves from the handle's node: the node itself.
///
/// Playwright: `server/dom.ts` `ElementHandle.textContent`: `return this._frame.textContent(progress, { selector: ':scope', scope: this });`
const SCOPE: &str = ":scope";

/// Neo: one `ElementHandle` command, on the run's handle `guid`.
#[derive(Debug, Clone, PartialEq, Eq)]
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
        call => {
            let node = node.map_err(evaluate::page_error)?;
            read(tabs, tab, &node, call, progress).await
        }
    }
}

/// Every command but the visibility reads, on the adopted node.
async fn read(
    tabs: &Tabs,
    tab: &FrameTab,
    node: &UtilityElement,
    call: ElementHandleCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let scoped = Scoped { tab, node };
    let scope = || ReadOptions {
        selector: SCOPE.to_string(),
        strict: false,
    };
    let state = |state| async move {
        states::element_state(
            &scoped,
            &StateOptions::new(SCOPE.to_string(), None),
            state,
            progress,
        )
        .await
    };
    match call {
        ElementHandleCall::TextContent(_) => {
            let read = reads::text_content(&scoped, &scope(), progress).await?;
            encode(&ElementHandleTextContentResult { value: read.value })
        }
        ElementHandleCall::InnerText(_) => {
            let read = reads::inner_text(&scoped, &scope(), progress).await?;
            encode(&ElementHandleInnerTextResult { value: read.value })
        }
        ElementHandleCall::InnerHTML(_) => {
            let read = reads::inner_html(&scoped, &scope(), progress).await?;
            encode(&ElementHandleInnerHTMLResult { value: read.value })
        }
        ElementHandleCall::GetAttribute(params) => {
            let options = AttributeOptions {
                selector: SCOPE.to_string(),
                strict: false,
                name: params.name,
            };
            let read = reads::get_attribute(&scoped, &options, progress).await?;
            encode(&ElementHandleGetAttributeResult { value: read.value })
        }
        ElementHandleCall::InputValue(_) => {
            let value = input_value(&scoped, progress).await?;
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
            Ok(Some(frame.map_or_else(|| json!({}), |frame| json!({ "frame": frame }))))
        }
        ElementHandleCall::ContentFrame(_) => {
            let frame = content_frame(tabs, tab, node, progress).await?;
            Ok(Some(frame.map_or_else(|| json!({}), |frame| json!({ "frame": frame }))))
        }
        ElementHandleCall::BoundingBox(_) => Err(PlaywrightError::error(
            "neo-runtime: ElementHandle.boundingBox is not served yet",
        )
        .into()),
        ElementHandleCall::IsVisible(_) | ElementHandleCall::IsHidden(_) => Err(
            PlaywrightError::error("neo-runtime: the visibility reads are served by `serve`")
                .into(),
        ),
    }
}

/// `Frame.inputValue` of `:scope` on the scoped tab: dom's `_waitForFunctionOnSelector` loop, as
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
/// frame of the run has it: the handle's page first, then the run's other pages, as Playwright
/// looks through the context's pages for an adopted node.
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
    let attached = tabs.attached();
    let own = attached
        .iter()
        .filter(|(_, page)| Arc::ptr_eq(page, &tab.page));
    let others = attached
        .iter()
        .filter(|(_, page)| !Arc::ptr_eq(page, &tab.page));
    for (entry, _) in own.chain(others) {
        if let Some(record) = progress
            .race(getters::frame_by_id(tabs, entry, &frame_id))
            .await??
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
        PlaywrightError::error(format!("neo-runtime: encoding an ElementHandle result: {error}"))
            .into()
    })
}

/// The frame tab dom's reads take, with every selector resolved from one adopted node: a
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
            Err(PageError::NotConnected) => Err(PageError::Threw(
                evaluate::CONTEXT_DESTROYED.to_string(),
            )),
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

    async fn aria_snapshot(
        &self,
        _params: &InjectedAriaSnapshotParams,
        _stitching: bool,
    ) -> Result<AriaRead, PageError> {
        Err(PageError::Contract(
            "neo-runtime: an element handle has no ariaSnapshot".to_string(),
        ))
    }
}

impl StateTab for Scoped<'_> {
    /// A node gone reads false, as dom's visibility reads take a lost context (Playwright's
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
