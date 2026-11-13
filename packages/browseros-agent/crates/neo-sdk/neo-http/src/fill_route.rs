//! `POST /v1/tabs/{tabId}/fill` and `POST /v1/tabs/{tabId}/fill/check`: the host types its
//! secrets into boxes it names, and nothing else may (protected-fill design, sections 4 and 7).
//!
//! ```text
//!  host ─ token, peer rule (host_service) ─▶ refuse_pages: Origin / Sec-Fetch-Site ─▶ 403
//!       ─▶ tab id ─▶ the app's FillDoor(FillAsk { verb, tab, body bytes })
//!                      neo-server: the body's values become PrivateText, a run of its own,
//!                      one `host.fill` op through the door, the request ledger
//!       ◀─ 200 FillResult | FillCheckResult, or a FillRefusal as the contract's error
//! ```
//!
//! This module owns only who may call and how an answer or a refusal looks on the wire. What a
//! fill does needs the dispatcher and the request ledger, which live in the app, so the app arms
//! the service with it ([`Service::arm_fill`](crate::Service::arm_fill)) before the socket
//! serves, as it arms the takeover lock. The body is handed over unread: the values must become
//! the app's private type at its edge and nowhere else, and this crate never parses, logs or
//! quotes them.

use crate::api_error::ApiError;
use crate::service::ServiceState;
use crate::{query, settings_routes};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::HeaderMap;
use browseros_core::TabId;
use futures_util::future::BoxFuture;
use neo_sdk_api as api;
use std::sync::{Arc, OnceLock};

/// What a fill does, as the app arms it: one call in, its answer or its refusal out. It owns
/// the body from here; a call it never answers leaves the host waiting, so every path answers.
pub type FillDoor =
    Arc<dyn Fn(FillAsk) -> BoxFuture<'static, Result<FillReply, FillRefusal>> + Send + Sync>;

/// One call of either route, as the route hands it to the app. A plain data bag.
pub struct FillAsk {
    /// Which route was called.
    pub verb: FillVerb,
    /// The tab from the path. The app checks it is open.
    pub tab: TabId,
    /// The request body, unread. For a fill it holds the values.
    pub body: Bytes,
}

/// The two routes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillVerb {
    /// `POST /v1/tabs/{tabId}/fill`: type the values.
    Fill,
    /// `POST /v1/tabs/{tabId}/fill/check`: look at the boxes, type nothing.
    Check,
}

/// A 200: what happened to the boxes.
#[derive(Debug)]
pub enum FillReply {
    /// The fill's answer.
    Fill(api::FillResult),
    /// The check's answer.
    Check(api::FillCheckResult),
}

/// A request error, as the contract's status and code. Every message is the app's fixed text:
/// none may quote the body.
#[derive(Debug)]
pub enum FillRefusal {
    /// 400 `invalid_request`: the body does not decode, or breaks a limit.
    Invalid(String),
    /// 404 `tab_not_found`: no open tab has the path's id.
    TabNotFound,
    /// 409 `lock_released`: the `lockId` the fill names is not held on this tab, so the agent
    /// is not locked out as the host believed.
    LockReleased(String),
    /// 503 `browser_unavailable`: the browser has not linked yet.
    BrowserUnavailable(String),
    /// 503 `trace_unavailable`: the trace cannot be read to check a `requestId`.
    TraceUnavailable(String),
    /// 500 `internal_error`: a failure inside Neo, logged with the request id.
    Internal(String),
}

/// The armed door, set once by the app before the socket serves.
#[derive(Default)]
pub(crate) struct FillGate {
    door: OnceLock<FillDoor>,
}

impl FillGate {
    /// Arms the routes. A second arm keeps the first: Neo has one app.
    pub(crate) fn arm(&self, door: FillDoor) {
        if self.door.set(door).is_err() {
            tracing::warn!("the fill routes were armed twice; the first door stays");
        }
    }
}

pub(crate) async fn fill(
    State(service): State<Arc<ServiceState>>,
    Path(tab): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<api::FillResult>, ApiError> {
    match ask(
        &service,
        FillVerb::Fill,
        &tab,
        query.as_deref(),
        &headers,
        body,
    )
    .await?
    {
        FillReply::Fill(answer) => Ok(Json(answer)),
        FillReply::Check(_) => Err(ApiError::internal("a fill was answered as a check")),
    }
}

pub(crate) async fn check(
    State(service): State<Arc<ServiceState>>,
    Path(tab): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<api::FillCheckResult>, ApiError> {
    match ask(
        &service,
        FillVerb::Check,
        &tab,
        query.as_deref(),
        &headers,
        body,
    )
    .await?
    {
        FillReply::Check(answer) => Ok(Json(answer)),
        FillReply::Fill(_) => Err(ApiError::internal("a check was answered as a fill")),
    }
}

async fn ask(
    service: &ServiceState,
    verb: FillVerb,
    tab: &str,
    raw_query: Option<&str>,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<FillReply, ApiError> {
    // First, before the path or the body: a page must learn nothing from this route, not even
    // whether a tab id exists. On the development TCP listener a script could otherwise open the
    // host API's address in a tab and post from there (design section 7, rule 5).
    settings_routes::refuse_pages(headers)?;
    // An address ends up in logs and shell history, so nothing rides in it: as a token in the
    // URL is refused, so is anything a host might put there, a value included.
    if raw_query.is_some() {
        return Err(ApiError::invalid(
            "this route takes no query; send everything in the JSON body",
        ));
    }
    let tab = query::tab_id(tab)?;
    let Some(door) = service.fill.door.get() else {
        // Armed before the socket serves, so only a host API served without the app sees this.
        return Err(ApiError::browser_unavailable(
            "private typing is not ready yet; retry in a few seconds",
        ));
    };
    door(FillAsk { verb, tab, body })
        .await
        .map_err(ApiError::from)
}

impl From<FillRefusal> for ApiError {
    fn from(refusal: FillRefusal) -> Self {
        match refusal {
            FillRefusal::Invalid(message) => Self::invalid(message),
            FillRefusal::TabNotFound => Self::tab_not_found(),
            FillRefusal::LockReleased(message) => Self::lock_released(message),
            FillRefusal::BrowserUnavailable(message) => Self::browser_unavailable(message),
            FillRefusal::TraceUnavailable(message) => Self::trace_unavailable(message),
            FillRefusal::Internal(cause) => Self::internal(cause),
        }
    }
}
