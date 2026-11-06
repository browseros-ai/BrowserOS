//! Options and handler seam for check/uncheck. The policy reuses click's per-element loop B.

use neo_runtime_spec::injected::Point;
use neo_runtime_spec::serve::{
    FrameCheckParams, FrameCheckParamsScroll, FrameUncheckParams, FrameUncheckParamsScroll,
};

use crate::click::{ClickOptions, MouseButton, MouseClick};
use crate::forms::FormTab;
use crate::progress::{CallError, PlaywrightError, Progress};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheckOptions {
    pub(crate) checked: bool,
    pub(crate) click: ClickOptions,
}

impl CheckOptions {
    pub(crate) fn from_check(params: FrameCheckParams) -> Self {
        Self {
            checked: true,
            click: ClickOptions {
                selector: params.selector,
                strict: params.strict.unwrap_or(false),
                force: params.force.unwrap_or(false),
                scroll: params.scroll != Some(FrameCheckParamsScroll::None),
                wait_after: true,
                position: params.position.map(|point| Point {
                    x: point.x,
                    y: point.y,
                }),
                mouse: MouseClick {
                    button: MouseButton::Left,
                    click_count: 1,
                    delay: None,
                },
                trial: params.trial.unwrap_or(false),
            },
        }
    }

    pub(crate) fn from_uncheck(params: FrameUncheckParams) -> Self {
        Self {
            checked: false,
            click: ClickOptions {
                selector: params.selector,
                strict: params.strict.unwrap_or(false),
                force: params.force.unwrap_or(false),
                scroll: params.scroll != Some(FrameUncheckParamsScroll::None),
                wait_after: true,
                position: params.position.map(|point| Point {
                    x: point.x,
                    y: point.y,
                }),
                mouse: MouseClick {
                    button: MouseButton::Left,
                    click_count: 1,
                    delay: None,
                },
                trial: params.trial.unwrap_or(false),
            },
        }
    }
}

#[expect(
    clippy::unused_async,
    reason = "the stub preserves the async handler contract for the next implementation piece"
)]
pub(crate) async fn set_checked<T: FormTab>(
    _tab: &T,
    options: &CheckOptions,
    _progress: &Progress,
) -> Result<(), CallError> {
    let method = if options.checked { "check" } else { "uncheck" };
    Err(PlaywrightError::error(format!("Neo does not serve Frame.{method} yet")).into())
}
