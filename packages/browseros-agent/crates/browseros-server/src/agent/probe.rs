//! `POST /acpx/probe`: start a coding agent's adapter, perform ACP's `initialize` and
//! `session/new`, read the models and config options it advertises, and stop it. It does not
//! prove a login. Chunk `acp` fills the probe itself.

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
