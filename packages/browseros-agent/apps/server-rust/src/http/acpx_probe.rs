//! `POST /acpx/probe`: initialize + `session/new` against an ACP agent to report its models,
//! reasoning levels and config-option support (`acp_agents::probe_agent`), launched exactly as chat
//! launches it. Port of `api/routes/acpx-probe.ts` + `probeAgent.ts`: the strict zod body, 200 with
//! the probe JSON (probe failures ride inside it), and `500 wrapper_error` when the command cannot
//! even be resolved.

use super::Json;
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use acp_agents::{AgentKind, DEFAULT_PROBE_TIMEOUT, HostEnvironment, ProbeRequest, probe_agent};
use axum::{
    extract::State,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

use super::{errors, extract::ZodJson, zod::Issues};
use crate::AppState;

const MIN_TIMEOUT_MS: f64 = 1_000.0;
const MAX_TIMEOUT_MS: f64 = 120_000.0;

pub async fn probe(State(state): State<AppState>, ZodJson(body): ZodJson<Value>) -> Response {
    let timeout_override = state
        .config
        .env
        .acpx_probe_timeout_ms
        .map(|millis| millis as f64);
    respond(body, state.acp.host(), timeout_override).await
}

/// The route minus axum state, so tests can drive it against a fake agent.
pub async fn respond(
    body: Value,
    host: &HostEnvironment,
    timeout_override_ms: Option<f64>,
) -> Response {
    let body = match ProbeBody::validate(&body) {
        Ok(body) => body,
        Err(issues) => return issues.into_response(),
    };
    // TS `resolveTimeout`: an in-range BROWSEROS_ACPX_PROBE_TIMEOUT_MS beats the request.
    let timeout = timeout_override_ms
        .filter(|millis| (MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(millis))
        .or(body.timeout_ms)
        .map_or(DEFAULT_PROBE_TIMEOUT, |millis| {
            Duration::from_secs_f64(millis / 1000.0)
        });
    let request = ProbeRequest {
        kind: body.kind,
        command: body.command,
        env: body.env.unwrap_or_default(),
        cwd: body.cwd.map(PathBuf::from),
        timeout,
    };
    match probe_agent(request, host).await {
        Ok(result) => Json(result).into_response(),
        Err(error) => {
            tracing::warn!(%error, "ACP probe wrapper crashed");
            errors::probe_wrapper_error(&error.to_string())
        }
    }
}

/// The parsed `probeRequestSchema`, after zod's trims.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeBody {
    pub kind: AgentKind,
    pub command: Option<String>,
    pub env: Option<BTreeMap<String, String>>,
    pub cwd: Option<String>,
    pub timeout_ms: Option<f64>,
}

const KINDS: [&str; 3] = ["claude", "codex", "custom"];
const FIELDS: [&str; 5] = ["type", "command", "env", "cwd", "timeoutMs"];

impl ProbeBody {
    /// Zod 4 `z.object({...}).strict().superRefine(..)`: shape fields in declaration order, then
    /// unrecognized keys, then the refinement. Issue shapes and order are the captured TS ones.
    pub fn validate(body: &Value) -> Result<Self, Issues> {
        let mut issues = Issues::default();
        let Some(object) = body.as_object() else {
            issues.invalid_type("object", Some(body), &[]);
            return Err(issues);
        };
        let kind = match object.get("type").and_then(Value::as_str) {
            Some("claude") => Some(AgentKind::Claude),
            Some("codex") => Some(AgentKind::Codex),
            Some("custom") => Some(AgentKind::Custom),
            _ => {
                issues.push(
                    vec![("code", json!("invalid_value")), ("values", json!(KINDS))],
                    &[json!("type")],
                    "Invalid option: expected one of \"claude\"|\"codex\"|\"custom\"".to_owned(),
                );
                None
            }
        };
        let command = trimmed_string(object.get("command"), "command", &mut issues);
        let env = match object.get("env") {
            None => None,
            Some(Value::Object(entries)) => {
                let mut env = BTreeMap::new();
                for (key, value) in entries {
                    match value.as_str() {
                        Some(text) => {
                            env.insert(key.clone(), text.to_owned());
                        }
                        None => {
                            issues.invalid_type("string", Some(value), &[json!("env"), json!(key)])
                        }
                    }
                }
                Some(env)
            }
            Some(other) => {
                issues.invalid_type("record", Some(other), &[json!("env")]);
                None
            }
        };
        let cwd = trimmed_string(object.get("cwd"), "cwd", &mut issues);
        let timeout_ms = match object.get("timeoutMs") {
            None => None,
            Some(Value::Number(number)) => {
                let value = number.as_f64().unwrap_or(f64::NAN);
                let path = [json!("timeoutMs")];
                if value.fract() != 0.0 || value.abs() > 9_007_199_254_740_991.0 {
                    issues.push(
                        vec![
                            ("expected", json!("int")),
                            ("format", json!("safeint")),
                            ("code", json!("invalid_type")),
                        ],
                        &path,
                        "Invalid input: expected int, received number".to_owned(),
                    );
                } else if value < MIN_TIMEOUT_MS {
                    issues.push(
                        vec![
                            ("origin", json!("number")),
                            ("code", json!("too_small")),
                            ("minimum", json!(1000)),
                            ("inclusive", json!(true)),
                        ],
                        &path,
                        "Too small: expected number to be >=1000".to_owned(),
                    );
                } else if value > MAX_TIMEOUT_MS {
                    issues.push(
                        vec![
                            ("origin", json!("number")),
                            ("code", json!("too_big")),
                            ("maximum", json!(120000)),
                            ("inclusive", json!(true)),
                        ],
                        &path,
                        "Too big: expected number to be <=120000".to_owned(),
                    );
                }
                Some(value)
            }
            Some(other) => {
                issues.invalid_type("number", Some(other), &[json!("timeoutMs")]);
                None
            }
        };
        let unknown: Vec<&String> = object
            .keys()
            .filter(|key| !FIELDS.contains(&key.as_str()))
            .collect();
        if !unknown.is_empty() {
            let quoted: Vec<String> = unknown.iter().map(|key| format!("\"{key}\"")).collect();
            let noun = if unknown.len() == 1 { "key" } else { "keys" };
            issues.push(
                vec![
                    ("code", json!("unrecognized_keys")),
                    ("keys", json!(unknown)),
                ],
                &[],
                format!("Unrecognized {noun}: {}", quoted.join(", ")),
            );
        }
        // The refinement sees the parsed value: a trimmed-empty command is still "missing".
        let command_missing = match object.get("command") {
            None | Some(Value::Null | Value::Bool(false)) => true,
            Some(Value::String(text)) => js_trim(text).is_empty(),
            Some(Value::Number(number)) => number.as_f64() == Some(0.0),
            Some(_) => false,
        };
        if object.get("type").and_then(Value::as_str) == Some("custom") && command_missing {
            issues.custom(
                &[json!("command")],
                "command is required to probe a custom agent",
            );
        }
        match kind {
            Some(kind) if issues.is_empty() => Ok(Self {
                kind,
                command,
                env,
                cwd,
                timeout_ms,
            }),
            _ => Err(issues),
        }
    }
}

/// `z.string().trim().min(1).optional()`.
fn trimmed_string(value: Option<&Value>, field: &str, issues: &mut Issues) -> Option<String> {
    let value = value?;
    let Some(text) = value.as_str() else {
        issues.invalid_type("string", Some(value), &[json!(field)]);
        return None;
    };
    let text = js_trim(text);
    if text.is_empty() {
        issues.push(
            vec![
                ("origin", json!("string")),
                ("code", json!("too_small")),
                ("minimum", json!(1)),
                ("inclusive", json!(true)),
            ],
            &[json!(field)],
            "Too small: expected string to have >=1 characters".to_owned(),
        );
        return None;
    }
    Some(text.to_owned())
}

fn js_trim(value: &str) -> &str {
    value.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}
