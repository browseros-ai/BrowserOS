//! `/conversations*`: list, detail, legacy import (`PUT`, no in-repo caller but shipped extensions may
//! use it), delete. Owned by P9.

use axum::response::Response;

pub async fn list() -> Response {
    super::not_implemented()
}

pub async fn get() -> Response {
    super::not_implemented()
}

pub async fn import() -> Response {
    super::not_implemented()
}

pub async fn delete() -> Response {
    super::not_implemented()
}
