//! `POST /v1/tabs/{tabId}/private`: the host makes a tab private, durably, before an owner
//! types into it.
//!
//! A tab the browser does not have is a 404 `tab_not_found`; only the browser knows which tabs
//! are open, so the route asks it (`BrowserTabs`) before the flag is committed and again after.
//! A tab that closed in between is a 404 too, and its flag is forgotten: the browser link's
//! report of the close may have come first (`Service::tab_closed`), and nothing would forget the
//! flag until the link reconnects (`retain_tabs`).
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
use browseros_takeover::{PartError, sweep};
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
    // Before the flag, so a reconcile whose listing missed the tab keeps it, and before the
    // check, since the stamp may wait on a reconcile while the tab closes (`tab_stamps`).
    service.stamps.stamp(tab).await;
    service.browser.page(tab).await?;
    service
        .privacy
        .make_private(tab, PrivateReason::Host, EpochMillis::now())
        .await?;
    // And again: if the tab closed since the check, its close may have forgotten its flag before
    // this one was set, and a flag left on disk makes the next tab with this id private after a
    // restart (`tab_stamps`).
    if let Err(closed @ PartError::TabNotFound(_)) = service.browser.page(tab).await {
        service.privacy.tab_closed(tab).await;
        return Err(closed.into());
    }
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
