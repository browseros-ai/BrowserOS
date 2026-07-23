use std::{collections::BTreeMap, fmt, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalog::{AgentId, HarnessDefinition, HttpShape, McpSurface, McpTransport, StdioShape};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentScope {
    #[default]
    System,
    Project,
}

impl fmt::Display for AgentScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::System => formatter.write_str("system"),
            Self::Project => formatter.write_str("project"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "lowercase")]
pub enum McpServerSpec {
    Stdio {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
    },
    Sse {
        url: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        headers: BTreeMap<String, String>,
    },
    Http {
        url: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        headers: BTreeMap<String, String>,
    },
}

impl McpServerSpec {
    pub const fn transport(&self) -> McpTransport {
        match self {
            Self::Stdio { .. } => McpTransport::Stdio,
            Self::Sse { .. } => McpTransport::Sse,
            Self::Http { .. } => McpTransport::Http,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    pub spec: McpServerSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestLinkEntry {
    pub config_path: PathBuf,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestServerEntry {
    pub name: String,
    pub spec: McpServerSpec,
    pub added_at: String,
    pub links: BTreeMap<AgentId, ManifestLinkEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerManifest {
    pub version: u8,
    pub servers: BTreeMap<String, ManifestServerEntry>,
}

impl Default for ServerManifest {
    fn default() -> Self {
        Self {
            version: 1,
            servers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInput {
    pub server: McpServer,
    pub agent: AgentId,
    pub scope: AgentScope,
    pub config_path: Option<PathBuf>,
    pub allow_overwrite: bool,
}

impl LinkInput {
    /// Creates a system-scope link request with overwrite protection enabled.
    pub fn new(server: McpServer, agent: AgentId) -> Self {
        Self {
            server,
            agent,
            scope: AgentScope::System,
            config_path: None,
            allow_overwrite: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnlinkInput {
    pub server_name: String,
    pub agent: AgentId,
    pub scope: AgentScope,
    pub config_path: Option<PathBuf>,
}

impl UnlinkInput {
    /// Creates a system-scope unlink request that uses the manifest-recorded path.
    pub fn new(server_name: impl Into<String>, agent: AgentId) -> Self {
        Self {
            server_name: server_name.into(),
            agent,
            scope: AgentScope::System,
            config_path: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectInput {
    pub server_name: String,
    pub agent: AgentId,
    pub scope: AgentScope,
    pub remove_if_last: bool,
}

impl DisconnectInput {
    /// Creates a system-scope disconnect request that removes a last-link manifest entry.
    pub fn new(server_name: impl Into<String>, agent: AgentId) -> Self {
        Self {
            server_name: server_name.into(),
            agent,
            scope: AgentScope::System,
            remove_if_last: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSummary {
    pub server_name: String,
    pub agent: AgentId,
    pub scope: AgentScope,
    pub created: bool,
    pub overwrote_foreign: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnlinkSummary {
    pub server_name: String,
    pub agent: AgentId,
    pub scope: AgentScope,
    pub removed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectSummary {
    pub server_name: String,
    pub agent: AgentId,
    pub scope: AgentScope,
    pub unlinked: bool,
    pub removed_manifest: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListLinksFilter {
    pub server_names: Option<Vec<String>>,
    pub agents: Option<Vec<AgentId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedLink {
    pub server_name: String,
    pub agent: AgentId,
    pub config_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RescanEntry {
    pub server_name: String,
    pub agent: AgentId,
    pub scope: AgentScope,
    pub config_path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RescanReport {
    pub verified: Vec<ListedLink>,
    pub drifted: Vec<RescanEntry>,
    pub missing: Vec<RescanEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentSurface {
    pub harness: &'static HarnessDefinition,
    pub mcp: &'static McpSurface,
    pub supported_transports: &'static [McpTransport],
    pub stdio: StdioShape,
    pub http: Option<HttpShape>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInfo {
    pub id: AgentId,
    pub display_name: String,
    pub config_path: Option<PathBuf>,
    pub installed: bool,
}
