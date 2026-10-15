//! `/providers*`: LLM provider CRUD and the default provider. `PUT /providers/{id}` returns raw
//! secrets and `customConfig` as a raw string (TS parity, via `http::wire`). Owned by P9.

use axum::response::Response;

pub async fn list() -> Response {
    super::not_implemented()
}

pub async fn get_default() -> Response {
    super::not_implemented()
}

pub async fn set_default() -> Response {
    super::not_implemented()
}

pub async fn get() -> Response {
    super::not_implemented()
}

pub async fn upsert() -> Response {
    super::not_implemented()
}

pub async fn delete() -> Response {
    super::not_implemented()
}
