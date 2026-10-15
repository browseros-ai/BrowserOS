//! `/oauth/{provider}/*`: start (200 device JSON or 302 redirect), store token (text/plain
//! errors), status, logout, over `oauth::TokenManager`. Owned by P10.

use axum::response::Response;

pub async fn start() -> Response {
    super::not_implemented()
}

pub async fn store_token() -> Response {
    super::not_implemented()
}

pub async fn status() -> Response {
    super::not_implemented()
}

pub async fn logout() -> Response {
    super::not_implemented()
}
