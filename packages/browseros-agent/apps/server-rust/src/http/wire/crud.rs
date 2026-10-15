//! Compatibility decoding and storage projections for the CRUD routes. Generated models
//! describe validated requests and known execution types; management reads also preserve
//! future type strings and legacy JSON. Zod rules (trimming, strict objects, UTF-16 lengths,
//! headers, and ordered issues) run before decoding request models.

use crate::http::Json;
use axum::{
    extract::{FromRequest, Request},
    response::{IntoResponse, Response},
};
use http::StatusCode;
use serde::{Serialize, Serializer, de::DeserializeOwned, ser::SerializeMap};
use serde_json::{Value, json};
use server_api::models as api;
use std::collections::HashSet;
use uuid::Uuid;

use crate::db;

/// A generated request after the TS route's validation and normalization. `extract::ZodJson`
/// owns media types and malformed JSON; CRUD supplies the schema rules missing from generated DTOs.
pub struct CrudJson<T>(pub T);

impl<S, T> FromRequest<S> for CrudJson<T>
where
    S: Send + Sync,
    T: CrudRequest + Send,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let super::super::extract::ZodJson(value) =
            super::super::extract::ZodJson::<Value>::from_request(request, state).await?;
        decode(value).map(Self).map_err(IntoResponse::into_response)
    }
}

/// Generated models use JavaScript numbers; the `db` stores represent SQLite integer columns as i64.
/// Convert once here, truncating fractional units toward zero while keeping the surrounding
/// optional/null patch states intact. Integer timestamps are returned as wire numbers again.
pub fn store_integer(value: f64) -> i64 {
    value as i64
}

/// The explicit route rules are deliberately kept outside generated code. Regeneration must
/// never remove validation or silently turn an omitted column into an explicit null.
pub trait CrudRequest: DeserializeOwned {
    fn normalize(value: &mut Value, issues: &mut Issues);
}

/// Box only the rejection path so helpers keep small Result values while carrying an exact
/// already-rendered HTTP response across the validation/projection-to-handler handoff.
pub struct CrudError(Box<Response>);

impl From<Response> for CrudError {
    fn from(response: Response) -> Self {
        Self(Box::new(response))
    }
}

impl IntoResponse for CrudError {
    fn into_response(self) -> Response {
        *self.0
    }
}

pub fn decode<T: CrudRequest>(mut value: Value) -> Result<T, CrudError> {
    let mut issues = Issues::default();
    T::normalize(&mut value, &mut issues);
    if !issues.0.is_empty() {
        return Err(issues.response().into());
    }
    serde_json::from_value(value).map_err(|error| {
        internal_error(&format!(
            "Validated CRUD body did not match generated contract: {error}"
        ))
        .into()
    })
}

