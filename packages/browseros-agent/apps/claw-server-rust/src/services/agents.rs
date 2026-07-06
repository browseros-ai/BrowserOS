use crate::{
    error::{AppError, AppResult},
    storage::JsonStore,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, str::FromStr};

const AGENTS_SUBDIR: &str = "agents";
const TOTAL_PROFILE_LOGINS: usize = 47;

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
    pub selected_sites: Vec<String>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfileSummary {
    pub id: String,
    pub name: String,
    pub harness: Harness,
    pub login_scope_label: String,
    pub login_count: usize,
    pub acl_rule_count: usize,
    pub blocked_action_count: usize,
    pub always_allow_count: usize,
    pub last_run_at: String,
    pub status: ProfileStatus,
    pub mcp_url: String,
}

#[derive(Clone)]
pub struct AgentService {
    store: JsonStore,
    public_mcp_url: String,
}

impl AgentService {
    #[must_use]
    pub fn new(store: JsonStore, public_mcp_url: String) -> Self {
        Self {
            store,
            public_mcp_url,
        }
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

    pub async fn list(&self) -> AppResult<Vec<AgentProfileSummary>> {
        let mut profiles = self.list_profiles().await?;
        profiles.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(profiles
            .iter()
            .map(|profile| self.summarize(profile))
            .collect())
    }

    pub async fn load_by_id(&self, id: &str) -> AppResult<Option<StoredAgentProfile>> {
        if !is_valid_id(id) {
            return Ok(None);
        }
        let rel = file_for(id);
        match self.store.read_json(&rel).await {
            Ok(profile) => Ok(Some(profile)),
            Err(AppError::StorageNotFound(_)) | Err(AppError::InvalidStoragePath(_)) => Ok(None),
            Err(err) => Err(err),
        }
    }

    fn summarize(&self, profile: &StoredAgentProfile) -> AgentProfileSummary {
        let blocked_action_count = profile
            .approvals
            .values()
            .filter(|value| matches!(value, ApprovalVerdict::Block))
            .count();
        let login_count = match profile.login_mode {
            LoginMode::Selective => profile.selected_sites.len(),
            LoginMode::Profile | LoginMode::All => TOTAL_PROFILE_LOGINS,
        };
        let login_scope_label = match profile.login_mode {
            LoginMode::Selective => format!("Selective ({})", profile.selected_sites.len()),
            LoginMode::All => format!("All my logins ({TOTAL_PROFILE_LOGINS})"),
            LoginMode::Profile => format!("Current profile ({TOTAL_PROFILE_LOGINS})"),
        };
        AgentProfileSummary {
            id: profile.id.clone(),
            name: profile.name.clone(),
            harness: profile.harness,
            login_scope_label,
            login_count,
            acl_rule_count: profile.acl_rule_ids.len(),
            blocked_action_count,
            always_allow_count: 0,
            last_run_at: "Never run".to_string(),
            status: profile.status,
            mcp_url: self.public_mcp_url.clone(),
        }
    }
}

fn file_for(id: &str) -> String {
    format!("{AGENTS_SUBDIR}/{id}.json")
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}
