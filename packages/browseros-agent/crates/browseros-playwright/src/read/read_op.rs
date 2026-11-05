//! The read ops and their registration metadata.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, TARGET};
use crate::page::OpPage;
use crate::probe::{ProbeReply, ProbeRequest, ProbeTarget, QueryState, Read, ReadAll};
use crate::read::element_read::Visibility;
use crate::read::element_wait::ElementWait;
use crate::read::selector_wait::{Returning, SelectorState};
use crate::read::{element_handle, element_read, element_wait, selector_wait, viewport_box};
use crate::resolve::{Attempt, Resolver};
use crate::target::{Strictness, Target};
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
    }
}

impl ReadOp {
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
                "a form control's value through the seal, masked when sensitive",
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
                "pin the matching element as an ElementHandle; also page.$ and frame.$",
            ),
            Self::ElementHandles => OpSpec::read(
                "locator.elementHandles",
                TARGET,
                "pin every match as an ElementHandle; also page.$$ and frame.$$",
            ),
        }
    }
}

/// The reads of every match (`count`, `allTextContents`, `allInnerTexts`): the locator, hops
/// included, and nothing else.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EveryMatchInput {
    target: Target,
}

/// What every match read: how many there were, and one value each for the text reads.
#[derive(Debug, Default)]
struct Matched {
    count: u32,
    values: Vec<Value>,
}

/// The reads of one element, and their page, frame and element-handle forms.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ElementInput {
    target: Target,
    strict: Strictness,
}

/// `locator.waitFor`, `page.waitForSelector` and `elementHandle.waitForElementState`, whose
/// states are the element-state names.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WaitForInput {
    target: Target,
    strict: Strictness,
    state: WaitState,
    /// `locator.waitFor`: no handle back, whatever the state.
    omit_return_value: bool,
}

/// Every state `locator.waitFor` takes, and `waitForElementState`'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// `locator.elementHandle`, and `page.$` (`noWait`: the first match now, or null).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ElementHandleInput {
    target: Target,
    strict: Strictness,
    #[serde(default)]
    no_wait: bool,
}

/// `locator.getAttribute(name)`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttributeInput {
    target: Target,
    strict: Strictness,
    name: String,
}

