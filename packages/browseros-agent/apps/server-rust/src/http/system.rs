//! `/system/*`, `/health`, `/status`, `/shutdown`: health (`cdpConnected`), OTA readiness (`can_update`),
//! diagnostics (memoized OS version) and the orderly shutdown (200, then exit 0). Owned by P3.

use axum::response::Response;

/// `GET /system/health` and the `/health` alias.
pub async fn health() -> Response {
    super::not_implemented()
}

/// `GET /status`: health plus `can_update` from `ServerActivity`.
pub async fn status() -> Response {
    super::not_implemented()
}

pub async fn diagnostics() -> Response {
    super::not_implemented()
}

/// `POST /system/shutdown` and the `/shutdown` alias.
pub async fn shutdown() -> Response {
    super::not_implemented()
}
