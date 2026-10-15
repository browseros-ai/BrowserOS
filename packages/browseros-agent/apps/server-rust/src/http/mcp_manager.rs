//! Settings HTTP adapter for MCP configuration. Install JSON is deliberately
//! lenient on malformed/empty bodies, unlike Zod routes; a valid client proxy URL
//! takes precedence over the internal listening port.

use crate::{app::AppState, klavis::Klavis, mcp_manager::McpManagerService};
use std::sync::Arc;

/// The settings surface needs only config management and connector publication.
/// Isolating it from AppState lets acceptance run without booting browser/DB tasks.
#[derive(Clone)]
pub struct ManagerState {
    pub manager: McpManagerService,
    pub klavis: Arc<Klavis>,
    pub server_port: u16,
}
impl axum::extract::FromRef<AppState> for ManagerState {
    fn from_ref(state: &AppState) -> Self {
        Self {
            manager: state.mcp_manager.clone(),
            klavis: state.klavis.clone(),
            server_port: state.config.server_port,
        }
    }
}
use super::errors;
use axum::{
    Json,
    body::to_bytes,
    extract::{Path, Request, State},
    response::{IntoResponse, Response},
};
use http::StatusCode;
use serde_json::{Value, json};

pub async fn tools(State(state): State<ManagerState>) -> Response {
    let mut tools: Vec<_> = browseros_mcp::tools::catalog()
        .into_iter()
        .map(|t| json!({"name":t.name,"description":t.description}))
        .collect();
    tools.extend(
        state
            .klavis
            .tool_provider()
            .tools(None)
            .await
            .into_iter()
            .map(|t| json!({"name":t.name,"description":t.description.unwrap_or_default()})),
    );
    Json(json!({"tools":tools})).into_response()
}
pub async fn agents(State(state): State<ManagerState>) -> Response {
    match state.manager.agents().await {
        Ok(agents) => Json(json!({"agents":agents})).into_response(),
        Err(error) => errors::message_only(StatusCode::INTERNAL_SERVER_ERROR, &error.0),
    }
}
pub async fn install(
    State(state): State<ManagerState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    // c.req.json().catch(() => ({})) reads JSON regardless of content-type.
    let value = match to_bytes(request.into_body(), usize::MAX).await {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes).unwrap_or_else(|_| json!({})),
        Err(_) => json!({}),
    };
    if !value.is_object()
        || value
            .get("mcpUrl")
            .is_some_and(|v| v.as_str().is_none_or(|s| url::Url::parse(s).is_err()))
    {
        return errors::success_false(
            StatusCode::BAD_REQUEST,
            "Invalid request body: mcpUrl must be a URL.",
        );
    }
    let fallback = format!("http://127.0.0.1:{}/mcp", state.server_port);
    let url = value
        .get("mcpUrl")
        .and_then(Value::as_str)
        .unwrap_or(&fallback);
    match state.manager.install_at(&id, url).await {
        Ok(()) => Json(json!({"success":true})).into_response(),
        Err(error) => errors::success_false(error.status(), &error.0),
    }
}
pub async fn uninstall(State(state): State<ManagerState>, Path(id): Path<String>) -> Response {
    match state.manager.uninstall(&id).await {
        Ok(()) => Json(json!({"success":true})).into_response(),
        Err(error) => errors::success_false(error.status(), &error.0),
    }
}
