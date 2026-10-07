use crate::{
    error::{AppError, AppResult},
    identity::ProfileView,
    ids::ProfileId,
    storage::JsonStore,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, str::FromStr};

const AGENTS_SUBDIR: &str = "agents";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Harness {
    #[serde(rename = "Claude Code")]
    ClaudeCode,
    #[serde(rename = "Claude Desktop")]
    ClaudeDesktop,
    #[serde(rename = "Cursor")]
    Cursor,
    #[serde(rename = "VS Code")]
    VsCode,
    #[serde(rename = "Zed")]
    Zed,
    #[serde(rename = "Codex")]
    Codex,
    #[serde(rename = "Gemini CLI")]
    GeminiCli,
    #[serde(rename = "Hermes")]
    Hermes,
    #[serde(rename = "OpenClaw")]
    OpenClaw,
}

impl Harness {
    pub const ALL: [Harness; 9] = [
        Harness::ClaudeCode,
        Harness::ClaudeDesktop,
        Harness::Cursor,
        Harness::VsCode,
        Harness::Zed,
        Harness::Codex,
        Harness::GeminiCli,
        Harness::Hermes,
        Harness::OpenClaw,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::ClaudeDesktop => "Claude Desktop",
            Self::Cursor => "Cursor",
            Self::VsCode => "VS Code",
            Self::Zed => "Zed",
            Self::Codex => "Codex",
            Self::GeminiCli => "Gemini CLI",
            Self::Hermes => "Hermes",
            Self::OpenClaw => "OpenClaw",
        }
    }

    #[must_use]
    pub fn agent_id(self) -> Option<&'static str> {
        match self {
            Self::ClaudeCode => Some("claude-code"),
            Self::ClaudeDesktop => Some("claude-desktop"),
            Self::Cursor => Some("cursor"),
            Self::VsCode => Some("vscode"),
            Self::Zed => Some("zed"),
            Self::Codex => Some("codex"),
            Self::GeminiCli => Some("gemini"),
            Self::Hermes | Self::OpenClaw => None,
        }
    }
}

impl fmt::Display for Harness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Harness {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let decoded = value.replace("%20", " ");
        match decoded.as_str() {
            "Claude Code" => Ok(Self::ClaudeCode),
            "Claude Desktop" => Ok(Self::ClaudeDesktop),
            "Cursor" => Ok(Self::Cursor),
            "VS Code" => Ok(Self::VsCode),
            "Zed" => Ok(Self::Zed),
            "Codex" => Ok(Self::Codex),
            "Gemini CLI" => Ok(Self::GeminiCli),
            "Hermes" => Ok(Self::Hermes),
            "OpenClaw" => Ok(Self::OpenClaw),
            _ => Err(AppError::bad_request("unsupported harness")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoginMode {
    Profile,
    All,
    Selective,
}

// Retained for deserializing existing on-disk agent profile JSON.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalVerdict {
    Auto,
    Ask,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileStatus {
    Configured,
    Paused,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomAclRule {
    pub id: String,
    pub label: String,
    pub domain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAgentProfile {
    pub name: String,
    pub harness: Harness,
    pub login_mode: LoginMode,
    // Retained for compatibility with existing on-disk agent profile JSON.
    #[allow(dead_code)]
    pub selected_sites: Vec<String>,
    #[allow(dead_code)]
    pub approvals: BTreeMap<String, ApprovalVerdict>,
    pub acl_rule_ids: Vec<String>,
    pub custom_acl_rules: Vec<CustomAclRule>,
    pub id: String,
    pub slug: String,
    pub mcp_url: String,
    pub status: ProfileStatus,
    pub created_at: String,
    pub updated_at: String,
}

impl From<&StoredAgentProfile> for ProfileView {
    fn from(profile: &StoredAgentProfile) -> Self {
        Self {
            id: ProfileId::new(profile.id.clone()),
            slug: profile.slug.clone(),
            name: profile.name.clone(),
        }
    }
}

#[derive(Clone)]
pub struct ProfileService {
    store: JsonStore,
}

impl ProfileService {
    #[must_use]
    pub fn new(store: JsonStore) -> Self {
        Self { store }
    }

    pub async fn list_profiles(&self) -> AppResult<Vec<StoredAgentProfile>> {
        let names = self.store.list_files(AGENTS_SUBDIR, ".json").await?;
        let mut out = Vec::new();
        for name in names {
            let rel = format!("{AGENTS_SUBDIR}/{name}");
            match self.store.read_json::<StoredAgentProfile>(&rel).await {
                Ok(profile) => out.push(profile),
                Err(err) => {
                    tracing::warn!(file = %rel, error = %err, "skipping unreadable agent profile")
                }
            }
        }
        Ok(out)
    }
}
