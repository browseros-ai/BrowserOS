//! zod 4.6.5's issue vocabulary, as `@hono/zod-validator` answers a failed body:
//! `400 {"success":false,"error":{"name":"ZodError","message":<pretty JSON of the issues>}}`.
//! The issues travel inside that string, so each issue's key order is wire-visible: issues are
//! ordered JSON objects (this crate enables serde_json's `preserve_order`) built in zod's own
//! insertion order. Routes keep their own schema walkers and emit issues through this module.

use super::errors;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use uuid::Uuid;

/// zod 4's `z.uuid()` expression. Its lowercase-only max UUID alternative is intentional.
pub const UUID_PATTERN: &str = "^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$";

/// zod's `received` word in an `invalid_type` message: `typeof`, with `null` and `array` split out.
pub fn received(value: Option<&Value>) -> &'static str {
    match value {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

/// `z.uuid()`: the hyphenated form with an RFC 9562 version (1–8) and variant, or the nil/max
/// UUID. `Uuid::parse_str` alone would also accept the simple, braced and URN forms zod rejects.
pub fn parse_uuid(value: &str) -> Option<Uuid> {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return None;
    }
    let canonical = bytes.iter().enumerate().all(|(i, byte)| {
        if [8, 13, 18, 23].contains(&i) {
            *byte == b'-'
        } else {
            byte.is_ascii_hexdigit()
        }
    });
    let variant = (b'1'..=b'8').contains(&bytes[14])
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b' | b'A' | b'B');
    (canonical
        && (variant
            || value == "00000000-0000-0000-0000-000000000000"
            || value == "ffffffff-ffff-ffff-ffff-ffffffffffff"))
        .then(|| Uuid::parse_str(value).ok())
        .flatten()
}

/// The issue a failed `z.uuid()` reports.
pub fn uuid_issue(path: &[Value]) -> Value {
    ordered(vec![
        ("origin", json!("string")),
        ("code", json!("invalid_format")),
        ("format", json!("uuid")),
        ("pattern", json!(format!("/{UUID_PATTERN}/"))),
        ("path", json!(path)),
        ("message", json!("Invalid UUID")),
    ])
}

/// The issues of one validation, in report order; answers as the zod 400.
#[derive(Debug, Default)]
pub struct Issues(Vec<Value>);

impl Issues {
    /// An issue whose kind-specific `fields` come first, then `path` and `message` (the order of
    /// every issue kind but `custom`).
    pub fn push(
        &mut self,
        mut fields: Vec<(&'static str, Value)>,
        path: &[Value],
        message: String,
    ) {
        fields.extend([("path", json!(path)), ("message", json!(message))]);
        self.0.push(ordered(fields));
    }

    /// A refinement issue. zod puts `message` before `path` for `custom`, unlike every other kind.
    pub fn custom(&mut self, path: &[Value], message: &str) {
        self.0.push(ordered(vec![
            ("code", json!("custom")),
            ("message", json!(message)),
            ("path", json!(path)),
        ]));
    }

    pub fn invalid_type(&mut self, expected: &str, actual: Option<&Value>, path: &[Value]) {
        self.push(
            vec![
                ("expected", json!(expected)),
                ("code", json!("invalid_type")),
            ],
            path,
            format!(
                "Invalid input: expected {expected}, received {}",
                received(actual)
            ),
        );
    }

    /// An issue already built in zod's key order (e.g. [`uuid_issue`]).
    pub fn push_issue(&mut self, issue: Value) {
        self.0.push(issue);
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The issues reported since `start` (an earlier [`Self::len`]), for zod's rules that skip
    /// refinements once a structural issue was reported.
    pub fn since(&self, start: usize) -> &[Value] {
        self.0.get(start..).unwrap_or_default()
    }
}

impl IntoResponse for Issues {
    fn into_response(self) -> Response {
        errors::zod_issues_400(&self.0)
    }
}

fn ordered(fields: Vec<(&'static str, Value)>) -> Value {
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<Map<_, _>>(),
    )
}
