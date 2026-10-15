//! Klavis HTTP compatibility: route-owned validation and fixed failure strings.
//! DELETE intentionally consumes JSON, and successful add responses retain the
//! Strata identity/added-server metadata returned by the upstream creation call.

use super::Json;
use super::{
    errors,
    extract::{ZodIssue, ZodJson},
};
use crate::{
    app::AppState,
    klavis::{Klavis, KlavisError},
};
use axum::{
    extract::State,
    response::{IntoResponse, Response},
};
use http::StatusCode;
use std::sync::Arc;

impl axum::extract::FromRef<AppState> for Arc<Klavis> {
    fn from_ref(state: &AppState) -> Self {
        state.klavis.clone()
    }
}
use serde_json::{Value, json};

pub async fn servers() -> Response {
    Json(json!({"servers":Klavis::catalog(),"count":Klavis::catalog().len()})).into_response()
}
pub async fn oauth_urls(State(klavis): State<Arc<Klavis>>) -> Response {
    match klavis.oauth_urls().await {
        Ok(urls) => Json(json!({"oauthUrls":urls.0,"servers":Klavis::catalog().iter().map(|s| s.name).collect::<Vec<_>>()})).into_response(),
        Err(error) => failure(error, "Failed to get OAuth URLs"),
    }
}
pub async fn user_integrations(State(klavis): State<Arc<Klavis>>) -> Response {
    match klavis.user_integrations().await {
        Ok(rows) => Json(json!({"count":rows.len(),"integrations":rows})).into_response(),
        Err(error) => failure(error, "Failed to fetch user integrations"),
    }
}
pub async fn add_server(
    State(klavis): State<Arc<Klavis>>,
    ZodJson(body): ZodJson<Value>,
) -> Response {
    let name = match validate(&body, false) {
        Ok(name) => name,
        Err(response) => return *response,
    };
    if let Some(response) = invalid_name(name) {
        return response;
    }
    match klavis.add_server(name).await {
        Ok(result) => {
            let mut value = json!({"success":true,"serverName":name,"strataId":result.strata_id,"addedServers":result.added_servers});
            if let Some(url) = result.oauth_url {
                value["oauthUrl"] = json!(url);
            }
            if let Some(url) = result.api_key_url {
                value["apiKeyUrl"] = json!(url);
            }
            Json(value).into_response()
        }
        Err(error) => failure(error, "Failed to add server"),
    }
}
pub async fn submit_api_key(
    State(klavis): State<Arc<Klavis>>,
    ZodJson(body): ZodJson<Value>,
) -> Response {
    let name = match validate(&body, true) {
        Ok(name) => name,
        Err(response) => return *response,
    };
    if let Some(response) = invalid_name(name) {
        return response;
    }
    let url = match url::Url::parse(body["apiKeyUrl"].as_str().unwrap_or_default()) {
        Ok(url) => url,
        Err(_) => return errors::zod_400(&[url_issue()]),
    };
    match klavis
        .submit_api_key(name, body["apiKey"].as_str().unwrap_or_default(), &url)
        .await
    {
        Ok(()) => Json(json!({"success":true,"serverName":name})).into_response(),
        Err(error) => failure(error, "Failed to submit API key"),
    }
}
pub async fn remove_server(
    State(klavis): State<Arc<Klavis>>,
    ZodJson(body): ZodJson<Value>,
) -> Response {
    let name = match validate(&body, false) {
        Ok(name) => name,
        Err(response) => return *response,
    };
    if let Some(response) = invalid_name(name) {
        return response;
    }
    match klavis.remove_server(name).await {
        Ok(()) => Json(json!({"success":true,"serverName":name})).into_response(),
        Err(error) => failure(error, "Failed to remove server"),
    }
}
fn invalid_name(name: &str) -> Option<Response> {
    (!Klavis::catalog().iter().any(|s| s.name == name))
        .then(|| errors::error_string(StatusCode::BAD_REQUEST, &format!("Invalid server: {name}")))
}
fn failure(error: KlavisError, message: &str) -> Response {
    tracing::warn!(%error, "Klavis HTTP request failed");
    errors::error_string(StatusCode::INTERNAL_SERVER_ERROR, message)
}
fn url_issue() -> ZodIssue {
    ZodIssue {
        code: "invalid_format".into(),
        path: vec![json!("apiKeyUrl")],
        message: "Invalid URL".into(),
        attributes: json!({"format":"url"}),
    }
}
fn validate(body: &Value, key: bool) -> Result<&str, Box<Response>> {
    let mut issues = vec![];
    if !body.is_object() {
        issues.push(type_issue(Some(body), "object", vec![]));
    } else {
        for field in if key {
            &["serverName", "apiKey", "apiKeyUrl"][..]
        } else {
            &["serverName"][..]
        } {
            let value = body.get(*field);
            match value.and_then(Value::as_str) {
                None => issues.push(type_issue(value, "string", vec![json!(field)])),
                Some(value) if *field == "apiKeyUrl" => {
                    if url::Url::parse(value).is_err() {
                        issues.push(url_issue());
                    }
                }
                Some("") => issues.push(ZodIssue {
                    code: "too_small".into(),
                    path: vec![json!(field)],
                    message: "Too small: expected string to have >=1 characters".into(),
                    attributes: json!({"origin":"string","minimum":1,"inclusive":true}),
                }),
                Some(_) => {}
            }
        }
    }
    if issues.is_empty() {
        Ok(body["serverName"].as_str().unwrap_or_default())
    } else {
        Err(Box::new(errors::zod_400(&issues)))
    }
}
fn type_issue(value: Option<&Value>, expected: &str, path: Vec<Value>) -> ZodIssue {
    let received = match value {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    };
    ZodIssue {
        code: "invalid_type".into(),
        path,
        message: format!("Invalid input: expected {expected}, received {received}"),
        attributes: json!({"expected":expected}),
    }
}
