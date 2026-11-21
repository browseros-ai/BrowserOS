//! Discovers settings from the same pinned adapter used by acpx. The probe owns a transient
//! ACP connection and never tries to authenticate or reuse a conversation session.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::time::timeout;

use super::AgentRuntime;
use super::acpx::{Adapter, Process, absolute_runtime};
use crate::turn::AgentKind;

/// What a coding agent advertised. A probe that could not run has `error` set and empty lists,
/// so the new-agent dialog still opens.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentProbe {
    pub models: Vec<ProbeModel>,
    pub reasoning: Option<ProbeReasoning>,
    pub supports_config_option: bool,
    pub agent_info: Option<ProbedAgentInfo>,
    pub protocol_version: i64,
    pub error: Option<ProbeError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeModel {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

/// The reasoning levels an agent offers, and its default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReasoning {
    pub values: Vec<String>,
    pub default_value: Option<String>,
}

/// The agent's own `agentInfo` from ACP `initialize`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProbedAgentInfo {
    pub name: Option<String>,
    pub title: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeError {
    pub code: String,
    pub message: String,
    /// The JSON-RPC error code when the adapter answered with one.
    pub acp_error_code: Option<i64>,
}

impl AgentProbe {
    /// A probe that could not run.
    pub fn failed(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: Some(ProbeError {
                code: code.into(),
                message: message.into(),
                acp_error_code: None,
            }),
            ..Self::default()
        }
    }
}

pub(super) async fn probe(runtime: &AgentRuntime, kind: AgentKind, budget: Duration) -> AgentProbe {
    discover(runtime, kind.into(), budget)
        .await
        .unwrap_or_else(|error| AgentProbe {
            error: Some(error),
            ..AgentProbe::default()
        })
}

async fn discover(
    runtime: &AgentRuntime,
    adapter: Adapter,
    budget: Duration,
) -> Result<AgentProbe, ProbeError> {
    let runtime = &absolute_runtime(runtime).map_err(probe_io)?;
    let root = &runtime.state_dir;
    std::fs::create_dir_all(root).map_err(probe_io)?;
    let scratch = tempfile::Builder::new()
        .prefix("probe-")
        .tempdir_in(root)
        .map_err(probe_io)?;
    let cwd = scratch.path().canonicalize().map_err(probe_io)?;
    let mut process = Process::spawn(
        adapter
            .launch(runtime)
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped()),
    )
    .map_err(probe_io)?;
    let mut connection = ProbeConnection {
        stdin: process
            .0
            .stdin
            .take()
            .ok_or_else(|| probe_io("missing stdin"))?,
        stdout: BufReader::new(
            process
                .0
                .stdout
                .take()
                .ok_or_else(|| probe_io("missing stdout"))?,
        )
        .lines(),
    };
    let discovery = timeout(budget, connection.discover(&cwd))
        .await
        .map_err(|_| ProbeError {
            code: "timeout".into(),
            message: format!(
                "{} did not answer the settings probe in time.",
                adapter.name()
            ),
            acp_error_code: None,
        })?;
    // The ACP adapter remains live after session/new. Closing its pipe and dropping
    // the process group before the temporary directory goes away retires the probe.
    drop(connection);
    drop(process);
    discovery
}

struct ProbeConnection {
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
}

impl ProbeConnection {
    async fn discover(&mut self, cwd: &Path) -> Result<AgentProbe, ProbeError> {
        let initialize = self
            .request(
                1,
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {},
                    "clientInfo": {"name":"browseros", "version":crate::VERSION}
                }),
            )
            .await?;
        let version = initialize["protocolVersion"]
            .as_i64()
            .filter(|version| *version == 1)
            .ok_or_else(|| probe_io("unsupported ACP protocol version"))?;
        let session = self
            .request(2, "session/new", json!({"cwd":cwd, "mcpServers":[]}))
            .await?;
        Ok(settings(version, &initialize, &session))
    }

    async fn request(&mut self, id: u32, method: &str, params: Value) -> Result<Value, ProbeError> {
        let mut request =
            json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}).to_string();
        request.push('\n');
        self.stdin
            .write_all(request.as_bytes())
            .await
            .map_err(probe_io)?;
        while let Some(line) = self.stdout.next_line().await.map_err(probe_io)? {
            let message: Value = serde_json::from_str(&line).map_err(probe_io)?;
            if message.get("method").is_some() {
                // The probe advertises no client capabilities. Reject agent requests
                // so an interactive adapter cannot stall model discovery.
                if let Some(request_id) = message.get("id") {
                    let mut reply = json!({"jsonrpc":"2.0", "id":request_id, "error":{
                        "code":-32601, "message":"Method not supported by the settings probe"
                    }})
                    .to_string();
                    reply.push('\n');
                    self.stdin
                        .write_all(reply.as_bytes())
                        .await
                        .map_err(probe_io)?;
                }
                continue;
            }
            if message["id"] != id {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(ProbeError {
                    code: "acp_error".into(),
                    message: format!("The coding agent rejected the {method} request."),
                    acp_error_code: error["code"].as_i64(),
                });
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| probe_io("missing ACP response"));
        }
        Err(probe_io("adapter closed its output"))
    }
}

