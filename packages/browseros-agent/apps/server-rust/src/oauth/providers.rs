//! The OAuth providers table: client ids, URLs, scopes and extra params verbatim (recon A2.5).

use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OAuthProvider {
    ChatGptPro,
    GitHubCopilot,
    QwenCode,
}

impl OAuthProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatGptPro => "chatgpt-pro",
            Self::GitHubCopilot => "github-copilot",
            Self::QwenCode => "qwen-code",
        }
    }
}

impl FromStr for OAuthProvider {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "chatgpt-pro" => Ok(Self::ChatGptPro),
            "github-copilot" => Ok(Self::GitHubCopilot),
            "qwen-code" => Ok(Self::QwenCode),
            other => Err(format!("Unknown OAuth provider: {other}")),
        }
    }
}

/// Static per-provider endpoints and client config.
#[derive(Debug, Clone)]
pub struct ProviderSpec {
    pub provider: OAuthProvider,
    pub display_name: &'static str,
}
