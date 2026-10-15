//! Input validation for static (local) tools, the stand-in for the zod schemas the TS tools are
//! declared with. The AI SDK validates a static tool's parsed input before executing it and turns
//! a failure into `tool-input-error` (`parse-tool-call.ts` `doParseToolCall`); MCP tools carry a
//! plain JSON schema and are never validated. Only the JSON Schema subset our own tools declare is
//! understood: an object of string/number/integer/boolean properties with `required`, `enum`,
//! `exclusiveMinimum`, `minimum` and `maximum`. Unknown keys are stripped, as zod does.

use serde_json::{Map, Value, json};

/// Returns zod's parsed value (declared properties only, in schema order) or zod-shaped issues
/// rendered the way `TypeValidationError` prints them.
pub(crate) fn validate_static_input(schema: &Value, input: Value) -> Result<Value, String> {
    let mut issues = Vec::new();
    let Value::Object(mut object) = input else {
        issues.push(invalid_type("object", &input, &[]));
        return Err(render(&input, &issues));
    };
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|keys| keys.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let original = Value::Object(object.clone());
    let mut parsed = Map::new();
    for (key, property) in &properties {
        match object.remove(key) {
            Some(value) => {
                if let Some(issue) = check_property(property, &value, key) {
                    issues.push(issue);
                } else {
                    parsed.insert(key.clone(), value);
                }
            }
            None if required.contains(&key.as_str()) => {
                let expected = property
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("any");
                issues
                    .push(invalid_type(expected, &Value::Null, &[key]).with_received("undefined"));
            }
            None => {}
        }
    }
    if issues.is_empty() {
        Ok(Value::Object(parsed))
    } else {
        Err(render(&original, &issues))
    }
}

struct Issue(Value);

impl Issue {
    fn with_received(mut self, received: &str) -> Self {
        let expected = self
            .0
            .get("expected")
            .and_then(Value::as_str)
            .unwrap_or("any")
            .to_owned();
        if let Some(message) = self.0.get_mut("message") {
            *message = Value::from(format!(
                "Invalid input: expected {expected}, received {received}"
            ));
        }
        self
    }
}

fn check_property(property: &Value, value: &Value, key: &str) -> Option<Issue> {
    let expected = property
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("any");
    let type_ok = match expected {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_f64().is_some_and(|n| n.fract() == 0.0),
        "boolean" => value.is_boolean(),
        _ => true,
    };
    if !type_ok {
        return Some(if expected == "integer" && value.is_number() {
            Issue(json!({
                "expected": "int",
                "format": "safeint",
                "code": "invalid_type",
                "path": [key],
                "message": "Invalid input: expected int, received number",
            }))
        } else {
            invalid_type(expected, value, &[key])
        });
    }
    if let Some(options) = property.get("enum").and_then(Value::as_array)
        && !options.contains(value)
    {
        let listed = options
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("|");
        return Some(Issue(json!({
            "code": "invalid_value",
            "values": options,
            "path": [key],
            "message": format!("Invalid option: expected one of {listed}"),
        })));
    }
    let number = value.as_f64()?;
    if let Some(min) = property.get("exclusiveMinimum").and_then(Value::as_f64)
        && number <= min
    {
        return Some(too_small(min, false, key));
    }
    if let Some(min) = property.get("minimum").and_then(Value::as_f64)
        && number < min
    {
        return Some(too_small(min, true, key));
    }
    if let Some(max) = property.get("maximum").and_then(Value::as_f64)
        && number > max
    {
        return Some(Issue(json!({
            "origin": "number",
            "code": "too_big",
            "maximum": max,
            "inclusive": true,
            "path": [key],
            "message": format!("Too big: expected number to be <={max}"),
        })));
    }
    None
}

fn too_small(min: f64, inclusive: bool, key: &str) -> Issue {
    let op = if inclusive { ">=" } else { ">" };
    Issue(json!({
        "origin": "number",
        "code": "too_small",
        "minimum": min,
        "inclusive": inclusive,
        "path": [key],
        "message": format!("Too small: expected number to be {op}{min}"),
    }))
}

fn invalid_type(expected: &str, value: &Value, path: &[&str]) -> Issue {
    Issue(json!({
        "expected": expected,
        "code": "invalid_type",
        "path": path,
        "message": format!("Invalid input: expected {expected}, received {}", js_type(value)),
    }))
}

fn js_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// `TypeValidationError`'s message with the zod issues pretty-printed, as `getErrorMessage(cause)`
/// renders a `ZodError` (its `message` is the 2-space JSON of its issues).
fn render(value: &Value, issues: &[Issue]) -> String {
    let issues = Value::Array(issues.iter().map(|issue| issue.0.clone()).collect());
    let pretty = serde_json::to_string_pretty(&issues).unwrap_or_else(|_| issues.to_string());
    format!(
        "AI_TypeValidationError: Type validation failed: Value: {value}.\nError message: {pretty}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "limit": {"type": "integer", "exclusiveMinimum": 0, "maximum": 9007199254740991_i64},
                "kind": {"type": "string", "enum": ["daily", "hourly"]},
            },
            "required": ["path"],
            "additionalProperties": false,
        })
    }

    #[test]
    fn strips_unknown_keys_and_keeps_declared_ones() {
        let parsed = validate_static_input(&schema(), json!({"path": "a", "extra": 1, "limit": 3}));
        assert_eq!(parsed.ok(), Some(json!({"path": "a", "limit": 3})));
    }

    #[test]
    fn rejects_missing_required_wrong_type_enum_and_bounds() {
        for bad in [
            json!({}),
            json!({"path": 1}),
            json!({"path": "a", "limit": 0}),
            json!({"path": "a", "limit": 1.5}),
            json!({"path": "a", "kind": "weekly"}),
            json!({"path": null}),
            json!("not an object"),
        ] {
            let error = validate_static_input(&schema(), bad.clone());
            assert!(
                error
                    .as_ref()
                    .is_err_and(|message| message.starts_with("AI_TypeValidationError")),
                "{bad} → {error:?}"
            );
        }
    }
}