/// Zod includes kind-specific metadata in an ordered pretty-printed JSON string. A plain
/// `Value` would reorder its keys and change that string, even though the outer JSON is equal.
struct Issue(Vec<(&'static str, Value)>);

impl Serialize for Issue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

#[derive(Default)]
pub struct Issues(Vec<Issue>);

impl Issues {
    fn push(&mut self, fields: Vec<(&'static str, Value)>, path: &[Value], message: String) {
        let mut fields = fields;
        fields.extend([("path", json!(path)), ("message", json!(message))]);
        self.0.push(Issue(fields));
    }

    fn custom(&mut self, path: &[Value], message: &str) {
        self.0.push(Issue(vec![
            ("code", json!("custom")),
            ("message", json!(message)),
            ("path", json!(path)),
        ]));
    }

    fn invalid_type(&mut self, expected: &str, actual: Option<&Value>, path: &[Value]) {
        let received = match actual {
            None => "undefined",
            Some(Value::Null) => "null",
            Some(Value::Bool(_)) => "boolean",
            Some(Value::Number(_)) => "number",
            Some(Value::String(_)) => "string",
            Some(Value::Array(_)) => "array",
            Some(Value::Object(_)) => "object",
        };
        self.push(
            vec![
                ("expected", json!(expected)),
                ("code", json!("invalid_type")),
            ],
            path,
            format!("Invalid input: expected {expected}, received {received}"),
        );
    }

    fn response(&self) -> Response {
        let message = serde_json::to_string_pretty(&self.0).unwrap_or_default();
        (
            StatusCode::BAD_REQUEST,
            Json(api::ValidationError::new(
                false,
                api::ZodIssueError::new(api::ZodIssueErrorName::ZodError, message),
            )),
        )
            .into_response()
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Text {
        trim: bool,
        min: usize,
        max: Option<usize>,
    },
    Number,
    Boolean,
    Enum(&'static [&'static str]),
    Array(&'static Kind),
    Record(&'static Kind),
    Object(&'static [Field], bool),
    ImportedMessage,
    Any,
    Headers,
}

#[derive(Clone, Copy)]
struct Field {
    name: &'static str,
    kind: Kind,
    optional: bool,
    nullable: bool,
}

const TEXT: Kind = Kind::Text {
    trim: false,
    min: 0,
    max: None,
};
const NONEMPTY: Kind = Kind::Text {
    trim: false,
    min: 1,
    max: None,
};
const TRIMMED: Kind = Kind::Text {
    trim: true,
    min: 1,
    max: None,
};
const AGENT_NAME: Kind = Kind::Text {
    trim: true,
    min: 1,
    max: Some(80),
};
const ICON: Kind = Kind::Text {
    trim: true,
    min: 1,
    max: Some(64),
};

const fn required(name: &'static str, kind: Kind) -> Field {
    Field {
        name,
        kind,
        optional: false,
        nullable: false,
    }
}
const fn optional(name: &'static str, kind: Kind) -> Field {
    Field {
        name,
        kind,
        optional: true,
        nullable: false,
    }
}
const fn nullish(name: &'static str, kind: Kind) -> Field {
    Field {
        name,
        kind,
        optional: true,
        nullable: true,
    }
}

const CUSTOM_CONFIG: &[Field] = &[
    required("command", TRIMMED),
    optional("env", Kind::Record(&TEXT)),
    optional("fullAccessModes", Kind::Array(&TRIMMED)),
    optional("reasoningEffortKey", TRIMMED),
    optional("systemPromptAppend", TRIMMED),
    optional("icon", ICON),
];
const CREATE_AGENT: &[Field] = &[
    required("name", AGENT_NAME),
    required("type", Kind::Enum(&["claude", "codex", "custom"])),
    optional("modelId", TRIMMED),
    optional("reasoningEffort", TRIMMED),
    optional("workingDirectory", TRIMMED),
    optional("customConfig", Kind::Object(CUSTOM_CONFIG, true)),
];
const UPDATE_AGENT: &[Field] = &[
    optional("name", AGENT_NAME),
    nullish("modelId", TRIMMED),
    nullish("reasoningEffort", TRIMMED),
    nullish("workingDirectory", TRIMMED),
    optional("customConfig", Kind::Object(CUSTOM_CONFIG, true)),
];
const IMPORT: &[Field] = &[
    required("messages", Kind::Array(&Kind::ImportedMessage)),
    optional("lastMessagedAt", Kind::Number),
    optional("targetType", Kind::Enum(&["browseros", "claude", "codex"])),
    optional("origin", TEXT),
    optional("agentId", TEXT),
];
const PROVIDER: &[Field] = &[
    required("type", NONEMPTY),
    required("name", NONEMPTY),
    nullish("baseUrl", TEXT),
    nullish("headers", Kind::Headers),
    required("modelId", NONEMPTY),
    optional("supportsImages", Kind::Boolean),
    required("contextWindow", Kind::Number),
    optional("temperature", Kind::Number),
    nullish("apiKey", TEXT),
    nullish("accessKeyId", TEXT),
    nullish("secretAccessKey", TEXT),
    nullish("sessionToken", TEXT),
    nullish("resourceName", TEXT),
    nullish("region", TEXT),
    nullish("reasoningEffort", TEXT),
    nullish("reasoningSummary", TEXT),
    optional("createdAt", Kind::Number),
];
const JOB: &[Field] = &[
    required("name", NONEMPTY),
    required("query", NONEMPTY),
    required("scheduleType", Kind::Enum(&["daily", "hourly", "minutes"])),
    nullish("scheduleTime", TEXT),
    nullish("scheduleInterval", Kind::Number),
    optional("enabled", Kind::Boolean),
    nullish("providerId", TEXT),
    nullish("lastRunAt", Kind::Number),
    optional("createdAt", Kind::Number),
];
const TOOL_CALL: &[Field] = &[
    required("id", TEXT),
    required("name", TEXT),
    required("input", Kind::Any),
    optional("output", Kind::Any),
    optional("error", TEXT),
    required("timestamp", TEXT),
];
const RUN: &[Field] = &[
    required("jobId", NONEMPTY),
    required("status", Kind::Enum(&["running", "completed", "failed"])),
    required("startedAt", Kind::Number),
    nullish("completedAt", Kind::Number),
    nullish("result", TEXT),
    nullish("finalResult", TEXT),
    nullish("executionLog", TEXT),
    nullish("toolCalls", Kind::Array(&Kind::Object(TOOL_CALL, false))),
    nullish("error", TEXT),
    optional("createdAt", Kind::Number),
];

fn object(value: &mut Value, fields: &[Field], strict: bool, path: &[Value], issues: &mut Issues) {
    let Some(source) = value.as_object_mut() else {
        issues.invalid_type("object", Some(value), path);
        return;
    };
    for field in fields {
        let mut field_path = path.to_vec();
        field_path.push(json!(field.name));
        match source.get_mut(field.name) {
            None if !field.optional => {
                if matches!(field.kind, Kind::Any) {
                    issues.push(
                        vec![
                            ("code", json!("invalid_type")),
                            ("expected", json!("nonoptional")),
                        ],
                        &field_path,
                        "Invalid input: expected nonoptional, received undefined".to_owned(),
                    );
                } else {
                    missing(field.kind, &field_path, issues);
                }
            }
            Some(value) if !(field.nullable && value.is_null()) => {
                validate(value, field.kind, &field_path, issues)
            }
            _ => {}
        }
    }
    let extra: Vec<_> = source
        .keys()
        .filter(|name| !fields.iter().any(|field| field.name == name.as_str()))
        .cloned()
        .collect();
    if strict && !extra.is_empty() {
        let names = extra
            .iter()
            .map(|name| serde_json::to_string(name).unwrap_or_default())
            .collect::<Vec<_>>()
            .join(", ");
        issues.push(
            vec![("code", json!("unrecognized_keys")), ("keys", json!(extra))],
            path,
            format!(
                "Unrecognized key{}: {names}",
                if extra.len() == 1 { "" } else { "s" }
            ),
        );
    }
    // z.object strips unknown properties, including nested tool-call properties. Preserve
    // passthrough only for imported messages, whose predicate intentionally accepts any object.
    source.retain(|name, _| fields.iter().any(|field| field.name == name));
}

fn missing(kind: Kind, path: &[Value], issues: &mut Issues) {
    if let Kind::Enum(values) = kind {
        enum_issue(values, path, issues);
    } else {
        issues.invalid_type(expected(kind), None, path);
    }
}

fn expected(kind: Kind) -> &'static str {
    match kind {
        Kind::Text { .. } => "string",
        Kind::Number => "number",
        Kind::Boolean => "boolean",
        Kind::Array(_) => "array",
        Kind::Any => "nonoptional",
        _ => "object",
    }
}

fn enum_issue(values: &[&str], path: &[Value], issues: &mut Issues) {
    issues.push(
        vec![("code", json!("invalid_value")), ("values", json!(values))],
        path,
        format!(
            "Invalid option: expected one of {}",
            values
                .iter()
                .map(|v| format!("\"{v}\""))
                .collect::<Vec<_>>()
                .join("|")
        ),
    );
}

fn validate(value: &mut Value, kind: Kind, path: &[Value], issues: &mut Issues) {
    match kind {
        Kind::Text { trim, min, max } => {
            let Some(text) = value.as_str() else {
                issues.invalid_type("string", Some(value), path);
                return;
            };
            // JS trim includes BOM but excludes Rust's NEL whitespace. JS and zod lengths
            // count UTF-16 code units, so astral symbols count twice.
            let text = if trim {
                text.trim_matches(js_whitespace)
            } else {
                text
            }
            .to_owned();
            let len = text.encode_utf16().count();
            if len < min {
                issues.push(
                    vec![
                        ("origin", json!("string")),
                        ("code", json!("too_small")),
                        ("minimum", json!(min)),
                        ("inclusive", json!(true)),
                    ],
                    path,
                    format!("Too small: expected string to have >={min} characters"),
                );
            }
            if let Some(max) = max.filter(|max| len > *max) {
                issues.push(
                    vec![
                        ("origin", json!("string")),
                        ("code", json!("too_big")),
                        ("maximum", json!(max)),
                        ("inclusive", json!(true)),
                    ],
                    path,
                    format!("Too big: expected string to have <={max} characters"),
                );
            }
            *value = json!(text);
        }
        Kind::Number if value.is_number() => {}
        Kind::Boolean if value.is_boolean() => {}
        Kind::Enum(values) => {
            if !value.as_str().is_some_and(|text| values.contains(&text)) {
                enum_issue(values, path, issues);
            }
        }
        Kind::Array(element) => {
            let Some(values) = value.as_array_mut() else {
                issues.invalid_type("array", Some(value), path);
                return;
            };
            for (index, value) in values.iter_mut().enumerate() {
                let mut path = path.to_vec();
                path.push(json!(index));
                validate(value, *element, &path, issues);
            }
        }
        Kind::Record(element) => {
            let Some(values) = value.as_object_mut() else {
                issues.invalid_type("record", Some(value), path);
                return;
            };
            for (name, value) in values {
                let mut path = path.to_vec();
                path.push(json!(name));
                validate(value, *element, &path, issues);
            }
        }
        Kind::Object(fields, strict) => object(value, fields, strict, path, issues),
        Kind::ImportedMessage => {
            if !value.is_object() && !value.is_array() {
                issues.push(
                    vec![("code", json!("custom"))],
                    path,
                    "Invalid input".to_owned(),
                );
            }
        }
        Kind::Any => {}
        Kind::Headers => headers(value, path, issues),
        _ => issues.invalid_type(expected(kind), Some(value), path),
    }
}

fn js_whitespace(c: char) -> bool {
    matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

const HEADER_NAME_PATTERN: &str = "/^[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\\s\\S])/";
const HEADER_VALUE_PATTERN: &str = "/^[\\t\\x20-\\x7e\\x80-\\xff]*(?![\\s\\S])/";

fn regex_issue(pattern: &str, path: &[Value]) -> Issue {
    Issue(vec![
        ("origin", json!("string")),
        ("code", json!("invalid_format")),
        ("format", json!("regex")),
        ("pattern", json!(pattern)),
        ("path", json!(path)),
        (
            "message",
            json!(format!("Invalid string: must match pattern {pattern}")),
        ),
    ])
}

fn headers(value: &mut Value, path: &[Value], issues: &mut Issues) {
    let Some(values) = value.as_object_mut() else {
        issues.invalid_type("record", Some(value), path);
        return;
    };
    let mut names = HashSet::new();
    let mut duplicates = Vec::new();
    let start = issues.0.len();
    let mut aborted = false;
    for (name, value) in values {
        let mut entry_path = path.to_vec();
        entry_path.push(json!(name));
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
        {
            let issue =
                serde_json::to_value(regex_issue(HEADER_NAME_PATTERN, &[])).unwrap_or(Value::Null);
            issues.push(
                vec![
                    ("code", json!("invalid_key")),
                    ("origin", json!("record")),
                    ("issues", json!([issue])),
                ],
                &entry_path,
                "Invalid key in record".to_owned(),
            );
            aborted = true;
            // z.record does not validate the value when its key is already invalid.
            continue;
        }
        if let Some(text) = value.as_str() {
            if !text
                .chars()
                .all(|c| c == '\t' || matches!(c, '\u{20}'..='\u{7e}' | '\u{80}'..='\u{ff}'))
            {
                issues
                    .0
                    .push(regex_issue(HEADER_VALUE_PATTERN, &entry_path));
            }
        } else {
            issues.invalid_type("string", Some(value), &entry_path);
            aborted = true;
        }
        if !names.insert(name.to_ascii_lowercase()) {
            duplicates.push(entry_path);
        }
    }
    // Zod superRefine still runs after refinements fail, but not after an invalid type/key.
    if !aborted || issues.0.len() == start {
        for path in duplicates {
            issues.custom(&path, "Duplicate header name");
        }
    }
}

macro_rules! request {
    ($model:ty, $fields:expr, $strict:expr) => {
        impl CrudRequest for $model {
            fn normalize(value: &mut Value, issues: &mut Issues) {
                object(value, $fields, $strict, &[], issues);
            }
        }
    };
}
request!(api::UpdateAgentRequest, UPDATE_AGENT, true);
request!(api::ImportConversationRequest, IMPORT, false);
request!(api::UpsertProviderRequest, PROVIDER, false);
request!(
    api::SetDefaultProviderRequest,
    &[required("providerId", NONEMPTY)],
    false
);
request!(api::UpsertScheduledJobRequest, JOB, false);
request!(api::UpsertScheduledJobRunRequest, RUN, false);
request!(api::CustomAcpAgentConfig, CUSTOM_CONFIG, true);

impl CrudRequest for api::CreateAgentRequest {
    fn normalize(value: &mut Value, issues: &mut Issues) {
        let start = issues.0.len();
        object(value, CREATE_AGENT, true, &[], issues);
        // superRefine is skipped for structural errors, but runs after length and strict-key
        // errors. Only invoke it when the discriminator/config have valid structural types.
        let structurally_valid = value.get("name").is_some_and(Value::is_string)
            && value
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|v| ["custom", "claude", "codex"].contains(&v))
            && !issues.0[start..].iter().any(|issue| {
                issue.0.iter().any(|(key, value)| {
                    *key == "code" && (value == "invalid_type" || value == "invalid_value")
                })
            });
        if structurally_valid {
            let custom = value["type"] == "custom";
            let has_config = value.get("customConfig").is_some();
            if custom && !has_config {
                issues.custom(
                    &[json!("customConfig")],
                    "customConfig is required for custom agents",
                );
            }
            if !custom && has_config {
                issues.custom(
                    &[json!("customConfig")],
                    "customConfig is only allowed for custom agents",
                );
            }
        }
    }
}

/// UUID parsing must validate the textual form before `Uuid::parse_str`; the latter accepts
/// simple/braced/URN representations that the shipped zod validator rejects.
pub fn uuid_param(value: &str, name: &str) -> Result<Uuid, CrudError> {
    let bytes = value.as_bytes();
    let nil_or_max = value == "00000000-0000-0000-0000-000000000000"
        || value == "ffffffff-ffff-ffff-ffff-ffffffffffff";
    let regular = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, b)| {
            if [8, 13, 18, 23].contains(&index) {
                *b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
        && matches!(bytes[14], b'1'..=b'8')
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'A' | b'b' | b'B');
    if (nil_or_max || regular)
        && let Ok(uuid) = Uuid::parse_str(value)
    {
        return Ok(uuid);
    }
    let mut issues = Issues::default();
    issues.push(vec![("origin", json!("string")), ("code", json!("invalid_format")), ("format", json!("uuid")),
        ("pattern", json!("/^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$/"))], &[json!(name)], "Invalid UUID".to_owned());
    Err(issues.response().into())
}

pub fn internal_error(message: &str) -> Response {
    let scrubbed = browseros_agent::llm::redact(message);
    error_envelope(&scrubbed)
}

/// Builds the TS unknown-error envelope from text that is already safe for the wire.
fn error_envelope(scrubbed: &str) -> Response {
    let scrubbed = scrubbed.trim_matches(js_whitespace);
    let message = if scrubbed.is_empty() {
        "An unexpected error occurred."
    } else {
        scrubbed
    }
    .to_owned();
    let details = if scrubbed.is_empty() {
        None
    } else {
        let mut length = 0;
        let mut end = scrubbed.len();
        for (index, c) in scrubbed.char_indices() {
            length += c.len_utf16();
            if length > 20_000 {
                end = index;
                break;
            }
        }
        Some(if end < scrubbed.len() {
            format!("{}…", &scrubbed[..end])
        } else {
            scrubbed.to_owned()
        })
    };
    super::super::errors::chat_envelope_500(&browseros_agent::llm::ChatError {
        code: browseros_agent::llm::ChatErrorCode::Unknown,
        title: "Something went wrong".to_owned(),
        message,
        retryable: true,
        provider: None,
        status_code: None,
        docs_url: None,
        retry_after_seconds: None,
        details,
    })
}

pub fn db_error(error: db::DbError) -> Response {
    let message = match error {
        db::DbError::Query(message) => message,
        error => error.to_string(),
    };
    // SeaORM/sqlx adds transport context around SQLite errors. Bun exposes the SQLite message
    // itself; retain that exact message for the two intentionally unhandled FK failures.
    if message.contains("FOREIGN KEY constraint failed") {
        // This fixed SQLite diagnostic contains no request values or credentials. Do not
        // feed it through provider-specific redaction/classification, which may change it.
        error_envelope("FOREIGN KEY constraint failed")
    } else {
        internal_error(&message)
    }
}

pub fn not_found(message: &str) -> Response {
    super::super::errors::error_string(StatusCode::NOT_FOUND, message)
}

pub fn deleted(result: db::DbResult<bool>, message: &str) -> Response {
    match result {
        Ok(true) => Json(api::SuccessResponse::new(true)).into_response(),
        Ok(false) => not_found(message),
        Err(error) => db_error(error),
    }
}

/// `/agents` parses and validates persisted config, omitting malformed legacy values. The
/// `/providers` projection instead returns the untouched string, including invalid JSON.
pub fn agent(row: db::AcpAgentRow) -> api::Agent {
    let id = row.id.to_string();
    let custom_config = stored_agent_config(row.custom_config.as_deref(), &id);
    api::Agent {
        id,
        name: row.name,
        r#type: match row.kind {
            db::AgentKind::Claude => api::AcpAgentType::Claude,
            db::AgentKind::Codex => api::AcpAgentType::Codex,
            db::AgentKind::Custom => api::AcpAgentType::Custom,
        },
        model_id: row.model_id,
        reasoning_effort: row.reasoning_effort,
        working_directory: row
            .working_directory
            .map(|path| path.to_string_lossy().into_owned()),
        custom_config,
        created_at: row.created_at as f64,
        updated_at: row.updated_at as f64,
    }
}

pub fn conversation_summary(row: db::ConversationSummary) -> api::ConversationSummary {
    api::ConversationSummary {
        id: row.id.to_string(),
        last_user_message: row.last_user_message,
        origin: row.origin,
        target_type: match row.target_type {
            db::TargetType::BrowserOs => api::ConversationSummaryTargetType::Browseros,
            db::TargetType::Claude => api::ConversationSummaryTargetType::Claude,
            db::TargetType::Codex => api::ConversationSummaryTargetType::Codex,
            db::TargetType::Custom => api::ConversationSummaryTargetType::Custom,
        },
        agent_id: row.agent_id,
        last_messaged_at: row.last_messaged_at as f64,
        created_at: row.created_at as f64,
        updated_at: row.updated_at as f64,
    }
}

/// Management reads are deliberately open to future stored type strings. Generated DTOs
/// still validate requests/execution, but cannot represent all rows that TS can persist.
pub fn stored_agent(row: db::StoredAgentRow) -> Value {
    let config = stored_agent_config(row.custom_config.as_deref(), &row.id);
    omit_null_fields(json!({
        "id": row.id, "name": row.name, "type": row.kind.as_str(),
        "modelId": row.model_id, "reasoningEffort": row.reasoning_effort,
        "workingDirectory": row.working_directory.map(|path| path.to_string_lossy().into_owned()),
        "customConfig": config, "createdAt": row.created_at as f64, "updatedAt": row.updated_at as f64,
    }))
}

pub fn stored_conversation_summary(row: db::StoredConversationSummary) -> Value {
    omit_null_fields(json!({
        "id": row.id, "lastUserMessage": row.last_user_message, "origin": row.origin,
        "targetType": row.target_type.as_str(), "agentId": row.agent_id,
        "lastMessagedAt": row.last_messaged_at as f64,
        "createdAt": row.created_at as f64, "updatedAt": row.updated_at as f64,
    }))
}

pub fn stored_conversation_detail(row: db::StoredConversationRecord) -> Value {
    let mut summary = stored_conversation_summary(row.summary);
    if let Some(object) = summary.as_object_mut() {
        // TS returns parsed history verbatim, including legacy messages outside today's DTO.
        object.insert("messages".into(), Value::Array(row.messages));
    }
    summary
}

/// SQLite NULL maps to JS undefined in these projections, so JSON omits just the outer field.
/// Nested nulls inside stored config/history remain untouched.
fn omit_null_fields(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.retain(|_, value| !value.is_null());
    }
    value
}

fn stored_agent_config(raw: Option<&str>, id: &str) -> Option<Box<api::CustomAcpAgentConfig>> {
    raw.filter(|raw| !raw.is_empty()).and_then(|raw| {
        let value = match serde_json::from_str(raw) {
            Ok(value) => value,
            Err(_) => {
                tracing::warn!(agent_id = %id, "Ignoring unparseable custom agent config");
                return None;
            }
        };
        match decode::<api::CustomAcpAgentConfig>(value) {
            Ok(config) => Some(Box::new(config)),
            Err(_) => {
                tracing::warn!(agent_id = %id, "Ignoring malformed custom agent config");
                None
            }
        }
    })
}

pub fn provider(row: db::ProviderRow) -> api::Provider {
    api::Provider {
        id: row.id,
        kind: if row.kind == "acp" {
            api::ProviderKind::Acp
        } else {
            api::ProviderKind::Llm
        },
        r#type: row.provider_type,
        name: row.name,
        model_id: row.model_id,
        is_default: row.is_default,
        base_url: row.base_url,
        headers: row.headers.map(|headers| headers.into_iter().collect()),
        supports_images: row.supports_images,
        context_window: row.context_window.map(|value| value as f64),
        temperature: row.temperature,
        api_key: row.api_key,
        access_key_id: row.access_key_id,
        secret_access_key: row.secret_access_key,
        session_token: row.session_token,
        resource_name: row.resource_name,
        region: row.region,
        reasoning_effort: row.reasoning_effort,
        reasoning_summary: row.reasoning_summary,
        working_directory: row.working_directory,
        custom_config: row.custom_config,
        created_at: row.created_at as f64,
        updated_at: row.updated_at as f64,
    }
}

pub fn public_provider(row: db::ProviderRow) -> api::PublicProvider {
    let row = provider(row);
    api::PublicProvider {
        id: row.id,
        kind: match row.kind {
            api::ProviderKind::Llm => api::PublicProviderKind::Llm,
            api::ProviderKind::Acp => api::PublicProviderKind::Acp,
        },
        r#type: row.r#type,
        name: row.name,
        model_id: row.model_id,
        is_default: row.is_default,
        base_url: row.base_url,
        headers: row.headers,
        supports_images: row.supports_images,
        context_window: row.context_window,
        temperature: row.temperature,
        // Do not serialize a raw row and remove fields afterward: explicitly building the
        // generated public model ensures new credential columns cannot accidentally escape.
        has_api_key: has_secret(&row.api_key),
        has_access_key_id: has_secret(&row.access_key_id),
        has_secret_access_key: has_secret(&row.secret_access_key),
        has_session_token: has_secret(&row.session_token),
        resource_name: row.resource_name,
        region: row.region,
        reasoning_effort: row.reasoning_effort,
        reasoning_summary: row.reasoning_summary,
        working_directory: row.working_directory,
        custom_config: row.custom_config,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn has_secret(value: &Option<String>) -> bool {
    value.as_ref().is_some_and(|value| !value.is_empty())
}

pub fn job(row: db::ScheduledJobRow) -> api::ScheduledJob {
    api::ScheduledJob {
        id: row.id,
        name: row.name,
        query: row.query,
        schedule_type: match row.schedule_type.as_str() {
            "hourly" => api::ScheduledJobScheduleType::Hourly,
            "minutes" => api::ScheduledJobScheduleType::Minutes,
            _ => api::ScheduledJobScheduleType::Daily,
        },
        schedule_time: row.schedule_time,
        schedule_interval: row.schedule_interval.map(|value| value as f64),
        enabled: row.enabled,
        provider_id: row.provider_id,
        last_run_at: row.last_run_at.map(|value| value as f64),
        created_at: row.created_at as f64,
        updated_at: row.updated_at as f64,
    }
}

pub fn run(row: db::ScheduledRunRow) -> Result<api::ScheduledJobRun, CrudError> {
    let tool_calls = row
        .tool_calls
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| CrudError::from(internal_error(&error.to_string())))?;
    let status = match row.status.as_str() {
        "running" => api::ScheduledJobRunStatus::Running,
        "completed" => api::ScheduledJobRunStatus::Completed,
        "failed" => api::ScheduledJobRunStatus::Failed,
        _ => return Err(internal_error("Unknown scheduled run status").into()),
    };
    Ok(api::ScheduledJobRun {
        id: row.id,
        job_id: row.job_id,
        status,
        started_at: row.started_at as f64,
        completed_at: row.completed_at.map(|value| value as f64),
        result: row.result,
        final_result: row.final_result,
        execution_log: row.execution_log,
        tool_calls,
        error: row.error,
        created_at: row.created_at as f64,
        updated_at: row.updated_at as f64,
    })
}
