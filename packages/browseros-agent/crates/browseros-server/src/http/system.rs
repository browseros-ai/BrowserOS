//! The browser's supervisor contract: liveness, update readiness, diagnostics and shutdown.
//! No origin check on the reads: Chromium's own C++ calls them with no `Origin`, every 30 s for
//! health and before it swaps in a downloaded server for `/status`. Shutdown is a separate
//! router because it takes the stricter `trust::supervisor_only` (wired in `http::router`).
//! `/health` and `/shutdown` are kept for browsers already shipped and are not in the contract.

use axum::Json;
use axum::extract::State;
use axum::routing::{get, post};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::dto;
use crate::app::App;

/// Health, status and diagnostics, under `trust::loopback_only`.
pub(super) fn routes() -> OpenApiRouter<App> {
    OpenApiRouter::new()
        .routes(routes!(get_health))
        .routes(routes!(get_status))
        .routes(routes!(get_diagnostics))
        .route("/health", get(get_health))
}

/// `/system/shutdown` and the `/shutdown` alias, under `trust::supervisor_only`.
pub(super) fn shutdown_routes() -> OpenApiRouter<App> {
    OpenApiRouter::new()
        .routes(routes!(shutdown))
        .route("/shutdown", post(shutdown))
}

/// Liveness. The browser polls it every 30 s and reads only the status code.
///
/// Also served at `/health` for browsers already shipped.
#[utoipa::path(
    get,
    path = "/system/health",
    tag = "system",
    operation_id = "getHealth",
    responses(
        (status = 200, description = "Alive.", body = dto::Health),
        (status = 403, description = "`Host` does not name loopback.", body = dto::ApiErrorBody),
    )
)]
async fn get_health(State(app): State<App>) -> Json<dto::Health> {
    Json(dto::Health {
        status: dto::StatusOk::Ok,
        cdp_connected: app.browser.connected(),
    })
}

/// Update readiness. The browser swaps in a downloaded server only when `can_update` is true.
#[utoipa::path(
    get,
    path = "/status",
    tag = "system",
    operation_id = "getStatus",
    responses(
        (status = 200, description = "`can_update` is false while any chat run or agent tool call is active.", body = dto::Status),
        (status = 403, description = "`Host` does not name loopback.", body = dto::ApiErrorBody),
    )
)]
async fn get_status(State(app): State<App>) -> Json<dto::Status> {
    Json(dto::Status {
        status: dto::StatusOk::Ok,
        cdp_connected: app.browser.connected(),
        can_update: !app.conversations.busy(),
    })
}

/// Version and operating system, for the diagnostics page.
#[utoipa::path(
    get,
    path = "/system/diagnostics",
    tag = "system",
    operation_id = "getDiagnostics",
    responses(
        (status = 200, description = "Version and operating system, for the diagnostics page.", body = dto::Diagnostics),
        (status = 403, description = "`Host` does not name loopback.", body = dto::ApiErrorBody),
    )
)]
async fn get_diagnostics() -> Json<dto::Diagnostics> {
    let info = os_info::get();
    let os = match info.os_type() {
        os_info::Type::Macos => "macOS".to_owned(),
        os_info::Type::Windows => "Windows".to_owned(),
        other => other.to_string(),
    };
    let version = info.version().to_string();
    Json(dto::Diagnostics {
        version: crate::VERSION.to_owned(),
        os,
        os_version: (version != "Unknown").then_some(version),
    })
}

/// Answer, then exit 0. Also served at `/shutdown` for BrowserOS 0.46.x.
///
/// For the browser itself and native callers only: a request that carries `Origin`, or a
/// `Sec-Fetch-Site` other than `none`, is one a web page sent, and is refused.
#[utoipa::path(
    post,
    path = "/system/shutdown",
    tag = "system",
    operation_id = "shutdown",
    responses(
        (status = 200, description = "Exiting.", body = dto::ShutdownResult),
        (status = 403, description = "A web page sent it (it carries `Origin`, or a `Sec-Fetch-Site` other than `none`), or `Host` does not name loopback.", body = dto::ApiErrorBody),
    )
)]
async fn shutdown(State(app): State<App>) -> Json<dto::ShutdownResult> {
    app.shutdown.request_exit();
    Json(dto::ShutdownResult {
        status: dto::StatusOk::Ok,
    })
}
