//! Host-only idle-cleanup claims. A keep changes only `Service::tab_hold`, never the agent's
//! lock policy. PUT checks the lock route's tab inventory on both sides of the change, with
//! the same reconcile fence as locks. DELETE works even when the tab or browser is gone.

use crate::api_error::ApiError;
use crate::service::ServiceState;
use crate::{query, wire};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use neo_sdk_api as api;
use neo_takeover::PartError;
use std::sync::Arc;

pub(crate) async fn put_keep(
    State(service): State<Arc<ServiceState>>,
    Path(tab): Path<String>,
    body: Bytes,
) -> Result<Json<api::TabKeepResult>, ApiError> {
    let tab = query::tab_id(&tab)?;
    let ttl = wire::keep_request(&body)?;
    // Stamping can wait on a reconcile, so neither existence check may precede it. Keeps name
    // the same tabs as locks, including Neo's blank tab, through the existing page inventory.
    service.stamps.stamp(tab).await;
    service.browser.page(tab).await?;
    let keep = service.keeps.set(tab, ttl);
    // A close may already have cleaned the tab between the first check and the set. Only a
    // successful listing that lacks the tab proves this race; an unavailable browser does not.
    if let Err(closed @ PartError::TabNotFound(_)) = service.browser.page(tab).await {
        service.keeps.tab_closed(tab);
        return Err(closed.into());
    }
    tracing::info!(
        tab_id = tab.0,
        ttl_seconds = ttl.map(|ttl| ttl.as_secs()),
        "tab keep set"
    );
    Ok(Json(api::TabKeepResult {
        tab_id: tab.0,
        keep: Box::new(wire::keep(keep)),
    }))
}

pub(crate) async fn delete_keep(
    State(service): State<Arc<ServiceState>>,
    Path(tab): Path<String>,
) -> Result<Json<api::TabKeepDeleted>, ApiError> {
    let tab = query::tab_id(&tab)?;
    service.keeps.delete(tab);
    tracing::info!(tab_id = tab.0, "tab keep deleted");
    Ok(Json(api::TabKeepDeleted { tab_id: tab.0 }))
}
