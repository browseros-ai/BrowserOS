//! `GET /v1/metrics`: Neo's metrics as Prometheus text exposition 0.0.4, the pull door of its
//! telemetry, for a scraper that holds the host token.
//!
//! ```text
//!  scraper ─ bearer ─▶ host_service ─▶ router ─▶ get_metrics ─▶ MetricsText (the app's closure)
//!                                                                 └▶ its telemetry, read now
//! ```
//!
//! The text is whatever the app's [`MetricsText`](crate::MetricsText) renders at each scrape:
//! the same meter provider Neo pushes OTLP from, so a scrape and a push agree, and this crate
//! names no OpenTelemetry crate. The route only reads and takes no parameter. It is on the host
//! listener whenever that is, behind the host auth like every `/v1` route, and never on the
//! agent's port, so a script cannot read it.

use crate::service::ServiceState;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// Prometheus text exposition 0.0.4, the content type every Prometheus scraper accepts. Fixed:
/// the route negotiates no other format.
const PROMETHEUS_TEXT: &str = "text/plain; version=0.0.4; charset=utf-8";

pub(crate) async fn get_metrics(State(service): State<Arc<ServiceState>>) -> Response {
    // On the request's own task: the closure reads in-memory state and never waits on I/O.
    let text = (service.metrics)();
    (
        [(CONTENT_TYPE, HeaderValue::from_static(PROMETHEUS_TEXT))],
        text,
    )
        .into_response()
}
