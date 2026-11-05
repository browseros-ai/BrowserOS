//! `POST /v1/tabs/{tabId}/private`: the host makes a tab private, durably, before an owner
//! types into it.
//!
//! A tab the browser does not have is a 404 `tab_not_found`; only the browser knows which tabs
//! are open, so the route asks it (`BrowserTabs`) after the request parses and before the flag
//! is committed. A tab that closes in between keeps a flag that only over-protects, until the
//! browser link reports it closed (`Service::tab_closed`) or reconnects (`retain_tabs`).
//!
//! `POST /v1/tabs/{tabId}/sweep`: reads what the tab's password, card and code fields hold into
//! the known secrets list, in every frame, and answers how many values that registered, never the
//! values themselves. 404 `tab_not_found` for a tab that is not open; 503 `browser_unavailable`
//! when the browser does not answer or not one frame could be read. The takeover flow sweeps its
//! own tab at release; this lets a host that composed its own handoff do the same.

use crate::api_error::ApiError;
use crate::service::ServiceState;
use crate::{query, wire};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use browseros_takeover::sweep;
use browseros_trace::{EpochMillis, PrivateReason};
use claw_sdk_api as api;
use std::sync::Arc;

pub(crate) async fn make_private(
    State(service): State<Arc<ServiceState>>,
    Path(tab): Path<String>,
    body: Bytes,
) -> Result<Json<api::Privacy>, ApiError> {
    let tab = query::tab_id(&tab)?;
    wire::private_request(&body)?;
    service.browser.page(tab).await?;
    service
        .privacy
        .make_private(tab, PrivateReason::Host, EpochMillis::now())
        .await?;
    // A tab that was already private keeps its first record, which is what this reads.
    Ok(Json(wire::privacy(service.privacy.is_private(tab))))
}

pub(crate) async fn sweep_tab(
    State(service): State<Arc<ServiceState>>,
    Path(tab): Path<String>,
) -> Result<Json<api::SweepResult>, ApiError> {
    let tab = query::tab_id(&tab)?;
    let swept = sweep(&service.browser, service.privacy.secrets(), tab).await?;
    Ok(Json(wire::sweep_result(swept)))
}
