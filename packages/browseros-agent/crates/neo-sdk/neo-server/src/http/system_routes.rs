use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::browser::{BrowserLink, LinkStatus};
use crate::host::{RestoreState, TabHolders};

/// Chromium's supervisor's view of Neo. claw-sdk writes its own types instead of depending on
/// claw-api, the desktop contract.
/// - `GET /system/health` answers 200 while the process serves, with the browser link's state.
///   Chromium polls it every second during its 30 s startup grace, then every 30 s, and kills
///   a sidecar that stops answering, so it never waits on the browser.
/// - `GET /system/ready` answers 200 once runs can start (linked and connected now), else 503.
///
/// `holders` is the sidecar's record of which task holds which tab: health says how its restore
/// at start ended. The Node embedding keeps no record and serves no routes.
pub(crate) fn system_routes(browser: Arc<BrowserLink>, holders: Option<Arc<TabHolders>>) -> Router {
    Router::new()
        .route("/system/health", get(health))
        .route("/system/ready", get(ready))
        .with_state(SystemState { browser, holders })
}

/// What the system routes read: the browser link, and the tab-holders record when there is one.
#[derive(Clone)]
struct SystemState {
    browser: Arc<BrowserLink>,
    holders: Option<Arc<TabHolders>>,
}

/// `{"status":"ok","browser":{"connected":…,"epoch":…,"lastError":…},"tabHolders":{"state":…}}`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Health {
    status: &'static str,
    browser: BrowserHealth,
    /// How the start's restore of which task holds which tab ended; absent with no record.
    #[serde(skip_serializing_if = "Option::is_none")]
    tab_holders: Option<TabHoldersHealth>,
}

/// `{"state":"pending"|"restored"|"unreadable"|"unlisted"}`.
#[derive(Debug, Serialize)]
struct TabHoldersHealth {
    state: RestoreState,
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

async fn health(State(system): State<SystemState>) -> Json<Health> {
    Json(Health {
        status: "ok",
        browser: BrowserHealth::from(system.browser.status()),
        tab_holders: system.holders.as_ref().map(|holders| TabHoldersHealth {
            state: holders.state(),
        }),
    })
}

async fn ready(
    State(SystemState { browser, .. }): State<SystemState>,
) -> (StatusCode, Json<Ready>) {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::system_routes;
    use crate::host::TabHolders;
    use crate::test_rig::{TestError, TestRig, on_neo_runtime, within};

    async fn health(routes: Router) -> Result<Value, TestError> {
        let response = routes
            .oneshot(Request::get("/system/health").body(Body::empty())?)
            .await?;
        Ok(serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024).await?,
        )?)
    }

    #[test]
    fn health_says_how_the_starts_restore_of_tab_holders_ended() -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let folder = tempfile::tempdir()?;
            let holders = TabHolders::open(folder.path()).await;
            let routes = system_routes(Arc::clone(&rig.link), Some(Arc::clone(&holders)));
            assert_eq!(
                health(routes.clone()).await?["tabHolders"],
                json!({ "state": "pending" }),
                "before the restore ended"
            );
            within(
                Duration::from_secs(5),
                holders.restore(&rig.link, &rig.tabs),
            )
            .await?;
            assert_eq!(
                health(routes).await?["tabHolders"],
                json!({ "state": "restored" })
            );
            let without = health(system_routes(Arc::clone(&rig.link), None)).await?;
            assert!(
                without.get("tabHolders").is_none(),
                "no record, no field: {without}"
            );
            Ok(())
        })
    }
}
