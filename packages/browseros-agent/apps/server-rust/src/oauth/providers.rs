//! The OAuth providers table: client ids, URLs, scopes and extra params verbatim from the TS
//! server.

use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OAuthProvider {
    ChatGptPro,
    GitHubCopilot,
    QwenCode,
}

impl OAuthProvider {
    pub const ALL: [Self; 3] = [Self::ChatGptPro, Self::GitHubCopilot, Self::QwenCode];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatGptPro => "chatgpt-pro",
            Self::GitHubCopilot => "github-copilot",
            Self::QwenCode => "qwen-code",
        }
    }

    pub fn spec(self) -> ProviderSpec {
        match self {
            Self::ChatGptPro => ProviderSpec {
                provider: self,
                display_name: "ChatGPT",
                client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
                auth_endpoint: "https://auth.openai.com/oauth/authorize".into(),
                token_endpoint: "https://auth.openai.com/oauth/token".into(),
                scope: "openid profile email offline_access",
            },
            Self::GitHubCopilot => ProviderSpec {
                provider: self,
                display_name: "GitHub Copilot",
                client_id: "Ov23li8tweQw6odWQebz",
                auth_endpoint: "https://github.com/login/device/code".into(),
                token_endpoint: "https://github.com/login/oauth/access_token".into(),
                scope: "read:user",
            },
            Self::QwenCode => ProviderSpec {
                provider: self,
                display_name: "Qwen Code",
                client_id: "f0304373b74a44d2b584a3fb70ca9e56",
                auth_endpoint: "https://chat.qwen.ai/api/v1/oauth2/device/code".into(),
                token_endpoint: "https://chat.qwen.ai/api/v1/oauth2/token".into(),
                scope: "openid profile email model.completion",
            },
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
    pub client_id: &'static str,
    pub auth_endpoint: String,
    pub token_endpoint: String,
    pub scope: &'static str,
}
