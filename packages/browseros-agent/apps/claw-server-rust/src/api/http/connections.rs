use super::{error, internal};
use crate::{
    AppState,
    error::{CanonicalError, RequestId},
    services::harness::{ConnectionState, Harness as ServiceHarness},
};
use axum::{Extension, Json, extract::Path, extract::State, http::StatusCode};
use claw_api::models::{Connection, ConnectionList, Harness};
use std::str::FromStr;

pub(super) async fn list(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<ConnectionList>, CanonicalError> {
    let items = state
        .harness
        .list_browseros_connections()
        .await
        .map_err(|source| internal(&request_id, source))?
        .into_iter()
        .map(to_contract)
        .collect();
    Ok(Json(ConnectionList::new(items)))
}

pub(super) async fn connect(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(harness): Path<String>,
) -> Result<Json<Connection>, CanonicalError> {
    let harness = parse_harness(&request_id, &harness)?;
    let connection = state
        .harness
        .connect_browseros(harness, &state.config.public_mcp_url())
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(to_contract(connection)))
}

pub(super) async fn disconnect(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(harness): Path<String>,
) -> Result<Json<Connection>, CanonicalError> {
    let harness = parse_harness(&request_id, &harness)?;
    let connection = state
        .harness
        .disconnect_browseros(harness)
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(to_contract(connection)))
}

fn parse_harness(request_id: &RequestId, raw: &str) -> Result<ServiceHarness, CanonicalError> {
    ServiceHarness::from_str(raw).map_err(|_| {
        error(
            request_id,
            StatusCode::NOT_FOUND,
            "harness_not_found",
            "unknown harness",
        )
    })
}

fn to_contract(state: ConnectionState) -> Connection {
    let mut connection = Connection::new(
        match state.harness {
            ServiceHarness::ClaudeCode => Harness::ClaudeCode,
            ServiceHarness::Codex => Harness::Codex,
            ServiceHarness::Cursor => Harness::Cursor,
            ServiceHarness::OpenCode => Harness::OpenCode,
            ServiceHarness::Antigravity => Harness::Antigravity,
            ServiceHarness::VsCode => Harness::VsCode,
            ServiceHarness::Zed => Harness::Zed,
        },
        state.installed,
        state.message,
    );
    connection.config_path = state.config_path;
    connection
}
