//! Options and dispatch seams for fill, selectOption and inputValue. Their policy bodies land
//! after the shared forms contract and keyboard writer, so all three currently reply explicitly.

use neo_runtime_spec::serve::{FrameFillParams, FrameInputValueParams, FrameSelectOptionParams};

use crate::forms::{FormTab, SelectOption};
use crate::progress::{CallError, PlaywrightError, Progress};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FillOptions {
    pub(crate) selector: String,
    pub(crate) strict: bool,
    pub(crate) value: String,
    pub(crate) force: bool,
}

impl FillOptions {
    pub(crate) fn from_params(params: FrameFillParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
            value: params.value,
            force: params.force.unwrap_or(false),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectOptions {
    pub(crate) selector: String,
    pub(crate) strict: bool,
    pub(crate) options: Vec<SelectOption>,
    pub(crate) force: bool,
}

impl SelectOptions {
    pub(crate) fn from_params(params: FrameSelectOptionParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
            options: params
                .options
                .unwrap_or_default()
                .into_iter()
                .map(|option| SelectOption {
                    value_or_label: option.value_or_label,
                    value: option.value,
                    label: option.label,
                    index: option.index,
                })
                .collect(),
            force: params.force.unwrap_or(false),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InputValueOptions {
    pub(crate) selector: String,
    pub(crate) strict: bool,
}

impl InputValueOptions {
    pub(crate) fn from_params(params: FrameInputValueParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
        }
    }
}

// The input handlers obtain the DeliveryCell from Progress, exactly as click does. inputValue
// has no delivery cell. All handlers keep the same generic tab seam when their policy lands.
#[expect(
    clippy::unused_async,
    reason = "the stub preserves the async handler contract for the next implementation piece"
)]
pub(crate) async fn fill<T: FormTab>(
    _tab: &T,
    _options: &FillOptions,
    _progress: &Progress,
) -> Result<(), CallError> {
    Err(PlaywrightError::error("Neo does not serve Frame.fill yet").into())
}

#[expect(
    clippy::unused_async,
    reason = "the stub preserves the async handler contract for the next implementation piece"
)]
pub(crate) async fn select_option<T: FormTab>(
    _tab: &T,
    _options: &SelectOptions,
    _progress: &Progress,
) -> Result<Vec<String>, CallError> {
    Err(PlaywrightError::error("Neo does not serve Frame.selectOption yet").into())
}

#[expect(
    clippy::unused_async,
    reason = "the stub preserves the async handler contract for the next implementation piece"
)]
pub(crate) async fn input_value<T: FormTab>(
    _tab: &T,
    _options: &InputValueOptions,
    _progress: &Progress,
) -> Result<String, CallError> {
    Err(PlaywrightError::error("Neo does not serve Frame.inputValue yet").into())
}
