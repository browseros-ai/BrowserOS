//! `/agents*`: ACP agent CRUD (201 on create; `.strict()` bodies; `customConfig` parsed). Update and
//! delete close the agent's live ACP sessions. Owned by P9.

use axum::response::Response;

pub async fn list() -> Response {
    super::not_implemented()
}

pub async fn create() -> Response {
    super::not_implemented()
}

pub async fn get() -> Response {
    super::not_implemented()
}

pub async fn update() -> Response {
    super::not_implemented()
}

pub async fn delete() -> Response {
    super::not_implemented()
}