/// Runs one read op.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: ReadOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    match op {
        ReadOp::Count => {
            let matched = every_match(pw, scope, args.decode(scope)?, ReadAll::Count).await?;
            Ok(json!(matched.count))
        }
        ReadOp::AllTextContents => {
            let matched =
                every_match(pw, scope, args.decode(scope)?, ReadAll::TextContents).await?;
            Ok(Value::Array(matched.values))
        }
        ReadOp::AllInnerTexts => {
            let matched = every_match(pw, scope, args.decode(scope)?, ReadAll::InnerTexts).await?;
            Ok(Value::Array(matched.values))
        }
        ReadOp::TextContent => element(pw, scope, args.decode(scope)?, Read::TextContent).await,
        ReadOp::InnerText => element(pw, scope, args.decode(scope)?, Read::InnerText).await,
        ReadOp::InnerHtml => element(pw, scope, args.decode(scope)?, Read::InnerHtml).await,
        ReadOp::InputValue => element(pw, scope, args.decode(scope)?, Read::InputValue).await,
        ReadOp::GetAttribute => {
            let AttributeInput {
                target,
                strict,
                name,
            } = args.decode(scope)?;
            let input = ElementInput { target, strict };
            element(pw, scope, input, Read::Attribute { name }).await
        }
        ReadOp::BoundingBox => {
            let ElementInput { target, strict } = args.decode(scope)?;
            let reading =
                element_read::read_attached(pw, scope, &target, strict, &Read::BoundingBox).await?;
            viewport_box::in_main_viewport(scope, &reading.path, reading.value).await
        }
        ReadOp::IsEnabled => state(pw, scope, args.decode(scope)?, QueryState::Enabled).await,
        ReadOp::IsDisabled => state(pw, scope, args.decode(scope)?, QueryState::Disabled).await,
        ReadOp::IsEditable => state(pw, scope, args.decode(scope)?, QueryState::Editable).await,
        ReadOp::IsChecked => state(pw, scope, args.decode(scope)?, QueryState::Checked).await,
        ReadOp::IsVisible => {
            let ElementInput { target, strict } = args.decode(scope)?;
            element_read::read_visibility(pw, scope, &target, strict, Visibility::Visible).await
        }
        ReadOp::IsHidden => {
            let ElementInput { target, strict } = args.decode(scope)?;
            element_read::read_visibility(pw, scope, &target, strict, Visibility::Hidden).await
        }
        ReadOp::WaitFor => wait_for(pw, scope, args.decode(scope)?).await,
        ReadOp::ElementHandle => {
            let ElementHandleInput {
                target,
                strict,
                no_wait,
            } = args.decode(scope)?;
            let Target::Locator(target) = &target else {
                return Err(scope.fail(
                    ErrorCode::InvalidInput,
                    "target: an element handle is not a selector; query from it with root",
                ));
            };
            if no_wait {
                element_handle::query_one(pw, scope, target, strict).await
            } else {
                let (state, returning) = (SelectorState::Attached, Returning::Handle);
                selector_wait::wait_for_selector(pw, scope, target, strict, state, returning).await
            }
        }
        ReadOp::ElementHandles => {
            let EveryMatchInput { target } = args.decode(scope)?;
            let Target::Locator(target) = &target else {
                return Err(scope.fail(
                    ErrorCode::InvalidInput,
                    "target: an element handle is not a selector; query from it with root",
                ));
            };
            element_handle::query_all(pw, scope, target).await
        }
    }
}

/// `locator.waitFor` and `page.waitForSelector`: upstream's `waitForSelector`, for the four
/// states a locator takes; `elementHandle.waitForElementState` for a handle's six.
async fn wait_for(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WaitForInput,
) -> Result<Value, PlaywrightError> {
    let target = match &input.target {
        Target::Locator(target) => target,
        Target::Element(target) => {
            let Some(state) = input.state.for_element() else {
                return Err(scope.fail(
                    ErrorCode::InvalidInput,
                    "state: expected one of (visible|hidden|stable|enabled|disabled|editable)",
                ));
            };
            return element_wait::wait_for_element_state(pw, scope, target, state).await;
        }
    };
    let Some(state) = input.state.for_selector() else {
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
    selector_wait::wait_for_selector(pw, scope, target, input.strict, state, returning).await
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
    let page = OpPage::admit(pw, scope, &target.page).await?;
    let resolver = Resolver::new(&page, scope, target, Strictness::FirstMatch);
    let attempt = resolver
        .attempt(|last, _| ProbeRequest::ReadAll {
            target: ProbeTarget::Chunk(last),
            read,
        })
        .await?;
    match attempt {
        Attempt::Missing => Ok(Matched::default()),
        Attempt::Reply(ProbeReply::Values {
            count,
            values,
            sweep,
        }) => {
            element_read::report_swept(pw, &target.page, &sweep);
            Ok(Matched { count, values })
        }
        Attempt::Reply(other) => Err(resolver.refused(other)),
    }
}

/// One element's read, once the locator matches (`read_attached`).
async fn element(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementInput,
    read: Read,
) -> Result<Value, PlaywrightError> {
    let reading =
        element_read::read_attached(pw, scope, &input.target, input.strict, &read).await?;
    Ok(reading.value)
}

/// `isEnabled`, `isDisabled`, `isEditable` and `isChecked`: upstream's `_elementState`, which
/// waits for the element as the other reads do.
async fn state(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ElementInput,
    state: QueryState,
) -> Result<Value, PlaywrightError> {
    element(pw, scope, input, Read::State { state }).await
}
