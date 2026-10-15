//! Existing OAuth wire dialect over TokenManager: PKCE uses a 302, device flows return JSON,
//! and token submission parses untyped JSON regardless of Content-Type. Other route groups'
//! zod extractors deliberately do not apply here.

use super::errors::{error_string, text_plain};
use crate::{
    app::AppState,
    oauth::{OAuthProvider, StartFlow, TokenInput, TokenManager},
};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use http::{StatusCode, header};
use serde_json::{Value, json};

pub async fn start(State(state): State<AppState>, Path(provider): Path<String>) -> Response {
    start_with_manager(&state.oauth, &provider).await
}

/// Kept beside the handlers so the fake-provider tests exercise the same response translation.
pub async fn start_with_manager(
    manager: &std::sync::Arc<TokenManager>,
    provider: &str,
) -> Response {
    let provider = match provider.parse::<OAuthProvider>() {
        Ok(provider) => provider,
        Err(error) => return text_plain(StatusCode::BAD_REQUEST, &error),
    };
    match manager.start(provider).await {
        Ok(StartFlow::Device {
            user_code,
            verification_uri,
            expires_in,
        }) => Json(json!({
            "userCode": user_code, "verificationUri": verification_uri, "expiresIn": expires_in,
        }))
        .into_response(),
        Ok(StartFlow::Redirect(url)) => {
            (StatusCode::FOUND, [(header::LOCATION, url.as_str())]).into_response()
        }
        Err(error) => {
            let message = error.to_string();
            let code = if message.contains("callback port") {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            error_string(code, &message)
        }
    }
}

pub async fn store_token(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    body: Bytes,
) -> Response {
    store_token_with_manager(&state.oauth, &provider, &body).await
}

pub async fn store_token_with_manager(
    manager: &TokenManager,
    provider: &str,
    body: &[u8],
) -> Response {
    let provider = match provider.parse::<OAuthProvider>() {
        Ok(provider) => provider,
        Err(error) => return text_plain(StatusCode::BAD_REQUEST, &error),
    };
    // Hono c.req.json() throws for malformed JSON and null, but primitives/arrays simply
    // have no accessToken property. Check JS truthiness before any SQLite value conversion.
    let body: Value = match serde_json::from_slice(body) {
        Ok(body) if !body.is_null() => body,
        _ => return text_plain(StatusCode::INTERNAL_SERVER_ERROR, "Failed to store token"),
    };
    let access_token = body.get("accessToken").unwrap_or(&Value::Null);
    if !truthy(access_token) {
        return text_plain(StatusCode::BAD_REQUEST, "Missing accessToken");
    }
    let input = token_input(&body);
    match input {
        Ok(input) if manager.store_token(provider, input).await.is_ok() => {
            Json(json!({"ok": true})).into_response()
        }
        _ => text_plain(StatusCode::INTERNAL_SERVER_ERROR, "Failed to store token"),
    }
}

pub async fn status(State(state): State<AppState>, Path(provider): Path<String>) -> Response {
    match state.oauth.try_status(&provider).await {
        Ok(status) => Json(status).into_response(),
        Err(error) => error_string(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

pub async fn logout(State(state): State<AppState>, Path(provider): Path<String>) -> Response {
    match state.oauth.try_logout(&provider).await {
        Ok(()) => Json(json!({"success": true})).into_response(),
        Err(error) => error_string(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|n| n != 0.0),
        Value::String(value) => !value.is_empty(),
        _ => true,
    }
}

fn token_input(body: &Value) -> Result<TokenInput, ()> {
    // Bun SQLite accepts string/number values for TEXT columns, but rejects booleans and
    // structured JSON. Avoid adding schema validation that would turn these 500s into 400s.
    fn sqlite_text(value: &Value) -> Result<String, ()> {
        match value {
            Value::String(s) => Ok(s.clone()),
            Value::Number(n) => Ok(n.to_string()),
            Value::Null => Ok(String::new()),
            _ => Err(()),
        }
    }
    let expires = match body.get("expiresIn").unwrap_or(&Value::Null) {
        Value::Null => 0.0,
        Value::Bool(value) => {
            if *value {
                1.0
            } else {
                0.0
            }
        }
        Value::Number(value) => value.as_f64().ok_or(())?,
        Value::String(value) => {
            if value.trim().is_empty() {
                0.0
            } else {
                value.trim().parse::<f64>().map_err(|_| ())?
            }
        }
        _ => return Err(()),
    };
    Ok(TokenInput {
        access_token: sqlite_text(&body["accessToken"])?,
        refresh_token: Some(sqlite_text(
            body.get("refreshToken").unwrap_or(&Value::Null),
        )?),
        expires_at: Some(if expires == 0.0 {
            0
        } else {
            super::super::oauth::expires_at(expires)
        }),
    })
}
