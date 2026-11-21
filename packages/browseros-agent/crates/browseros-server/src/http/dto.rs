//! Every wire type of the extension-facing contract, and its conversion to or from the domain.
//!
//! This module is the HTTP contract's one home (design rule 4): each type here derives
//! `ToSchema`, and `contracts/server-api/openapi.json` is generated from these declarations and
//! the `#[utoipa::path]` of each handler, so a schema cannot disagree with what the server
//! serialises. It is private to `http`: a handler parses a wire type, converts it into a domain
//! type here, and nothing below `http` ever sees a wire type. Field names and shapes follow the
//! routes the extension calls today; DESIGN.md section 4.2 lists what changed.
//!
//! Frozen after the skeleton lands: other chunks code against these types, so a change goes
//! through the captain.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::agent::{AgentProbe, ProbeError, ProbeModel, ProbeReasoning, ProbedAgentInfo};
use crate::conversations::{
    Conversation, History, NewTurn, PanelAssignment, PanelTable, RunState, RunStatus,
};
use crate::model::SignInStatus;
use crate::store::{
    AgentPatch, AgentRow, ConversationSummary, Edit, NewAgent, ProviderEdit, ProviderKind,
    ProviderRow, RowKind, Secret, SecretEdit,
};
use crate::turn::{
    AgentKind, Attachment, ChatError, ChatErrorCode, MessageId, Mode, PartState, Role,
    SelectedText, TabRef, Target, TurnContext, UiMessage, UiPart,
};

// ------------------------------------------------------------------ shared

/// `{"success": true}`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Success {
    #[schema(value_type = bool, example = true)]
    pub success: bool,
}

#[allow(
    dead_code,
    reason = "used by the handlers chunks `runs` and `settings` write"
)]
impl Success {
    pub const OK: Success = Success { success: true };
}

/// `{"error": "<one sentence>"}`: every refusal outside the chat path.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = ApiError)]
pub struct ApiErrorBody {
    pub error: String,
}

/// A `{conversationId}` path segment.
#[allow(
    dead_code,
    reason = "read by the handlers chunks `runs` and `settings` write"
)]
#[derive(Debug, Clone, Copy, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Path)]
pub struct ConversationPath {
    pub conversation_id: Uuid,
}

/// Deserialises a field that distinguishes absent (`Patch::Absent`, through `#[serde(default)]`),
/// `null` and a value, which `Option` cannot.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Patch<T> {
    #[default]
    Absent,
    Null,
    Value(T),
}

fn patch<'de, D, T>(deserializer: D) -> Result<Patch<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(|value| match value {
        Some(value) => Patch::Value(value),
        None => Patch::Null,
    })
}

impl<T> Patch<T> {
    fn into_edit(self) -> Edit<T> {
        match self {
            Patch::Absent => Edit::Keep,
            Patch::Null => Edit::Clear,
            Patch::Value(value) => Edit::Set(value),
        }
    }
}

impl Patch<String> {
    /// A credential: absent or blank keeps the stored value (an untouched form field submits
    /// `""`), `null` clears it.
    fn into_secret_edit(self) -> SecretEdit {
        match self {
            Patch::Absent => Edit::Keep,
            Patch::Value(value) if value.is_empty() => Edit::Keep,
            Patch::Null => Edit::Clear,
            Patch::Value(value) => Edit::Set(Secret::new(value)),
        }
    }
}

/// A blank string is no value.
fn non_blank(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

// ------------------------------------------------------------------ chat request

/// One turn. The server owns history: the client sends the new message and the conversation
/// id, never the transcript. Unknown fields are ignored, because the extension and the server
/// update separately.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    /// Made by the extension. Keys the run, the stored row and the agent session.
    pub conversation_id: Uuid,
    pub target: ChatTarget,
    /// The user's text for this turn.
    #[serde(default)]
    pub message: String,
    /// The id the client gave this user message. If the stored transcript already holds a
    /// message with this id, everything after it is dropped and it is answered again: this is
    /// how Retry and a repeated POST avoid storing the question twice. Absent: the server makes
    /// an id and appends.
    #[serde(default)]
    pub user_message_id: Option<String>,
    /// `chat` gives the agent read-only access to the browser.
    #[serde(default)]
    pub mode: ChatMode,
    #[serde(default)]
    pub browser_context: Option<BrowserContext>,
    /// The user's personalization text.
    #[serde(default)]
    pub user_system_prompt: Option<String>,
    /// Working directory for a coding agent. Ignored for LLM targets.
    #[serde(default)]
    pub user_working_dir: Option<String>,
    /// From the extension's model catalogue. False turns reasoning options off.
    #[serde(default = "yes")]
    pub supports_reasoning: bool,
    #[serde(default)]
    pub selected_text: Option<String>,
    #[serde(default)]
    pub selected_text_source: Option<SelectedTextSource>,
    /// False in an incognito window: nothing is written, for either kind of target.
    #[serde(default = "yes")]
    pub persist: bool,
    /// Text-only history, sent only when `persist` is false.
    #[serde(default)]
    pub previous_conversation: Vec<PreviousMessage>,
    /// Files attached to this message. Coding agents only.
    #[serde(default)]
    pub attachments: Vec<ChatAttachment>,
}

