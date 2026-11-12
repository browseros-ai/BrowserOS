//! The read ops and their registration metadata.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use ts_rs::TS;

use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, TARGET};
use crate::page::OpPage;
use crate::probe::{
    PageText, ProbeReply, ProbeRequest, ProbeTarget, QueryState, Read, ReadAll, Rect,
};
use crate::read::element_read::Visibility;
use crate::read::element_wait::ElementWait;
use crate::read::selector_wait::{Returning, SelectorState};
use crate::read::{
    element_handle, element_read, element_wait, normalize, selector_wait, viewport_box,
};
use crate::resolve::{Landed, Resolver};
use crate::target::{RootHandle, Strictness, Target};
use crate::wire::{self, Describe, OpWire, Route, Run, options};
use crate::{ErrorCode, Playwright, PlaywrightError};

listed_enum! {
    /// Reads of one element or of every match. The locator, page, frame and element-handle
    /// forms of a method are one op; the prelude names the method it ran in `call.api`.
    pub(crate) enum ReadOp {
        TextContent,
        InnerText,
        InnerHtml,
        GetAttribute,
        InputValue,
        IsEnabled,
        IsDisabled,
        IsEditable,
        IsChecked,
        IsVisible,
        IsHidden,
        Count,
        AllTextContents,
        AllInnerTexts,
        BoundingBox,
        WaitFor,
        ElementHandle,
        ElementHandles,
        Normalize,
    }
}

impl ReadOp {
    /// Describes exactly the inputs and results used by the runtime route.
    pub(crate) fn wire(self) -> Option<OpWire> {
        route(self, Describe)
    }

    /// This op's registration metadata.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::TextContent => OpSpec::read(
                "locator.textContent",
                TARGET,
                "an element's textContent, once attached",
            ),
            Self::InnerText => OpSpec::read(
                "locator.innerText",
                TARGET,
                "an element's innerText, once attached",
            ),
            Self::InnerHtml => OpSpec::read(
                "locator.innerHTML",
                TARGET,
                "an element's innerHTML, once attached",
            ),
            Self::GetAttribute => OpSpec::read(
                "locator.getAttribute",
                TARGET,
                "one attribute of an element, once attached",
            ),
            Self::InputValue => OpSpec::read(
                "locator.inputValue",
                TARGET,
                "a form control's value, as upstream's inputValue reads it",
            ),
            Self::IsEnabled => OpSpec::read(
                "locator.isEnabled",
                TARGET,
                "whether an element is enabled, once attached",
            ),
            Self::IsDisabled => OpSpec::read(
                "locator.isDisabled",
                TARGET,
                "whether an element is disabled, once attached",
            ),
            Self::IsEditable => OpSpec::read(
                "locator.isEditable",
                TARGET,
                "whether an element is editable, once attached",
            ),
            Self::IsChecked => OpSpec::read(
                "locator.isChecked",
                TARGET,
                "whether a checkbox or radio is checked, once attached",
            ),
            Self::IsVisible => OpSpec::read(
                "locator.isVisible",
                TARGET,
                "whether an element is visible now, without waiting",
            ),
            Self::IsHidden => OpSpec::read(
                "locator.isHidden",
                TARGET,
                "whether an element is hidden now, without waiting",
            ),
            Self::Count => OpSpec::read(
                "locator.count",
                TARGET,
                "how many elements match now, without waiting; also all()",
            ),
            Self::AllTextContents => OpSpec::read(
                "locator.allTextContents",
                TARGET,
                "every match's textContent, without waiting",
            ),
            Self::AllInnerTexts => OpSpec::read(
                "locator.allInnerTexts",
                TARGET,
                "every match's innerText, without waiting",
            ),
            Self::BoundingBox => OpSpec::read(
                "locator.boundingBox",
                TARGET,
                "an element's box in page coordinates, or null when it is not visible",
            ),
            Self::WaitFor => OpSpec::read(
                "locator.waitFor",
                TARGET,
                "wait for a state; also waitForSelector and elementHandle.waitForElementState",
            ),
            Self::ElementHandle => OpSpec::read(
                "locator.elementHandle",
                TARGET,
                "pin the first match now, or null; page.$, frame.$ and elementHandle.$",
            ),
            Self::ElementHandles => OpSpec::read(
                "locator.elementHandles",
                TARGET,
                "pin every match as an ElementHandle; also page.$$ and frame.$$",
            ),
            Self::Normalize => OpSpec::read(
                "locator.normalize",
                TARGET,
                "the selector Playwright's generator writes for the first match, frames included",
            ),
        }
    }
}

