//! Agent discovery for `POST /acpx/probe` (port of `probeAgent.ts` over npm `acp-probe@0.0.2`):
//! launch exactly as chat would, `initialize` + `session/new` with no MCP servers and no prompt
//! (no token cost), detect `session/set_config_option` support, read models and reasoning levels,
//! tear down. The response JSON is the TS route's, field for field.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use acp_client::{
    AcpError, AgentConnection, ClientOptions, PermissionDecision, PermissionPolicy, SessionParams,
    schema::{
        RequestPermissionRequest, SessionConfigKind, SessionConfigOption,
        SessionConfigOptionCategory, SessionConfigSelectOption, SessionConfigSelectOptions,
    },
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::{
    definition::{AgentKind, CustomAgentConfig},
    launch::{HostEnvironment, LaunchError, resolve_launch},
};

/// Cold adapter downloads can take two minutes on slow networks (TS default and maximum).
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(120);
const MIN_PROBE_TIMEOUT_MS: f64 = 1_000.0;
const MAX_PROBE_TIMEOUT_MS: f64 = 120_000.0;
const CONFIG_POKE_TIMEOUT: Duration = Duration::from_secs(5);
const SESSION_CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
const TEARDOWN_GRACE: Duration = Duration::from_secs(1);

pub struct ProbeRequest {
    pub kind: AgentKind,
    /// Full command line of a not-yet-saved custom agent; when present it is launched as a custom
    /// command whatever `kind` says (TS launcher rule).
    pub command: Option<String>,
    pub env: BTreeMap<String, String>,
    /// Defaults to the user's home: the server's own cwd is the read-only app bundle when packaged.
    pub cwd: Option<PathBuf>,
    /// `BROWSEROS_ACPX_PROBE_TIMEOUT_MS` in the host environment (1000–120000) overrides this.
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeModel {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReasoning {
    pub values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeErrorBody {
    /// acp-probe codes: `spawn_failed`, `initialize_timeout`, `session_new_timeout`,
    /// `agent_crashed`, `protocol_mismatch`, `unknown`.
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acp_error_code: Option<i32>,
}

/// `reasoning` and `agentInfo` serialize as `null` when absent; `error` is omitted.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub models: Vec<ProbeModel>,
    pub reasoning: Option<ProbeReasoning>,
    pub supports_config_option: bool,
    pub agent_info: Option<Value>,
    pub protocol_version: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProbeErrorBody>,
}

impl ProbeResult {
    fn failed(error: ProbeErrorBody) -> Self {
        Self {
            models: vec![],
            reasoning: None,
            supports_config_option: false,
            agent_info: None,
            protocol_version: 0,
            error: Some(error),
        }
    }
}

/// acp-probe answers every permission request with Cancelled.
struct CancelAll;
impl PermissionPolicy for CancelAll {
    fn decide(&self, _: &RequestPermissionRequest) -> BoxFuture<'static, PermissionDecision> {
        Box::pin(async { PermissionDecision::Cancel })
    }
}

/// Probe failures are part of the result (HTTP 200 with `error`), as in TS.
///
/// DECIDED: return `Err(LaunchError)` when the command cannot be resolved (the spike signature
/// returned a bare `ProbeResult`). WHY: TS throws there and the route answers
/// `500 {"error":{"code":"wrapper_error",…}}`; the handler needs to tell the two apart.
pub async fn probe_agent(
    request: ProbeRequest,
    host: &HostEnvironment,
) -> Result<ProbeResult, LaunchError> {
    let timeout = env_timeout(host).unwrap_or(request.timeout);
    let cwd = request
        .cwd
        .as_ref()
        .and_then(|cwd| cwd.to_str())
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .map(PathBuf::from)
        .or_else(|| request.cwd.clone().filter(|cwd| cwd.to_str().is_none()))
        .or_else(|| host.inherited_env.get("HOME").map(PathBuf::from))
        .or_else(std::env::home_dir)
        .unwrap_or_else(|| host.browseros_dir.clone());
    let custom = request.command.map(|command| CustomAgentConfig {
        command,
        ..CustomAgentConfig::default()
    });
    let (kind, env, launch_host, launch_cwd) =
        (request.kind, request.env, host.clone(), cwd.clone());
    let launch = tokio::task::spawn_blocking(move || {
        resolve_launch(kind, custom.as_ref(), &env, &launch_cwd, &launch_host)
    })
    .await
    .unwrap_or(Err(LaunchError::MissingCommand))?;
    tracing::debug!(kind = kind.as_str(), source = ?launch.source, "ACP probe using launcher-resolved command");
    Ok(run_probe(launch.spec, cwd, timeout).await)
}

fn env_timeout(host: &HostEnvironment) -> Option<Duration> {
    let millis: f64 = host
        .inherited_env
        .get("BROWSEROS_ACPX_PROBE_TIMEOUT_MS")?
        .trim()
        .parse()
        .ok()?;
    (millis.is_finite() && (MIN_PROBE_TIMEOUT_MS..=MAX_PROBE_TIMEOUT_MS).contains(&millis))
        .then(|| Duration::from_secs_f64(millis / 1000.0))
}

async fn run_probe(spec: acp_client::LaunchSpec, cwd: PathBuf, timeout: Duration) -> ProbeResult {
    let deadline = Instant::now() + timeout;
    let remaining = || deadline.saturating_duration_since(Instant::now());
    let options = ClientOptions {
        client_name: "acp-probe".into(),
        client_version: env!("CARGO_PKG_VERSION").into(),
        control_timeout: remaining(),
        stderr_tail_lines: 20,
    };
    let conn = match AgentConnection::connect(spec, options, Arc::new(CancelAll)).await {
        Ok(conn) => conn,
        Err(error) => return ProbeResult::failed(probe_error(error, Phase::Initialize)),
    };
    let init = conn.initialize_response();
    let protocol_version = serde_json::to_value(init.protocol_version)
        .ok()
        .and_then(|value| value.as_u64())
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or(1);
    let agent_info = init.agent_info.as_ref().and_then(|info| {
        let info = serde_json::to_value(info).ok()?;
        let name = info.get("name")?.as_str().filter(|name| !name.is_empty())?;
        let mut normalized = json!({ "name": name });
        for key in ["title", "version"] {
            if let Some(value) = info.get(key).and_then(Value::as_str) {
                normalized[key] = json!(value);
            }
        }
        Some(normalized)
    });
    let params = SessionParams {
        cwd,
        mcp_servers: vec![],
        meta: None,
    };
    let session = match tokio::time::timeout(remaining(), conn.new_session(params)).await {
        Ok(Ok(session)) => session,
        Ok(Err(error)) => {
            conn.shutdown(TEARDOWN_GRACE).await;
            return ProbeResult::failed(probe_error(error, Phase::SessionNew));
        }
        Err(_) => {
            conn.shutdown(TEARDOWN_GRACE).await;
            return ProbeResult::failed(ProbeErrorBody {
                code: "session_new_timeout".into(),
                message: format!("deadline ({}ms) exceeded", timeout.as_millis()),
                acp_error_code: None,
            });
        }
    };
    let options = session.config_options().to_vec();
    // acp-probe pokes the first option with its current value: success proves the method exists.
    let mut supports_config_option = false;
    if let Some(SessionConfigOption {
        id,
        kind: SessionConfigKind::Select(select),
        ..
    }) = options.first()
    {
        let poke = session.set_config_option(&id.0, &select.current_value.0);
        supports_config_option = matches!(
            tokio::time::timeout(CONFIG_POKE_TIMEOUT.min(remaining()), poke).await,
            Ok(Ok(_))
        );
    }
    if conn.features().close_session {
        let _ = tokio::time::timeout(SESSION_CLOSE_TIMEOUT, session.close()).await;
    }
    drop(session);
    conn.shutdown(TEARDOWN_GRACE).await;
    ProbeResult {
        models: models(&options),
        reasoning: reasoning(&options),
        supports_config_option,
        agent_info,
        protocol_version,
        error: None,
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Initialize,
    SessionNew,
}

fn probe_error(error: AcpError, phase: Phase) -> ProbeErrorBody {
    let body = |code: &str, message: String, acp_error_code| ProbeErrorBody {
        code: code.into(),
        message,
        acp_error_code,
    };
    match error {
        AcpError::Spawn { source, .. } => body("spawn_failed", source.to_string(), None),
        AcpError::Timeout { after, .. } => body(
            match phase {
                Phase::Initialize => "initialize_timeout",
                Phase::SessionNew => "session_new_timeout",
            },
            format!("deadline ({}ms) exceeded", after.as_millis()),
            None,
        ),
        AcpError::Exited { status, .. } => {
            let code = status.and_then(|status| status.code());
            let signal = status.and_then(exit_signal);
            let when = match phase {
                Phase::Initialize => "before responding to initialize",
                Phase::SessionNew => "during session/new",
            };
            body(
                "agent_crashed",
                format!(
                    "Agent exited (code={}, signal={}) {when}",
                    code.map_or("null".into(), |code| code.to_string()),
                    signal.map_or("null".into(), |signal| signal.to_string()),
                ),
                None,
            )
        }
        AcpError::ProtocolVersion(version) => body(
            "protocol_mismatch",
            format!(
                "Agent advertised protocol version {}; this probe supports 1.",
                serde_json::to_value(version).unwrap_or(Value::Null)
            ),
            None,
        ),
        AcpError::Rpc { code, message, .. } => body("unknown", message, Some(code)),
        AcpError::AuthRequired { message, .. } => body("unknown", message, None),
        other => body("unknown", other.to_string(), None),
    }
}

#[cfg(unix)]
fn exit_signal(status: std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn exit_signal(_: std::process::ExitStatus) -> Option<i32> {
    None
}

fn select_values(options: &SessionConfigSelectOptions) -> Vec<&SessionConfigSelectOption> {
    match options {
        SessionConfigSelectOptions::Ungrouped(values) => values.iter().collect(),
        SessionConfigSelectOptions::Grouped(groups) => {
            groups.iter().flat_map(|group| &group.options).collect()
        }
        _ => vec![],
    }
}

/// From the `model` select option.
///
/// DECIDED: no fallback to the unstable `models` session field and its `model[effort]` splitting.
/// WHY: ACP schema 1.11's stable `NewSessionResponse` has no such field; Claude and Codex both
/// advertise the `model` config option, which TS already preferred.
fn models(options: &[SessionConfigOption]) -> Vec<ProbeModel> {
    options
        .iter()
        .find(|option| option.id.0.as_ref() == "model")
        .and_then(|option| match &option.kind {
            SessionConfigKind::Select(select) => Some(select_values(&select.options)),
            _ => None,
        })
        .unwrap_or_default()
        .into_iter()
        .map(|value| ProbeModel {
            id: value.value.0.to_string(),
            name: Some(value.name.clone()),
            description: value.description.clone(),
        })
        .collect()
}

/// From the first select option in category `thought_level`; its current value is the default.
fn reasoning(options: &[SessionConfigOption]) -> Option<ProbeReasoning> {
    options.iter().find_map(|option| {
        let SessionConfigKind::Select(select) = &option.kind else {
            return None;
        };
        (option.category == Some(SessionConfigOptionCategory::ThoughtLevel)).then(|| {
            ProbeReasoning {
                values: select_values(&select.options)
                    .into_iter()
                    .map(|value| value.value.0.to_string())
                    .collect(),
                default_value: Some(select.current_value.0.to_string()),
            }
        })
    })
}