fn yes() -> bool {
    true
}

/// What the turn talks to: the server's own loop on an LLM provider (`browseros`), or a coding
/// agent run through acpx.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ChatTarget {
    /// The server's own agent loop on one of the user's LLM providers.
    Browseros {
        /// A provider row id. Absent means the selected provider.
        #[serde(default, rename = "providerId")]
        provider_id: Option<String>,
    },
    Claude {
        #[serde(rename = "agentId")]
        #[schema(format = Uuid)]
        agent_id: String,
    },
    Codex {
        #[serde(rename = "agentId")]
        #[schema(format = Uuid)]
        agent_id: String,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ChatMode {
    Chat,
    #[default]
    Agent,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrowserContext {
    #[serde(default)]
    pub window_id: Option<i64>,
    #[serde(default)]
    pub active_tab: Option<Tab>,
    #[serde(default)]
    pub selected_tabs: Vec<Tab>,
}

/// A browser tab by the browser's own id.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct Tab {
    /// The browser's own tab id.
    pub id: i64,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SelectedTextSource {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PreviousMessage {
    pub role: PreviousRole,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum PreviousRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatAttachment {
    pub media_type: String,
    /// A data URL.
    pub data: String,
}

impl From<Tab> for TabRef {
    fn from(tab: Tab) -> Self {
        TabRef {
            tab_id: tab.id,
            url: tab.url,
            title: tab.title,
        }
    }
}

impl From<ChatRequest> for NewTurn {
    /// Every check a request body needs is in its types (the uuid, the target's tag), so once
    /// the body parsed the conversion cannot fail.
    fn from(body: ChatRequest) -> Self {
        let target = match body.target {
            ChatTarget::Browseros { provider_id } => Target::Provider(non_blank(provider_id)),
            ChatTarget::Claude { agent_id } => Target::Coding {
                kind: AgentKind::Claude,
                agent_id,
            },
            ChatTarget::Codex { agent_id } => Target::Coding {
                kind: AgentKind::Codex,
                agent_id,
            },
        };
        let working_dir = match target {
            Target::Coding { .. } => non_blank(body.user_working_dir).map(PathBuf::from),
            Target::Provider(_) => None,
        };
        let browser = body.browser_context.unwrap_or_default();
        let panel_tab = browser.active_tab.as_ref().map(|tab| tab.id);
        let selected_text = non_blank(body.selected_text).map(|text| {
            let source = body.selected_text_source;
            SelectedText {
                text,
                url: source.as_ref().map(|s| s.url.clone()).unwrap_or_default(),
                title: source.map(|s| s.title).unwrap_or_default(),
            }
        });
        let history = if body.persist {
            History::Stored
        } else {
            History::Ephemeral(
                body.previous_conversation
                    .into_iter()
                    .enumerate()
                    .map(|(position, message)| UiMessage {
                        id: MessageId(format!("previous-{position}")),
                        role: match message.role {
                            PreviousRole::User => Role::User,
                            PreviousRole::Assistant => Role::Assistant,
                        },
                        parts: vec![UiPart::Text {
                            text: message.content,
                            state: Some(PartState::Done),
                            extra: serde_json::Map::new(),
                        }],
                        metadata: None,
                    })
                    .collect(),
            )
        };
        NewTurn {
            conversation: body.conversation_id,
            user_message_id: non_blank(body.user_message_id).map(MessageId),
            text: body.message,
            attachments: body
                .attachments
                .into_iter()
                .map(|attachment| Attachment {
                    media_type: attachment.media_type,
                    data_url: attachment.data,
                })
                .collect(),
            target,
            context: TurnContext {
                mode: match body.mode {
                    ChatMode::Chat => Mode::Chat,
                    ChatMode::Agent => Mode::Agent,
                },
                window_id: browser.window_id,
                active_tab: browser.active_tab.map(TabRef::from),
                selected_tabs: browser
                    .selected_tabs
                    .into_iter()
                    .map(TabRef::from)
                    .collect(),
                selected_text,
                user_system_prompt: non_blank(body.user_system_prompt),
                working_dir,
                supports_reasoning: body.supports_reasoning,
            },
            history,
            panel_tab,
        }
    }
}

// ------------------------------------------------------------------ chat responses

/// The chat path's error body: the same envelope an `error` chunk carries in `errorText`, so
/// the chat error card renders both the same way.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ChatErrorEnvelope {
    pub error: ChatErrorBody,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = ChatError)]
pub struct ChatErrorBody {
    pub code: ChatErrorCodeName,
    pub title: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
    /// Upstream text with secrets scrubbed, at most 20,000 characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = ChatErrorCode)]
pub enum ChatErrorCodeName {
    RateLimited,
    AuthFailed,
    ProviderConfig,
    ContextLength,
    ContentFilter,
    ConnectionFailed,
    ProviderUnavailable,
    Unknown,
}

impl From<ChatErrorCode> for ChatErrorCodeName {
    fn from(code: ChatErrorCode) -> Self {
        match code {
            ChatErrorCode::RateLimited => Self::RateLimited,
            ChatErrorCode::AuthFailed => Self::AuthFailed,
            ChatErrorCode::ProviderConfig => Self::ProviderConfig,
            ChatErrorCode::ContextLength => Self::ContextLength,
            ChatErrorCode::ContentFilter => Self::ContentFilter,
            ChatErrorCode::ConnectionFailed => Self::ConnectionFailed,
            ChatErrorCode::ProviderUnavailable => Self::ProviderUnavailable,
            ChatErrorCode::Unknown => Self::Unknown,
        }
    }
}

impl From<ChatError> for ChatErrorEnvelope {
    fn from(error: ChatError) -> Self {
        ChatErrorEnvelope {
            error: ChatErrorBody {
                code: error.code.into(),
                title: error.title,
                message: error.message,
                retryable: error.retryable,
                provider: error.provider,
                status_code: error.status_code,
                docs_url: error.docs_url,
                retry_after_seconds: error.retry_after_seconds,
                details: error.details,
            },
        }
    }
}

/// `GET /chat/{conversationId}/stream?runId=`.
#[allow(dead_code, reason = "read by the handler chunk `runs` writes")]
#[derive(Debug, Clone, Copy, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct StreamQuery {
    /// The run the panel read from `getRunState`. Absent means the conversation's current run.
    #[serde(default)]
    pub run_id: Option<Uuid>,
}

/// `DELETE /chat/panels/{tabId}`.
#[allow(dead_code, reason = "read by the handler chunk `runs` writes")]
#[derive(Debug, Clone, Copy, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Path)]
pub struct TabPath {
    /// The browser's tab id; a negative one is `400`.
    #[param(minimum = 0)]
    #[serde(deserialize_with = "non_negative")]
    pub tab_id: i64,
}

/// Browser tab ids are never negative; the old server refused one with `400` too.
fn non_negative<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    let value = i64::deserialize(deserializer)?;
    if value < 0 {
        return Err(serde::de::Error::custom(
            "tabId must be a non-negative integer",
        ));
    }
    Ok(value)
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct StopResult {
    /// False when nothing was running.
    pub stopped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
#[schema(as = RunStatus)]
pub enum RunStatusName {
    Running,
    Completed,
    Aborted,
    Failed,
}

impl From<RunStatus> for RunStatusName {
    fn from(status: RunStatus) -> Self {
        match status {
            RunStatus::Running => Self::Running,
            RunStatus::Completed => Self::Completed,
            RunStatus::Aborted => Self::Aborted,
            RunStatus::Failed => Self::Failed,
        }
    }
}

/// `messages` is the history up to and including this turn's user message, without the
/// assistant message in flight; the stream supplies that.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = RunState)]
pub struct RunStateBody {
    pub conversation_id: String,
    pub run_id: String,
    pub status: RunStatusName,
    pub messages: Vec<UiMessageBody>,
    pub chunk_count: usize,
}

impl From<RunState> for RunStateBody {
    fn from(state: RunState) -> Self {
        RunStateBody {
            conversation_id: state.conversation.to_string(),
            run_id: state.run.to_string(),
            status: state.status.into(),
            messages: state.messages.iter().map(UiMessageBody::from).collect(),
            chunk_count: state.frame_count,
        }
    }
}

/// One event of `GET /chat/panels`: the whole table.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PanelAssignments {
    pub assignments: Vec<PanelAssignmentBody>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = PanelAssignment)]
pub struct PanelAssignmentBody {
    pub tab_id: i64,
    pub conversation_id: String,
    pub run_id: String,
    pub status: RunStatusName,
    /// Set once for a tab the agent opened; tells the extension to open its panel there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_open_id: Option<String>,
}

impl From<&PanelTable> for PanelAssignments {
    fn from(table: &PanelTable) -> Self {
        PanelAssignments {
            assignments: table
                .assignments
                .iter()
                .map(|assignment: &PanelAssignment| PanelAssignmentBody {
                    tab_id: assignment.tab_id,
                    conversation_id: assignment.conversation.to_string(),
                    run_id: assignment.run.to_string(),
                    status: assignment.status.into(),
                    auto_open_id: assignment.auto_open_id.clone(),
                })
                .collect(),
        }
    }
}

/// The subset of the AI SDK `ai@7.0.114` UI message chunk union this server emits, one per
/// `data:` event of a chat stream. Field names and order rules are the SDK's; every chunk is
/// validated by the client and an unknown `type` fails the turn. `turn` serialises the real
/// chunks; this type only describes them for the contract.
#[allow(
    dead_code,
    reason = "describes the stream in the contract; `turn` writes the chunks"
)]
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum UiMessageChunk {
    /// First chunk. `messageId` is the server's id for the assistant message; it is the same in
    /// the stored transcript and in every replay.
    Start {
        #[serde(rename = "messageId")]
        message_id: String,
    },
    /// Around each model call of the loop. A coding-agent turn is one step.
    StartStep,
    FinishStep,
    TextStart {
        id: String,
    },
    TextDelta {
        id: String,
        delta: String,
    },
    TextEnd {
        id: String,
    },
    ReasoningStart {
        id: String,
    },
    ReasoningDelta {
        id: String,
        delta: String,
    },
    ReasoningEnd {
        id: String,
    },
    /// A tool call exists. For the server's loop the tool is `run`; for a coding agent
    /// `dynamic` and `providerExecuted` are true.
    #[serde(rename_all = "camelCase")]
    ToolInputStart {
        tool_call_id: String,
        tool_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        dynamic: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
    },
    #[serde(rename_all = "camelCase")]
    ToolInputAvailable {
        tool_call_id: String,
        tool_name: String,
        input: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        dynamic: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
    },
    #[serde(rename_all = "camelCase")]
    ToolOutputAvailable {
        tool_call_id: String,
        output: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        dynamic: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
    },
    #[serde(rename_all = "camelCase")]
    ToolOutputError {
        tool_call_id: String,
        error_text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        dynamic: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
    },
    #[serde(rename_all = "camelCase")]
    Finish {
        #[serde(skip_serializing_if = "Option::is_none")]
        finish_reason: Option<FinishReasonName>,
    },
    /// `errorText` is a JSON string holding a ChatErrorEnvelope, for both kinds of target.
    #[serde(rename_all = "camelCase")]
    Error {
        error_text: String,
    },
    /// The turn was stopped.
    Abort {
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

#[allow(
    dead_code,
    reason = "describes the stream in the contract; `turn` writes the chunks"
)]
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
#[schema(as = FinishReason)]
pub enum FinishReasonName {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
    Error,
    Other,
}