fn probe_io(error: impl std::fmt::Display) -> ProbeError {
    tracing::debug!(%error, "coding agent probe I/O failed");
    ProbeError {
        code: "wrapper_error".into(),
        message: "The coding agent could not start or respond to the settings probe.".into(),
        acp_error_code: None,
    }
}

fn settings(version: i64, initialize: &Value, session: &Value) -> AgentProbe {
    let mut report = AgentProbe {
        agent_info: initialize
            .get("agentInfo")
            .filter(|info| info.is_object())
            .map(|info| ProbedAgentInfo {
                name: string(&info["name"]),
                title: string(&info["title"]),
                version: string(&info["version"]),
            }),
        protocol_version: version,
        ..AgentProbe::default()
    };

    let options = entries(&session["configOptions"]);
    report.supports_config_option = !options.is_empty();
    let model = option(options, &["model"]);
    let reasoning = option(options, &["thought_level", "reasoning_effort", "effort"]);
    let mut legacy_efforts = Vec::new();

    let legacy_models = &session["models"];
    let models = entries(&legacy_models["availableModels"]);
    for item in models {
        let Some(id) = item["modelId"].as_str() else {
            continue;
        };
        let (base, effort) = split_model(id);
        if let Some(effort) = effort
            && !legacy_efforts.iter().any(|known| known == effort)
        {
            legacy_efforts.push(effort.to_owned());
        }
        if !report.models.iter().any(|model| model.id == base) {
            let name = item["name"].as_str().map(|name| {
                effort
                    .and_then(|effort| name.strip_suffix(&format!(" ({effort})")))
                    .unwrap_or(name)
                    .to_owned()
            });
            report.models.push(ProbeModel {
                id: base.into(),
                name,
                description: string(&item["description"]),
            });
        }
    }
    if let Some(model) = model {
        let advertised = choices(model)
            .filter_map(|item| {
                Some(ProbeModel {
                    id: item["value"].as_str()?.into(),
                    name: string(&item["name"]),
                    description: string(&item["description"]),
                })
            })
            .collect::<Vec<_>>();
        if !advertised.is_empty() {
            report.models = advertised;
        }
    }

    let legacy_current = legacy_models["currentModelId"].as_str();
    report.reasoning = reasoning
        .map(|option| ProbeReasoning {
            values: choices(option)
                .filter_map(|item| string(&item["value"]))
                .collect(),
            default_value: string(&option["currentValue"]),
        })
        .filter(|reasoning| !reasoning.values.is_empty())
        .or_else(|| {
            (!legacy_efforts.is_empty()).then(|| ProbeReasoning {
                values: legacy_efforts,
                default_value: legacy_current
                    .and_then(|current| split_model(current).1)
                    .map(str::to_owned),
            })
        });
    report
}

fn entries(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn option<'a>(options: &'a [Value], keys: &[&str]) -> Option<&'a Value> {
    options.iter().find(|option| {
        keys.iter()
            .any(|key| option["id"] == *key || option["category"] == *key)
    })
}

fn choices(option: &Value) -> impl Iterator<Item = &Value> {
    entries(&option["options"]).iter().flat_map(|choice| {
        choice["options"]
            .as_array()
            .map_or_else(|| std::slice::from_ref(choice), Vec::as_slice)
    })
}

fn string(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

pub(super) fn split_model(model: &str) -> (&str, Option<&str>) {
    const EFFORTS: &[&str] = &[
        "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
    ];
    let variant = model
        .strip_suffix(']')
        .and_then(|id| id.rsplit_once('['))
        .or_else(|| model.rsplit_once('/'));
    if let Some((base, effort)) = variant
        && !base.is_empty()
        && EFFORTS.contains(&effort)
    {
        return (base, Some(effort));
    }
    (model, None)
}
