//! One normalized `/chat` body: the `ChatInputSchema` union (ACP variant tried first) with zod
//! defaults/transforms applied, split into the target and the per-turn fields both engines read.
//! Owned by P8.

use crate::db::AgentKind;
use browseros_agent::prompt::Origin;
use std::path::PathBuf;
use uuid::Uuid;

/// The browser context as the app sends it (contract `BrowserContext`).
pub type BrowserContext = server_api::BrowserContext;
/// The browser context after tab ids are resolved to page ids against the live browser.
pub type ResolvedBrowserContext = browseros_agent::prompt::BrowserContextView;
pub type SelectedTextSource = server_api::SelectedTextSource;
pub type PreviousMessage = server_api::PreviousMessage;
pub type Attachment = server_api::ChatAttachment;

pub struct ChatRequest {
    pub conversation_id: Uuid,
    pub message: String,
    pub target: RequestTarget,
    pub input: ChatTurnRequest,
}

pub enum RequestTarget {
    BrowserOs {
        provider_id: Option<String>,
        inline: Box<InlineLlm>,
    },
    Acp {
        kind: AgentKind,
        agent_id: Uuid,
    },
}

/// Inline provider fields of a BrowserOS-target body; a stored row (by `provider_id`) overrides
/// them during hydration.
#[derive(Debug, Clone, Default)]
pub struct InlineLlm {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub headers: Option<std::collections::BTreeMap<String, String>>,
    pub resource_name: Option<String>,
    pub region: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    pub upstream_provider: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChatMode {
    #[default]
    Agent,
    Chat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryMode {
    Local,
    Cloud,
}

#[derive(Debug, Clone, Default)]
pub struct ChatTurnRequest {
    pub context_window_size: Option<f64>,
    pub browser_context: Option<BrowserContext>,
    pub user_system_prompt: Option<String>,
    pub is_scheduled_task: bool,
    pub user_working_dir: Option<PathBuf>,
    pub supports_images: bool,
    pub supports_reasoning: bool,
    pub mode: ChatMode,
    pub origin: Origin,
    pub declined_apps: Vec<String>,
    pub selected_text: Option<String>,
    pub selected_text_source: Option<SelectedTextSource>,
    pub previous_conversation: Vec<PreviousMessage>,
    pub persist: Option<bool>,
    pub history_mode: Option<HistoryMode>,
    /// Used by ACP only; the built-in path ignores attachments (recon F9 parity).
    pub attachments: Vec<Attachment>,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
}
