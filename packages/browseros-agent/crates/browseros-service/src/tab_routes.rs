//! `POST /v1/tabs/{tabId}/private`: the host makes a tab private, durably, before an owner
//! types into it.
//!
//! The route does not check that the tab is open: that takes the browser, and this crate has
//! no browser connection. A flag on a tab that is not open only over-protects. The browser
//! link drops it when it reports the open tabs (`Privacy::retain_tabs`), and until then it
//! covers whichever tab later opens with that id.

use crate::api_error::ApiError;
use crate::service::ServiceState;
use crate::{query, wire};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
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
    service
        .privacy
        .make_private(tab, PrivateReason::Host, EpochMillis::now())
        .await?;
    // A tab that was already private keeps its first record, which is what this reads.
    Ok(Json(wire::privacy(service.privacy.is_private(tab))))
}