// ------------------------------------------------------------------ transcript

/// An AI SDK `UIMessage` (`ai@7.0.114`). The extension passes it straight to `setMessages`, so
/// the part shapes are the SDK's, not ours; they are left open here on purpose.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = UiMessage)]
pub struct UiMessageBody {
    pub id: String,
    pub role: RoleName,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    pub parts: Vec<UiPartBody>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
#[schema(as = Role)]
pub enum RoleName {
    System,
    User,
    Assistant,
}

/// One part of a `UIMessage`, in the SDK's shape: an object with a `type` and that type's own
/// fields.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(as = UiPart, value_type = Object)]
pub struct UiPartBody(pub Value);

impl From<&UiMessage> for UiMessageBody {
    fn from(message: &UiMessage) -> Self {
        UiMessageBody {
            id: message.id.0.clone(),
            role: match message.role {
                Role::System => RoleName::System,
                Role::User => RoleName::User,
                Role::Assistant => RoleName::Assistant,
            },
            metadata: message.metadata.clone(),
            parts: message
                .parts
                .iter()
                .map(|part| UiPartBody(serde_json::to_value(part).unwrap_or(Value::Null)))
                .collect(),
        }
    }
}

// ------------------------------------------------------------------ conversations

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConversationList {
    pub conversations: Vec<ConversationSummaryBody>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = ConversationSummary)]
