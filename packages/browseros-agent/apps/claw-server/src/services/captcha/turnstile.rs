use super::{
    SolveCtx,
    page::CaptchaPage,
    providers::CaptchaBackend,
    types::{Attempts, Challenge, FlowResult, SolveMethod, SolveReason, SolveStatus},
};

pub async fn solve<P: CaptchaPage, B: CaptchaBackend>(
    _ctx: &SolveCtx<'_, P, B>,
    _challenge: &Challenge,
) -> FlowResult {
    FlowResult {
        status: SolveStatus::Failed,
        method: SolveMethod::None,
        reason: SolveReason::BrowserError,
        attempts: Attempts::default(),
        paid_tasks: 0,
        cost_usd: 0.0,
        message: "not implemented".into(),
    }
}