/// The reads of every match (`count`, `allTextContents`, `allInnerTexts`): the locator, hops
/// included, and nothing else.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "ReadEveryMatchInput")]
struct EveryMatchInput {
    target: Target,
}
options!(EveryMatchInput);

/// What every match read: how many there were, and one value each for the text reads.
#[derive(Debug, Default)]
struct Matched {
    count: u32,
    values: Vec<Value>,
}

/// The reads of one element, and their page, frame and element-handle forms.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "ReadElementInput")]
struct ElementInput {
    target: Target,
    strict: Strictness,
}
options!(ElementInput);

/// `locator.waitFor`, `page.waitForSelector` and `elementHandle.waitForElementState`, whose
/// states are the element-state names. `omitReturnValue` suppresses a handle for
/// `locator.waitFor`, whatever the requested state.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename = "ReadWaitForInput")]
struct WaitForInput {
    target: Target,
    strict: Strictness,
    // Selector waits default to visible; an element-state call still requires its state.
    // Decode a supplied value as the enum so omission does not also admit JSON null.
    #[serde(default, deserialize_with = "supplied_state")]
    #[ts(optional)]
    state: Option<WaitState>,
    // `locator.waitFor`: no handle back, whatever the state.
    #[serde(default)]
    #[ts(optional = nullable)]
    omit_return_value: bool,
}
options!(WaitForInput);

fn supplied_state<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<WaitState>, D::Error> {
    WaitState::deserialize(deserializer).map(Some)
}

/// Every state `locator.waitFor` takes, and `waitForElementState`'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "ReadWaitState")]
enum WaitState {
    Attached,
    Detached,
    Visible,
    Hidden,
    Stable,
    Enabled,
    Disabled,
    Editable,
}

impl WaitState {
    /// The state as a selector wait, for a locator; `None` for an element-only state.
    fn for_selector(self) -> Option<SelectorState> {
        match self {
            Self::Attached => Some(SelectorState::Attached),
            Self::Detached => Some(SelectorState::Detached),
            Self::Visible => Some(SelectorState::Visible),
            Self::Hidden => Some(SelectorState::Hidden),
            Self::Stable | Self::Enabled | Self::Disabled | Self::Editable => None,
        }
    }

    /// The state as an element handle waits for it; `None` for `attached` and `detached`.
    fn for_element(self) -> Option<ElementWait> {
        match self {
            Self::Visible => Some(ElementWait::Visible),
            Self::Hidden => Some(ElementWait::Hidden),
            Self::Stable => Some(ElementWait::Stable),
            Self::Enabled => Some(ElementWait::Enabled),
            Self::Disabled => Some(ElementWait::Disabled),
            Self::Editable => Some(ElementWait::Editable),
            Self::Attached | Self::Detached => None,
        }
    }
}

/// The immediate query used by `page.$`, `frame.$` and `elementHandle.$`. A locator's
/// waiting `elementHandle()` uses `locator.waitFor`, like upstream's `waitForSelector`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename = "ReadElementHandleInput")]
struct ElementHandleInput {
    target: Target,
    strict: Strictness,
}
options!(ElementHandleInput);

/// `locator.getAttribute(name)`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "ReadAttributeInput")]
struct AttributeInput {
    target: Target,
    strict: Strictness,
    name: String,
}
options!(AttributeInput);

/// Runs the selected typed handler through the shared decode/call/encode boundary.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: ReadOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    route(op, Run { pw, scope, args }).await
}

