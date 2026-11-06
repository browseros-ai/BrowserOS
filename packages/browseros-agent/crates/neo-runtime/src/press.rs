//! Frame.press options and its handler seam. The keyboard writer and press policy land next.

use neo_runtime_spec::serve::FramePressParams;

use crate::forms::FormTab;
use crate::progress::{CallError, PlaywrightError, Progress};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PressOptions {
    pub(crate) selector: String,
    pub(crate) strict: bool,
    pub(crate) key: String,
    pub(crate) delay: Option<f64>,
    pub(crate) wait_after: bool,
}

impl PressOptions {
    pub(crate) fn from_params(params: FramePressParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
            key: params.key,
            delay: params.delay,
            wait_after: !params.no_wait_after.unwrap_or(false),
        }
    }
}

#[expect(
    clippy::unused_async,
    reason = "the stub preserves the async handler contract for the next implementation piece"
)]
pub(crate) async fn press<T: FormTab>(
    _tab: &T,
    _options: &PressOptions,
    _progress: &Progress,
) -> Result<(), CallError> {
    Err(PlaywrightError::error("Neo does not serve Frame.press yet").into())
}
