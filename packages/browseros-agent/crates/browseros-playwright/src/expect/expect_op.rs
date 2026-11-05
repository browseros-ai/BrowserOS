//! The expect ops and their registration metadata.

use serde_json::Value;

use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, PAGE, TARGET};
use crate::{Playwright, PlaywrightError};

listed_enum! {
    /// Retrying matchers. Non-retrying value matchers (`toBe`, `toEqual`, ...) run in the
    /// prelude and never reach Rust.
    pub(crate) enum ExpectOp {
        Locator,
        Page,
    }
}

impl ExpectOp {
    /// This op's registration metadata.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::Locator => OpSpec::read(
                "expect.locator",
                TARGET,
                "one retrying locator matcher (toBeVisible, toHaveText, ...), with .not",
            ),
            Self::Page => OpSpec::read(
                "expect.page",
                PAGE,
                "one retrying page matcher (toHaveTitle, toHaveURL), with .not",
            ),
        }
    }
}

/// Runs one expect op.
#[expect(
    clippy::unused_async,
    reason = "every matcher answers `not built yet`, so nothing awaits here yet"
)]
pub(crate) async fn invoke(
    _pw: &Playwright,
    scope: &OpScope<'_>,
    _op: ExpectOp,
    _args: OpArgs,
) -> Result<Value, PlaywrightError> {
    Err(scope.not_built())
}