/// Runtime dispatch and description share these rows. Resolution, polling and policy remain
/// in the existing read runners; their probe JSON is decoded into each op's concrete result.
fn route<'a, R: Route<'a>>(op: ReadOp, r: R) -> R::Out {
    match op {
        ReadOp::TextContent => r.op(wire::command("ElementHandle.textContent"), text_content),
        ReadOp::InnerText => r.op(wire::command("ElementHandle.innerText"), inner_text),
        ReadOp::InnerHtml => r.op(wire::command("ElementHandle.innerHTML"), inner_html),
        ReadOp::GetAttribute => r.op(wire::command("ElementHandle.getAttribute"), get_attribute),
        ReadOp::InputValue => r.op(wire::command("ElementHandle.inputValue"), input_value),
        ReadOp::IsEnabled => r.op(wire::command("ElementHandle.isEnabled"), is_enabled),
        ReadOp::IsDisabled => r.op(wire::command("ElementHandle.isDisabled"), is_disabled),
        ReadOp::IsEditable => r.op(wire::command("ElementHandle.isEditable"), is_editable),
        ReadOp::IsChecked => r.op(wire::command("ElementHandle.isChecked"), is_checked),
        ReadOp::IsVisible => r.op(wire::command("ElementHandle.isVisible"), is_visible),
        ReadOp::IsHidden => r.op(wire::command("ElementHandle.isHidden"), is_hidden),
        ReadOp::BoundingBox => r.op(wire::command("ElementHandle.boundingBox"), bounding_box),
        ReadOp::Count => r.op(wire::command("Frame.queryCount"), count),
        ReadOp::AllTextContents => r.op(
            wire::neo_only(
                "upstream evaluates all matches in the client; Neo performs a probe read",
            ),
            all_text_contents,
        ),
        ReadOp::AllInnerTexts => r.op(
            wire::neo_only(
                "upstream evaluates all matches in the client; Neo performs a probe read",
            ),
            all_inner_texts,
        ),
        ReadOp::Normalize => r.op(
            wire::command("Frame.resolveSelector")
                .adds(&[(
                    "selector",
                    "original selector for diagnostics after brand resolution",
                )])
                .omits(&[("sdkLanguage", "Neo always formats a JavaScript locator")]),
            normalize::normalize,
        ),
        ReadOp::WaitFor => r.op(
            wire::command("Frame.waitForSelector").retypes(&[(
                "state",
                "also serves waitForElementState's stable/enabled/disabled/editable states",
            )]),
            wait_for,
        ),
        ReadOp::ElementHandle => r.op(wire::command("Frame.querySelector"), handle),
        ReadOp::ElementHandles => r.op(wire::command("Frame.querySelectorAll"), handles),
    }
}

async fn text_content(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementInput,
) -> Result<Option<String>, PlaywrightError> {
    element(pw, scope, input, Read::TextContent).await
}

async fn inner_text(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementInput,
) -> Result<String, PlaywrightError> {
    element(pw, scope, input, Read::InnerText).await
}

async fn inner_html(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementInput,
) -> Result<String, PlaywrightError> {
    element(pw, scope, input, Read::InnerHtml).await
}

async fn input_value(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementInput,
) -> Result<String, PlaywrightError> {
    element(pw, scope, input, Read::InputValue).await
}

async fn is_enabled(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementInput,
) -> Result<bool, PlaywrightError> {
    element(
        pw,
        scope,
        input,
        Read::State {
            state: QueryState::Enabled,
        },
    )
    .await
}

async fn is_disabled(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementInput,
) -> Result<bool, PlaywrightError> {
    element(
        pw,
        scope,
        input,
        Read::State {
            state: QueryState::Disabled,
        },
    )
    .await
}

async fn is_editable(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementInput,
) -> Result<bool, PlaywrightError> {
    element(
        pw,
        scope,
        input,
        Read::State {
            state: QueryState::Editable,
        },
    )
    .await
}

async fn is_checked(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementInput,
) -> Result<bool, PlaywrightError> {
    element(
        pw,
        scope,
        input,
        Read::State {
            state: QueryState::Checked,
        },
    )
    .await
}

async fn get_attribute(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: AttributeInput,
) -> Result<Option<String>, PlaywrightError> {
    let AttributeInput {
        target,
        strict,
        name,
    } = input;
    element(
        pw,
        scope,
        ElementInput { target, strict },
        Read::Attribute { name },
    )
    .await
}

async fn is_visible(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementInput,
) -> Result<bool, PlaywrightError> {
    let ElementInput { target, strict } = input;
    element_read::read_visibility(pw, scope, &target, strict, Visibility::Visible).await
}

async fn is_hidden(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementInput,
) -> Result<bool, PlaywrightError> {
    let ElementInput { target, strict } = input;
    element_read::read_visibility(pw, scope, &target, strict, Visibility::Hidden).await
}

