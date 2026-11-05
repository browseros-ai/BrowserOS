//! The action ops and their registration metadata.

use serde_json::Value;

use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, TARGET};
use crate::{Playwright, PlaywrightError};

listed_enum! {
    /// Actions on one element. The locator, page, frame and element-handle forms of a method
    /// are one op, and so are Playwright's client-side aliases: `clear` is `fill('')`,
    /// `setChecked` is `check` or `uncheck`, `type` is `pressSequentially` and `dragAndDrop`
    /// is `dragTo`. The prelude names the method it ran in `call.api`.
    pub(crate) enum ActionOp {
        Click,
        Dblclick,
        Hover,
        Check,
        Uncheck,
        DragTo,
        Fill,
        Press,
        PressSequentially,
        SelectOption,
        SelectText,
        Focus,
        Blur,
        SetInputFiles,
        DispatchEvent,
        ScrollIntoViewIfNeeded,
    }
}

impl ActionOp {
    /// This op's registration metadata.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::Click => OpSpec::write(
                "locator.click",
                TARGET,
                "click once visible, enabled, stable and hit; also page, frame and handle click",
            ),
            Self::Dblclick => OpSpec::write(
                "locator.dblclick",
                TARGET,
                "double-click after the click checks; also the page, frame and handle forms",
            ),
            Self::Hover => OpSpec::write(
                "locator.hover",
                TARGET,
                "move the mouse over an element once visible, stable and hit",
            ),
            Self::Check => OpSpec::write(
                "locator.check",
                TARGET,
                "check a checkbox or radio and verify it changed; also setChecked(true)",
            ),
            Self::Uncheck => OpSpec::write(
                "locator.uncheck",
                TARGET,
                "uncheck a checkbox and verify it changed; also setChecked(false)",
            ),
            Self::DragTo => OpSpec::write(
                "locator.dragTo",
                TARGET,
                "drag one element onto another; also page.dragAndDrop and frame.dragAndDrop",
            ),
            Self::Fill => OpSpec::write(
                "locator.fill",
                TARGET,
                "replace an editable element's value; also clear()",
            )
            .sensitive(&["/value"]),
            Self::Press => OpSpec::write(
                "locator.press",
                TARGET,
                "focus an element, then press a key or chord",
            )
            .sensitive(&["/key"]),
            Self::PressSequentially => OpSpec::write(
                "locator.pressSequentially",
                TARGET,
                "focus an element, then type text key by key; also type()",
            )
            .sensitive(&["/text"]),
            Self::SelectOption => OpSpec::write(
                "locator.selectOption",
                TARGET,
                "select options of a <select> by value, label or index",
            ),
            Self::SelectText => {
                OpSpec::write("locator.selectText", TARGET, "select an element's text")
            }
            Self::Focus => OpSpec::write("locator.focus", TARGET, "focus an element"),
            Self::Blur => OpSpec::write("locator.blur", TARGET, "remove focus from an element"),
            Self::SetInputFiles => OpSpec::write(
                "locator.setInputFiles",
                TARGET,
                "set a file input's files from host paths",
            ),
            Self::DispatchEvent => OpSpec::write(
                "locator.dispatchEvent",
                TARGET,
                "dispatch a DOM event on an element",
            ),
            Self::ScrollIntoViewIfNeeded => OpSpec::read(
                "locator.scrollIntoViewIfNeeded",
                TARGET,
                "scroll an element into view once it is stable",
            ),
        }
    }
}

/// Runs one action op.
#[expect(
    clippy::unused_async,
    reason = "every action answers `not built yet`, so nothing awaits here yet"
)]
pub(crate) async fn invoke(
    _pw: &Playwright,
    scope: &OpScope<'_>,
    _op: ActionOp,
    _args: OpArgs,
) -> Result<Value, PlaywrightError> {
    Err(scope.not_built())
}
