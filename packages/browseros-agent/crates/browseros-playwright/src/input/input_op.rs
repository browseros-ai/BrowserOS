//! The keyboard and mouse ops and their registration metadata.

use serde_json::Value;

use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, PAGE};
use crate::{Playwright, PlaywrightError};

listed_enum! {
    /// Page-level keyboard and mouse input. `mouse.dblclick` is `mouse.click` with a click
    /// count of 2, as in Playwright's client.
    pub(crate) enum InputOp {
        KeyboardDown,
        KeyboardUp,
        KeyboardPress,
        KeyboardType,
        KeyboardInsertText,
        MouseMove,
        MouseDown,
        MouseUp,
        MouseClick,
        MouseWheel,
    }
}

impl InputOp {
    /// This op's registration metadata.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::KeyboardDown => {
                OpSpec::write("keyboard.down", PAGE, "hold a key down").sensitive(&["/key"])
            }
            Self::KeyboardUp => OpSpec::write("keyboard.up", PAGE, "release a held key"),
            Self::KeyboardPress => OpSpec::write(
                "keyboard.press",
                PAGE,
                "press a key or chord on the focused element",
            )
            .sensitive(&["/key"]),
            Self::KeyboardType => OpSpec::write(
                "keyboard.type",
                PAGE,
                "type text key by key into the focused element",
            )
            .sensitive(&["/text"]),
            Self::KeyboardInsertText => OpSpec::write(
                "keyboard.insertText",
                PAGE,
                "insert text into the focused element without key events",
            )
            .sensitive(&["/text"]),
            Self::MouseMove => OpSpec::write("mouse.move", PAGE, "move the mouse to a point"),
            Self::MouseDown => OpSpec::write(
                "mouse.down",
                PAGE,
                "press a mouse button where the mouse is",
            ),
            Self::MouseUp => OpSpec::write("mouse.up", PAGE, "release a mouse button"),
            Self::MouseClick => {
                OpSpec::write("mouse.click", PAGE, "click at a point; also mouse.dblclick")
            }
            Self::MouseWheel => {
                OpSpec::write("mouse.wheel", PAGE, "scroll the wheel where the mouse is")
            }
        }
    }
}

/// Runs one keyboard or mouse op.
#[expect(
    clippy::unused_async,
    reason = "every input op answers `not built yet`, so nothing awaits here yet"
)]
pub(crate) async fn invoke(
    _pw: &Playwright,
    scope: &OpScope<'_>,
    _op: InputOp,
    _args: OpArgs,
) -> Result<Value, PlaywrightError> {
    Err(scope.not_built())
}