async fn bounding_box(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementInput,
) -> Result<Option<Rect>, PlaywrightError> {
    let ElementInput { target, strict } = input;
    let reading =
        element_read::read_attached(pw, scope, &target, strict, &Read::BoundingBox).await?;
    viewport_box::in_main_viewport(scope, &reading.path, reading.value).await
}

async fn count(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: EveryMatchInput,
) -> Result<u32, PlaywrightError> {
    Ok(every_match(pw, scope, input, ReadAll::Count).await?.count)
}

async fn all_text_contents(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: EveryMatchInput,
) -> Result<Vec<String>, PlaywrightError> {
    read_texts(pw, scope, input, ReadAll::TextContents).await
}

async fn all_inner_texts(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: EveryMatchInput,
) -> Result<Vec<String>, PlaywrightError> {
    read_texts(pw, scope, input, ReadAll::InnerTexts).await
}

/// Both text-vector reads decode the probe's page values at this shared protocol boundary.
async fn read_texts(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: EveryMatchInput,
    read: ReadAll,
) -> Result<Vec<String>, PlaywrightError> {
    let matched = every_match(pw, scope, input, read).await?;
    matched
        .values
        .into_iter()
        .map(|value| element_read::read_value(scope, value))
        .collect()
}

async fn handle(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementHandleInput,
) -> Result<Option<RootHandle>, PlaywrightError> {
    let ElementHandleInput { target, strict } = input;
    let Target::Locator(target) = &target else {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            "target: an element handle is not a selector; query from it with root",
        ));
    };
    element_handle::find_one(pw, scope, target, strict).await
}

async fn handles(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: EveryMatchInput,
) -> Result<Vec<RootHandle>, PlaywrightError> {
    let EveryMatchInput { target } = input;
    let Target::Locator(target) = &target else {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            "target: an element handle is not a selector; query from it with root",
        ));
    };
    element_handle::query_all(pw, scope, target).await
}

/// `locator.waitFor` and `page.waitForSelector`: upstream's `waitForSelector`, for the four
/// states a locator takes; `elementHandle.waitForElementState` for a handle's six.
async fn wait_for(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WaitForInput,
) -> Result<Option<RootHandle>, PlaywrightError> {
    let target = match &input.target {
        Target::Locator(target) => target,
        Target::Element(target) => {
            let state = input
                .state
                .ok_or_else(|| scope.fail(ErrorCode::InvalidInput, "missing field `state`"))?;
            let Some(state) = state.for_element() else {
                return Err(scope.fail(
                    ErrorCode::InvalidInput,
                    "state: expected one of (visible|hidden|stable|enabled|disabled|editable)",
                ));
            };
            element_wait::wait_for_element_state(pw, scope, target, state).await?;
            return Ok(None);
        }
    };
    let Some(state) = input.state.unwrap_or(WaitState::Visible).for_selector() else {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            "state: expected one of (attached|detached|visible|hidden)",
        ));
    };
    let returning = if input.omit_return_value {
        Returning::Nothing
    } else {
        Returning::Handle
    };
    selector_wait::wait_for_match(pw, scope, target, input.strict, state, returning).await
}

/// Every element that matches now, as upstream's `queryCount` and the client's `$$eval`s: one
/// attempt, no waiting, frame hops first-match. A frame on the way that is not there yet
/// matches nothing.
async fn every_match(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: EveryMatchInput,
    read: ReadAll,
) -> Result<Matched, PlaywrightError> {
    let Target::Locator(target) = &input.target else {
        return Err(scope.not_built());
    };
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    let resolver = Resolver::new(&page, scope, target, Strictness::FirstMatch);
    let request = |last, _| ProbeRequest::ReadAll {
        target: ProbeTarget::Chunk(last),
        read,
    };
    let Some(Landed { reply, .. }) = resolver.attempt_at(request).await? else {
        return Ok(Matched::default());
    };
    let ProbeReply::Values { count, values } = reply else {
        return Err(resolver.refused(reply));
    };
    Ok(Matched {
        count,
        values: values.into_iter().map(PageText::into_value).collect(),
    })
}

/// One element's read, once the locator matches (`read_attached`).
async fn element<T: DeserializeOwned>(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementInput,
    read: Read,
) -> Result<T, PlaywrightError> {
    let reading =
        element_read::read_attached(pw, scope, &input.target, input.strict, &read).await?;
    element_read::read_value(scope, reading.value)
}
