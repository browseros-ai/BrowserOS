//! `/klavis/*`: Klavis/Strata connector management over `klavis::Klavis`. `DELETE
//! /klavis/servers/remove` carries a JSON body. Owned by P12.

use axum::response::Response;

pub async fn servers() -> Response {
    super::not_implemented()
}

pub async fn oauth_urls() -> Response {
    super::not_implemented()
}

pub async fn user_integrations() -> Response {
    super::not_implemented()
}

pub async fn add_server() -> Response {
    super::not_implemented()
}

pub async fn submit_api_key() -> Response {
    super::not_implemented()
}

pub async fn remove_server() -> Response {
    super::not_implemented()
}
