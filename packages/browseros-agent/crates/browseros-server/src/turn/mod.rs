//! `turn`: the vocabulary of one chat turn, shared by both backends, and the only code that
//! knows the AI SDK UI message stream (v1, `ai@7.0.114`).
//!
//! A backend emits `TurnEvent`s into an `EventSink` and returns one `TurnEnd`. `Transcript`
//! turns each event into wire chunks and folds those chunks into the assistant `UiMessage`
//! exactly as the client's reducer does, so what is stored is by construction what the panel
//! rendered. Pure: no I/O, no axum, no tokio (the layering test enforces it).
//!
//! Everything here sits at the bottom of the crate: every other module may import it, and it
//! imports none of them.

mod error;
mod message;
mod transcript;

pub use error::{ChatError, ChatErrorCode};
pub use message::{PartState, Role, ToolOutcome, ToolPart, UiMessage, UiPart};
pub use transcript::{Checkpoint, DONE, Frame, STREAM_HEADERS, Transcript};

/// The extension makes conversation ids; they key the run, the stored row and a coding agent's
/// session.
pub type ConversationId = uuid::Uuid;
/// One server-owned turn. A newer run of the same conversation replaces the older one.
pub type RunId = uuid::Uuid;

/// The id of a `UiMessage`. Assistant ids are made by the server when a run starts; user ids
/// come from the client when it sends one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MessageId(pub String);

/// A tool call's id: the model's own (`call_…`, `toolu_…`) or a coding agent's ACP id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolCallId(pub String);

/// What happened in a turn, in order, whichever backend ran it. Text and reasoning are
/// deltas: the reducer opens a block on the first delta and closes it when anything else
/// arrives, so no backend mints a block id or can emit chunks out of order (which would
/// fail the user's turn in the extension's validator).
#[derive(Debug, Clone, PartialEq)]
pub enum TurnEvent {
    StepStart,
    Text(String),
    Reasoning(String),
    /// A tool card appears; its input is not complete yet.
    ToolStarted {
        id: ToolCallId,
        name: String,
        owner: ToolOwner,
    },
    /// The input is complete and THIS call is about to run now. A checkpoint.
    /// Own loop: emitted by the loop, immediately before `Browser::call` for that call, never for
    /// a call still queued behind another. Coding agent: emitted when the agent's `tool_call`
    /// notification is first seen, which may be after the agent started the tool.
    ToolInput {
        id: ToolCallId,
        name: String,
        input: serde_json::Value,
        owner: ToolOwner,
    },
    /// The tool returned. A checkpoint. Never carries picture bytes (see `ToolOutput::for_panel`).
    ToolDone {
        id: ToolCallId,
        result: Result<serde_json::Value, String>,
    },
    StepEnd,
}

/// Who executes the tool: this server (`tool-<name>` part) or the coding agent itself
/// (`dynamic-tool` with `providerExecuted`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOwner {
    Server,
    Agent,
}

/// How a backend's turn ended. Returned, never emitted, so "exactly one end, and nothing
/// after it" is a fact of the types: `conversations` writes the terminal chunk from this.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnEnd {
    Done(FinishReason),
    Stopped,
    Failed(ChatError),
}

/// The AI SDK's finish reasons, as the `finish` chunk carries them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
    Error,
    Other,
}

/// Where a backend writes its events. The run's sink reduces the event and, when it is a
/// checkpoint, awaits the SQLite write before returning: that await is what makes
/// "persisted before the tool runs" true without a backend knowing about storage.
/// Tests use a `Vec<TurnEvent>`.
#[async_trait::async_trait]
pub trait EventSink: Send {
    async fn emit(&mut self, event: TurnEvent);
}

/// The test sink: keeps every event in order, so a test can compare the whole list.
#[async_trait::async_trait]
impl EventSink for Vec<TurnEvent> {
    async fn emit(&mut self, event: TurnEvent) {
        self.push(event);
    }
}

/// The seam both backends sit behind. `agent::Agent` is the one production implementation;
/// tests of `conversations` use a scripted one.
#[async_trait::async_trait]
pub trait TurnRunner: Send + Sync {
    /// Must return soon after `cancel` fires; `conversations` drops this future after a
    /// 5 s grace, which kills a child process and closes in-flight requests.
    async fn run_turn(
        &self,
        target: &Target,
        input: &TurnInput,
        sink: &mut dyn EventSink,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> TurnEnd;
    /// The conversation is being deleted: end any session a backend keeps for it.
    async fn end_session(&self, conversation: ConversationId);
}

/// What the extension asked to talk to. Resolved against the store when the turn starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The server's own loop on an LLM provider row; `None` is the selected (default) row.
    Provider(Option<String>),
    /// A coding agent run through acpx, by its agent row.
    Coding { kind: AgentKind, agent_id: String },
}

/// The coding agents this server can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
    Codex,
}

/// Everything a backend needs to run one turn. Built by `conversations`; read-only to the
/// backend.
#[derive(Debug, Clone)]
pub struct TurnInput {
    pub conversation: ConversationId,
    /// The transcript before this turn's answer; it ends with the new user message.
    pub history: std::sync::Arc<[UiMessage]>,
    pub context: TurnContext,
    pub attachments: Vec<Attachment>,
    /// True for an incognito turn (`History::Ephemeral`): nothing about it may outlive it.
    /// The own loop keeps nothing anyway. A coding agent must run it in a throwaway session,
    /// seeded from `history`, and remove that session and any staged attachment when the
    /// turn ends.
    pub ephemeral: bool,
}

/// Per-turn context from the extension. It is wrapped around the model's copy of the user
/// message only; storage and the panel keep the raw text.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnContext {
    /// `Chat` gives the browser read-only access (`Mode::access`, in `browser`).
    pub mode: Mode,
    pub window_id: Option<i64>,
    pub active_tab: Option<TabRef>,
    pub selected_tabs: Vec<TabRef>,
    pub selected_text: Option<SelectedText>,
    pub user_system_prompt: Option<String>,
    /// Coding agents only; `None` for an LLM target whatever the request said.
    pub working_dir: Option<std::path::PathBuf>,
    pub supports_reasoning: bool,
}

/// Chat mode reads pages; agent mode may act on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Chat,
    Agent,
}

/// A browser tab as the extension names it. `tab_id` is the browser's own id, which is also the
/// engine's, so a script can find the tab by it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabRef {
    pub tab_id: i64,
    pub url: Option<String>,
    pub title: Option<String>,
}

/// Text the user selected on a page, with where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedText {
    pub text: String,
    pub url: String,
    pub title: String,
}

/// A file attached to the user's message (coding agents only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub media_type: String,
    /// A `data:` URL, as the extension sends it.
    pub data_url: String,
}
