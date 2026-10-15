//! ACP agent definitions as the runtime sees them. The rows live in the server's `providers` table
//! (`kind='acp'`); the server validates and loads them, this crate only consumes the parsed values.

use std::{collections::BTreeMap, path::PathBuf};

use uuid::Uuid;

/// `AcpAgentTypeSchema` (`claude | codex | custom`); serialized exactly as the TS wire strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
    Custom,
}

impl AgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Custom => "custom",
        }
    }
}

/// `CustomAcpAgentConfigSchema` minus the UI-only `icon`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CustomAgentConfig {
    /// Full command line, shell-split by [`crate::split_command_line`] and run as given.
    pub command: String,
    pub env: BTreeMap<String, String>,
    /// Mode ids tried in order after every attach; empty leaves the agent's own mode untouched.
    pub full_access_modes: Vec<String>,
    /// Config option id for reasoning effort; `None` means TS's default `effort`.
    pub reasoning_effort_key: Option<String>,
    pub system_prompt_append: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDefinition {
    pub id: Uuid,
    pub kind: AgentKind,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<String>,
    pub working_directory: Option<PathBuf>,
    /// Present only for [`AgentKind::Custom`].
    pub custom: Option<CustomAgentConfig>,
}