pub struct ConversationSummaryBody {
    pub id: String,
    /// First 200 characters of the last user text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_user_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// `browseros`, `claude`, `codex`, or `custom` on rows from the old server.
    #[schema(value_type = ConversationTargetType)]
    pub target_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub last_messaged_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The kinds of target a stored conversation can name.
#[allow(dead_code, reason = "names the `targetType` values in the contract")]
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ConversationTargetType {
    Browseros,
    Claude,
    Codex,
    Custom,
}

impl From<ConversationSummary> for ConversationSummaryBody {
    fn from(summary: ConversationSummary) -> Self {
        ConversationSummaryBody {
            id: summary.id.to_string(),
            last_user_message: summary.last_user_message,
            origin: summary.origin,
            target_type: summary.target_type,
            agent_id: summary.agent_id,
            last_messaged_at: summary.last_messaged_at,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConversationResponse {
    pub conversation: ConversationDetail,
}

/// A conversation summary with its transcript.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConversationDetail {
    #[serde(flatten)]
    pub summary: ConversationSummaryBody,
    pub messages: Vec<UiMessageBody>,
}

impl From<Conversation> for ConversationDetail {
    fn from(conversation: Conversation) -> Self {
        ConversationDetail {
            summary: conversation.summary.into(),
            messages: conversation
                .messages
                .iter()
                .map(UiMessageBody::from)
                .collect(),
        }
    }
}

// ------------------------------------------------------------------ providers

/// A `{providerId}` path segment.
#[allow(dead_code, reason = "read by the handlers chunk `settings` writes")]
#[derive(Debug, Clone, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Path)]
pub struct ProviderPath {
    pub provider_id: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProviderList {
    pub providers: Vec<ProviderBody>,
}

/// The selected row, or `null` when none is selected.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DefaultProvider {
    #[schema(required = true)]
    pub provider: Option<ProviderBody>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProviderResponse {
    pub provider: ProviderBody,
}

#[allow(dead_code, reason = "read by the handler chunk `settings` writes")]
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetDefaultProvider {
    pub provider_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProviderRowKind {
    Llm,
    Acp,
}

/// A row of the `providers` table without its four secrets. Every key is always present;
/// nullable ones are `null` (the extension's converter reads the keys).
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = Provider)]
pub struct ProviderBody {
    pub id: String,
    pub kind: ProviderRowKind,
    /// A ProviderType for `llm` rows, an AgentType for `acp` rows.
    pub r#type: String,
    pub name: String,
    #[schema(required = true)]
    pub model_id: Option<String>,
    #[schema(required = true)]
    pub reasoning_effort: Option<String>,
    #[schema(required = true)]
    pub reasoning_summary: Option<String>,
    pub is_default: bool,
    #[schema(required = true)]
    pub base_url: Option<String>,
    #[schema(required = true)]
    pub headers: Option<BTreeMap<String, String>>,
    pub supports_images: bool,
    #[schema(required = true)]
    pub context_window: Option<i64>,
    pub temperature: f64,
    #[schema(required = true)]
    pub resource_name: Option<String>,
    #[schema(required = true)]
    pub region: Option<String>,
    #[schema(required = true)]
    pub working_directory: Option<String>,
    #[schema(required = true)]
    pub custom_config: Option<String>,
    pub has_api_key: bool,
    pub has_access_key_id: bool,
    pub has_secret_access_key: bool,
    pub has_session_token: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<ProviderRow> for ProviderBody {
    fn from(row: ProviderRow) -> Self {
        let (kind, r#type) = match row.kind {
            RowKind::Llm(provider) => (ProviderRowKind::Llm, provider.as_str()),
            RowKind::Acp(agent) => (ProviderRowKind::Acp, AgentType::from(agent).as_str()),
        };
        ProviderBody {
            id: row.id,
            kind,
            r#type: r#type.to_owned(),
            name: row.name,
            model_id: row.model_id,
            reasoning_effort: row.reasoning_effort,
            reasoning_summary: row.reasoning_summary,
            is_default: row.is_default,
            base_url: row.base_url,
            headers: row.headers.map(|headers| headers.into_iter().collect()),
            supports_images: row.supports_images,
            context_window: row.context_window,
            temperature: row.temperature,
            resource_name: row.resource_name,
            region: row.region,
            working_directory: row.working_directory,
            custom_config: row.custom_config,
            has_api_key: row.has_api_key,
            has_access_key_id: row.has_access_key_id,
            has_secret_access_key: row.has_secret_access_key,
            has_session_token: row.has_session_token,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// The provider types this server runs. Derived from `store::ProviderKind`, the one list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderType {
    Anthropic,
    Openai,
    Google,
    Openrouter,
    Fireworks,
    Azure,
    Bedrock,
    OpenaiCompatible,
    Ollama,
    Lmstudio,
    Moonshot,
    ChatgptPro,
}

impl From<ProviderType> for ProviderKind {
    fn from(kind: ProviderType) -> Self {
        match kind {
            ProviderType::Anthropic => ProviderKind::Anthropic,
            ProviderType::Openai => ProviderKind::Openai,
            ProviderType::Google => ProviderKind::Google,
            ProviderType::Openrouter => ProviderKind::Openrouter,
            ProviderType::Fireworks => ProviderKind::Fireworks,
            ProviderType::Azure => ProviderKind::Azure,
            ProviderType::Bedrock => ProviderKind::Bedrock,
            ProviderType::OpenaiCompatible => ProviderKind::OpenaiCompatible,
            ProviderType::Ollama => ProviderKind::Ollama,
            ProviderType::Lmstudio => ProviderKind::Lmstudio,
            ProviderType::Moonshot => ProviderKind::Moonshot,
            ProviderType::ChatgptPro => ProviderKind::ChatgptPro,
        }
    }
}

/// `PUT /providers/{providerId}`. A credential that is absent or an empty string keeps the
/// stored value; `null` clears it. Any other nullable field: absent keeps, `null` clears.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInput {
    pub r#type: ProviderType,
    #[schema(min_length = 1)]
    pub name: String,
    #[schema(min_length = 1)]
    pub model_id: String,
    /// Tokens. A number, stored rounded (the old server validated a number).
    pub context_window: f64,
    #[serde(default, deserialize_with = "patch")]
    #[schema(value_type = Option<String>)]
    pub base_url: Patch<String>,
    /// A value may contain the literal `{{conversationId}}`.
    #[serde(default, deserialize_with = "patch")]
    #[schema(value_type = Option<BTreeMap<String, String>>)]
    pub headers: Patch<BTreeMap<String, String>>,
    #[serde(default)]
    pub supports_images: Option<bool>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default, deserialize_with = "patch")]
    #[schema(value_type = Option<String>)]
    pub api_key: Patch<String>,
    #[serde(default, deserialize_with = "patch")]
    #[schema(value_type = Option<String>)]
    pub access_key_id: Patch<String>,
    #[serde(default, deserialize_with = "patch")]
    #[schema(value_type = Option<String>)]
    pub secret_access_key: Patch<String>,
    #[serde(default, deserialize_with = "patch")]
    #[schema(value_type = Option<String>)]
    pub session_token: Patch<String>,
    #[serde(default, deserialize_with = "patch")]
    #[schema(value_type = Option<String>)]
    pub resource_name: Patch<String>,
    #[serde(default, deserialize_with = "patch")]
    #[schema(value_type = Option<String>)]
    pub region: Patch<String>,
    #[serde(default, deserialize_with = "patch")]
    #[schema(value_type = Option<String>)]
    pub reasoning_effort: Patch<String>,
    #[serde(default, deserialize_with = "patch")]
    #[schema(value_type = Option<String>)]
    pub reasoning_summary: Patch<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
}

impl TryFrom<ProviderInput> for ProviderEdit {
    type Error = String;

    fn try_from(input: ProviderInput) -> Result<Self, String> {
        if input.name.trim().is_empty() {
            return Err("name must not be empty".into());
        }
        if input.model_id.trim().is_empty() {
            return Err("modelId must not be empty".into());
        }
        if !input.context_window.is_finite() || input.context_window < 1.0 {
            return Err("contextWindow must be a positive number".into());
        }
        // Finite and at least 1, so the cast saturates only above i64::MAX tokens.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a finite token count, rounded"
        )]
        let context_window = input.context_window.round() as i64;
        Ok(ProviderEdit {
            kind: input.r#type.into(),
            name: input.name,
            model_id: input.model_id,
            context_window,
            base_url: input.base_url.into_edit(),
            headers: match input.headers {
                Patch::Absent => Edit::Keep,
                Patch::Null => Edit::Clear,
                Patch::Value(headers) => Edit::Set(headers.into_iter().collect()),
            },
            supports_images: input.supports_images,
            temperature: input.temperature,
            api_key: input.api_key.into_secret_edit(),
            access_key_id: input.access_key_id.into_secret_edit(),
            secret_access_key: input.secret_access_key.into_secret_edit(),
            session_token: input.session_token.into_secret_edit(),
            resource_name: input.resource_name.into_edit(),
            region: input.region.into_edit(),
            reasoning_effort: input.reasoning_effort.into_edit(),
            reasoning_summary: input.reasoning_summary.into_edit(),
            created_at: input.created_at,
        })
    }
}

/// `POST /test-provider`. A blank credential is filled from the stored row named by
/// `providerId`; when it is, the stored base URL and region are forced too, so a saved secret
/// is never sent to an address the caller chose. Unknown fields are ignored.
#[allow(dead_code, reason = "read by the handler chunk `models` writes")]
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestProviderRequest {
    /// A saved row to borrow blank credentials from.
    #[serde(default)]
    pub provider_id: Option<String>,
    pub provider: ProviderType,
    #[schema(min_length = 1)]
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub resource_name: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub access_key_id: Option<String>,
    #[serde(default)]
    pub secret_access_key: Option<String>,
    #[serde(default)]
    pub session_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestProviderResult {
    pub success: bool,
    pub message: String,
    /// Milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_time: Option<u64>,
}

// ------------------------------------------------------------------ agents

/// An `{agentId}` path segment.
#[allow(dead_code, reason = "read by the handlers chunk `settings` writes")]
#[derive(Debug, Clone, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Path)]
pub struct AgentPath {
    #[param(format = Uuid)]
    pub agent_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AgentType {
    Claude,
    Codex,
}

impl AgentType {
    fn as_str(self) -> &'static str {
        match self {
            AgentType::Claude => "claude",
            AgentType::Codex => "codex",
        }
    }
}

impl From<AgentKind> for AgentType {
    fn from(kind: AgentKind) -> Self {
        match kind {
            AgentKind::Claude => AgentType::Claude,
            AgentKind::Codex => AgentType::Codex,
        }
    }
}

impl From<AgentType> for AgentKind {
    fn from(kind: AgentType) -> Self {
        match kind {
            AgentType::Claude => AgentKind::Claude,
            AgentType::Codex => AgentKind::Codex,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentList {
    pub agents: Vec<AgentBody>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentResponse {
    pub agent: AgentBody,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = Agent)]
pub struct AgentBody {
    #[schema(format = Uuid)]
    pub id: String,
    pub name: String,
    pub r#type: AgentType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<AgentRow> for AgentBody {
    fn from(row: AgentRow) -> Self {
        AgentBody {
            id: row.id,
            name: row.name,
            r#type: row.kind.into(),
            model_id: row.model_id,
            reasoning_effort: row.reasoning_effort,
            working_directory: row.working_directory,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

const AGENT_NAME_MAX: usize = 80;

fn check_agent_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        Err("name must not be empty".into())
    } else if name.chars().count() > AGENT_NAME_MAX {
        Err(format!("name must be at most {AGENT_NAME_MAX} characters"))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentInput {
    #[schema(min_length = 1, max_length = 80)]
    pub name: String,
    pub r#type: AgentType,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
}

impl TryFrom<AgentInput> for NewAgent {
    type Error = String;

    fn try_from(input: AgentInput) -> Result<Self, String> {
        check_agent_name(&input.name)?;
        Ok(NewAgent {
            name: input.name,
            kind: input.r#type.into(),
            model_id: non_blank(input.model_id),
            reasoning_effort: non_blank(input.reasoning_effort),
            working_directory: non_blank(input.working_directory),
        })
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schema(as = AgentPatch)]
pub struct AgentPatchBody {
    #[serde(default)]
    #[schema(min_length = 1, max_length = 80)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "patch")]
    #[schema(value_type = Option<String>)]
    pub model_id: Patch<String>,
    #[serde(default, deserialize_with = "patch")]
    #[schema(value_type = Option<String>)]
    pub reasoning_effort: Patch<String>,
    #[serde(default, deserialize_with = "patch")]
    #[schema(value_type = Option<String>)]
    pub working_directory: Patch<String>,
}

impl TryFrom<AgentPatchBody> for AgentPatch {
    type Error = String;

    fn try_from(body: AgentPatchBody) -> Result<Self, String> {
        if let Some(name) = &body.name {
            check_agent_name(name)?;
        }
        Ok(AgentPatch {
            name: body.name,
            model_id: body.model_id.into_edit(),
            reasoning_effort: body.reasoning_effort.into_edit(),
            working_directory: body.working_directory.into_edit(),
        })
    }
}

/// `POST /acpx/probe`.
#[allow(dead_code, reason = "read by the handler chunk `acp` writes")]
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProbeRequest {
    pub r#type: AgentType,
    #[serde(default)]
    #[schema(minimum = 1000, maximum = 120000)]
    pub timeout_ms: Option<u64>,
}

/// What a coding agent advertised. A probe that could not run has `error` set.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = AgentProbe)]
pub struct AgentProbeBody {
    pub models: Vec<ProbeModelBody>,
    #[schema(required = true)]
    pub reasoning: Option<ProbeReasoningBody>,
    pub supports_config_option: bool,
    #[schema(required = true)]
    pub agent_info: Option<AgentInfoBody>,
    pub protocol_version: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProbeErrorBody>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = ProbeModel)]
pub struct ProbeModelBody {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = ProbeReasoning)]
pub struct ProbeReasoningBody {
    pub values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = AgentInfo)]
pub struct AgentInfoBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = ProbeError)]
pub struct ProbeErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acp_error_code: Option<i64>,
}

impl From<AgentProbe> for AgentProbeBody {
    fn from(probe: AgentProbe) -> Self {
        AgentProbeBody {
            models: probe
                .models
                .into_iter()
                .map(|model: ProbeModel| ProbeModelBody {
                    id: model.id,
                    name: model.name,
                    description: model.description,
                })
                .collect(),
            reasoning: probe
                .reasoning
                .map(|reasoning: ProbeReasoning| ProbeReasoningBody {
                    values: reasoning.values,
                    default_value: reasoning.default_value,
                }),
            supports_config_option: probe.supports_config_option,
            agent_info: probe.agent_info.map(|info: ProbedAgentInfo| AgentInfoBody {
                name: info.name,
                title: info.title,
                version: info.version,
            }),
            protocol_version: probe.protocol_version,
            error: probe.error.map(|error: ProbeError| ProbeErrorBody {
                code: error.code,
                message: error.message,
                acp_error_code: error.acp_error_code,
            }),
        }
    }
}

// ------------------------------------------------------------------ sign-in

/// The sign-in providers this server owns. Today only ChatGPT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SignInProvider {
    ChatgptPro,
}

#[allow(dead_code, reason = "used by the handlers chunk `chatgpt` writes")]
impl SignInProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            SignInProvider::ChatgptPro => "chatgpt-pro",
        }
    }
}

