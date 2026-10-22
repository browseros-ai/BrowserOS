use super::internal;
use crate::{
    AppState,
    error::{CanonicalError, RequestId},
    services::session_efficiency::{SessionEfficiencyAggregate, SessionEfficiencyWindow},
};
use axum::{Extension, Json, extract::State};
use claw_api::models::{CockpitStats, CockpitStatsWindow};

pub(super) async fn stats(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<CockpitStats>, CanonicalError> {
    let aggregate = state
        .session_efficiency
        .aggregate()
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(to_contract(aggregate)))
}

fn to_contract(aggregate: Option<SessionEfficiencyAggregate>) -> CockpitStats {
    let Some(aggregate) = aggregate else {
        return CockpitStats::new(
            false,
            CockpitStatsWindow::default(),
            CockpitStatsWindow::default(),
            CockpitStatsWindow::default(),
        );
    };
    CockpitStats::new(
        true,
        to_contract_window(aggregate.all_time),
        to_contract_window(aggregate.last_30_days),
        to_contract_window(aggregate.last_7_days),
    )
}

fn to_contract_window(window: SessionEfficiencyWindow) -> CockpitStatsWindow {
    CockpitStatsWindow::new(
        window.browser_claw_token_estimate,
        window.screenshot_first_token_estimate,
        window.raw_token_savings_estimate,
        window.human_time_saved_ms,
        window.session_count,
        window.tool_call_count,
    )
}
