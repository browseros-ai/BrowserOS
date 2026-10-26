use super::internal;
use crate::{
    AppState,
    error::{CanonicalError, RequestId},
};
use axum::{Extension, Json, extract::State};
use claw_api::models::{ClaudeDesktopExtension, ClaudeDesktopInstallResult};

pub(super) async fn extension(State(state): State<AppState>) -> Json<ClaudeDesktopExtension> {
    Json(state.claude_desktop.status().await)
}

pub(super) async fn install(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<ClaudeDesktopInstallResult>, CanonicalError> {
    let status = state
        .claude_desktop
        .install()
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(ClaudeDesktopInstallResult::new(status)))
}
