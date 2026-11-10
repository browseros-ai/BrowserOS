use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::browser::{BrowserLink, LinkStatus};

/// Chromium's supervisor's view of Neo. claw-sdk writes its own types instead of depending on
/// claw-api, the desktop contract.
/// - `GET /system/health` answers 200 while the process serves, with the browser link's state.
///   Chromium polls it every second during its 30 s startup grace, then every 30 s, and kills
///   a sidecar that stops answering, so it never waits on the browser.
/// - `GET /system/ready` answers 200 once runs can start (linked and connected now), else 503.
pub(crate) fn system_routes(browser: Arc<BrowserLink>) -> Router {
    Router::new()
        .route("/system/health", get(health))
        .route("/system/ready", get(ready))
        .with_state(browser)
}

/// `{"status":"ok","browser":{"connected":…,"epoch":…,"lastError":…}}`.
#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
    browser: BrowserHealth,
}

/// `{"status":"ready"}`, or `{"status":"starting","browser":{…}}` with the 503.
#[derive(Debug, Serialize)]
struct Ready {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    browser: Option<BrowserHealth>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowserHealth {
    /// Linked and connected now.
    connected: bool,
    /// The CDP connection epoch; `null` before the first connect.
    epoch: Option<u64>,
    /// Why the latest first-connect attempt failed, until one succeeds.
    last_error: Option<String>,
}

async fn health(State(browser): State<Arc<BrowserLink>>) -> Json<Health> {
    Json(Health {
        status: "ok",
        browser: BrowserHealth::from(browser.status()),
    })
}

async fn ready(State(browser): State<Arc<BrowserLink>>) -> (StatusCode, Json<Ready>) {
    if browser.is_ready() {
        let ready = Ready {
            status: "ready",
            browser: None,
        };
        return (StatusCode::OK, Json(ready));
    }
    let starting = Ready {
        status: "starting",
        browser: Some(BrowserHealth::from(browser.status())),
    };
    (StatusCode::SERVICE_UNAVAILABLE, Json(starting))
}

impl From<LinkStatus> for BrowserHealth {
    fn from(status: LinkStatus) -> Self {
        Self {
            connected: status.connected,
            epoch: status.epoch,
            last_error: status.last_error,
        }
    }
}
