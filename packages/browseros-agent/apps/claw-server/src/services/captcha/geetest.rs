//! TY3 owns the GeeTest solver and vendor mappings.

use serde_json::Value;

use super::{
    SolveCtx,
    page::CaptchaPage,
    providers::CaptchaBackend,
    types::{Challenge, FlowResult, SolveAnswer, SolveReason, SolveStatus, SolveTask},
};

pub const SUPPORTED: bool = false;
pub(super) const CAPSOLVER_ESTIMATE_USD: f64 = 0.0012;

pub async fn solve<P: CaptchaPage, B: CaptchaBackend>(
    _ctx: &SolveCtx<'_, P, B>,
    _challenge: &Challenge,
) -> FlowResult {
    super::terminal(
        SolveStatus::Unsupported,
        SolveReason::UnsupportedType,
        "This CAPTCHA type is not supported.",
    )
}

pub(super) fn capsolver_task(_task: &SolveTask) -> Result<Value, String> {
    Err("not implemented".into())
}
pub(super) fn capsolver_answer(_task: &SolveTask, _solution: &Value) -> Option<SolveAnswer> {
    None
}
pub(super) fn twocaptcha_task(_task: &SolveTask) -> Result<Value, String> {
    Err("not implemented".into())
}
pub(super) fn twocaptcha_answer(_task: &SolveTask, _solution: &Value) -> Option<SolveAnswer> {
    None
}
pub(super) fn secrets(_task: &SolveTask) -> Vec<&str> {
    vec![]
}