/// A `{provider}` path segment of the sign-in routes.
#[allow(dead_code, reason = "read by the handlers chunk `chatgpt` writes")]
#[derive(Debug, Clone, Copy, Deserialize, IntoParams)]
#[into_params(parameter_in = Path)]
pub struct SignInPath {
    pub provider: SignInProvider,
}

/// Whether tokens are stored for this install.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = SignInStatus)]
pub struct SignInStatusBody {
    pub authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub provider: String,
}

#[allow(dead_code, reason = "used by the handler chunk `chatgpt` writes")]
impl SignInStatusBody {
    pub fn new(provider: SignInProvider, status: SignInStatus) -> Self {
        SignInStatusBody {
            authenticated: status.authenticated,
            email: status.email,
            provider: provider.as_str().to_owned(),
        }
    }
}

// ------------------------------------------------------------------ system

/// Always `ok`: an answer at all is the signal.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum StatusOk {
    Ok,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub status: StatusOk,
    /// Whether the browser engine is linked to the browser. Always false where the engine is
    /// not built.
    pub cdp_connected: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Status {
    pub status: StatusOk,
    #[serde(rename = "cdpConnected")]
    pub cdp_connected: bool,
    /// Snake case because the browser's C++ reads this key.
    pub can_update: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub version: String,
    pub os: String,
    #[schema(required = true)]
    pub os_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ShutdownResult {
    pub status: StatusOk,
}
