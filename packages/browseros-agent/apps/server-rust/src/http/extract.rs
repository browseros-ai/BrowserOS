//! Input compatibility belongs at this seam: Hono treats non-JSON bodies as {},
//! malformed JSON escapes as a 500, and route-owned schemas supply full Zod issues.
//! Keeping validation explicit avoids pretending serde enforces Zod defaults/constraints.

use axum::{
    body::to_bytes,
    extract::{FromRequest, FromRequestParts, Path, Request},
    response::Response,
};
use http::request::Parts;
use serde::Serialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::LazyLock};
use uuid::Uuid;

/// Consumes the body once, then delegates TS schema semantics to the route's T.
#[derive(Debug, Clone)]
pub struct ZodJson<T>(pub T);

/// Route bodies implement their TS schema here, including transforms, defaults,
/// stripping and strictness. Value is the raw JSON adapter for hand-written schemas.
pub trait ZodSchema: Sized {
    fn validate(value: Value) -> Result<Self, Vec<ZodIssue>>;
}
impl ZodSchema for Value {
    fn validate(value: Value) -> Result<Self, Vec<ZodIssue>> {
        Ok(value)
    }
}
impl<T: ZodSchema + Send, S: Send + Sync> FromRequest<S> for ZodJson<T> {
    type Rejection = Response;
    async fn from_request(request: Request, _: &S) -> Result<Self, Self::Rejection> {
        let content_type = request
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        // Hono's MIME expression is narrower than a generic +json suffix check:
        // whitespace before ';' and digits in a vendor subtype are not accepted.
        static JSON_TYPE: LazyLock<Result<regex::Regex, regex::Error>> = LazyLock::new(|| {
            regex::Regex::new(r"(?i)^application/([a-z\-.]+\+)?json(;\s*[a-zA-Z0-9\-]+=([^;]+))*$")
        });
        let json_type = JSON_TYPE
            .as_ref()
            .is_ok_and(|pattern| pattern.is_match(content_type));
        let value = if json_type {
            // Hono has no application-level JSON size limit. MCP owns its SDK limit separately.
            let bytes = to_bytes(request.into_body(), usize::MAX)
                .await
                .map_err(|_| super::errors::malformed_json())?;
            serde_json::from_slice(&bytes).map_err(|_| super::errors::malformed_json())?
        } else {
            json!({})
        };
        T::validate(value)
            .map(Self)
            .map_err(|issues| super::errors::zod_400(&issues))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZodUuid(pub Uuid);
impl ZodUuid {
    pub fn parse(value: &str) -> Option<Self> {
        super::zod::parse_uuid(value).map(Self)
    }
}
impl<S: Send + Sync> FromRequestParts<S> for ZodUuid {
    type Rejection = Response;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(params) = Path::<BTreeMap<String, String>>::from_request_parts(parts, state)
            .await
            .map_err(|_| {
                super::errors::error_string(http::StatusCode::BAD_REQUEST, "Invalid path parameter")
            })?;
        let (name, value) = params.iter().next().ok_or_else(|| {
            super::errors::error_string(http::StatusCode::BAD_REQUEST, "Missing path parameter")
        })?;
        Self::parse(value)
            .ok_or_else(|| super::errors::zod_issues_400(&[super::zod::uuid_issue(&[json!(name)])]))
    }
}

/// Zod carries schema-specific fields before path/message (expected, minimum,
/// pattern, union errors, etc.). Preserve them; a three-field issue loses wire data.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ZodIssue {
    pub code: String,
    pub path: Vec<Value>,
    pub message: String,
    #[serde(flatten)]
    pub attributes: Value,
}
impl ZodIssue {
    pub fn new(code: impl Into<String>, path: Vec<Value>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            path,
            message: message.into(),
            attributes: json!({}),
        }
    }
    pub fn wire_value(&self) -> Value {
        // Record-key and custom-header issues (the LLM-tool routes) carry these codes. Zod
        // inserts origin after code for invalid_key, and message before path for a custom issue;
        // those orders remain observable inside the JSON-stringified error message.
        if self.code == "invalid_key" || self.code == "custom" {
            let mut issue = serde_json::Map::new();
            issue.insert("code".into(), json!(self.code));
            if let Some(attributes) = self.attributes.as_object() {
                issue.extend(attributes.clone());
            }
            if self.code == "custom" {
                issue.insert("message".into(), json!(self.message));
                issue.insert("path".into(), json!(self.path));
            } else {
                issue.insert("path".into(), json!(self.path));
                issue.insert("message".into(), json!(self.message));
            }
            return Value::Object(issue);
        }
        let mut issue = serde_json::Map::new();
        // Zod's field insertion order is visible inside its JSON-stringified message.
        if let Some(origin) = self.attributes.get("origin") {
            issue.insert("origin".into(), origin.clone());
        }
        if let Some(expected) = self.attributes.get("expected") {
            issue.insert("expected".into(), expected.clone());
        }
        issue.insert("code".into(), json!(self.code));
        if let Some(attributes) = self.attributes.as_object() {
            for (key, value) in attributes {
                if key != "origin" && key != "expected" {
                    issue.insert(key.clone(), value.clone());
                }
            }
        }
        issue.insert("path".into(), json!(self.path));
        issue.insert("message".into(), json!(self.message));
        Value::Object(issue)
    }
}
