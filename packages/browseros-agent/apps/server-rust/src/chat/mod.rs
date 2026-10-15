//! The chat conversation layer (layer 1 of the agent seam): hydrates the target, owns the run
//! registry (409, replay, panels), the one display history per conversation, chunk → UIMessage
//! assembly and persistence, then hands each turn to a [`ChatEngine`].
//!
//! ```text
//! POST /chat ─► ChatService::start ── trust + provider hydration (403 / 400)
//!                 └─► ConversationRuns::start (409 gate, before any await)
//!                       └─ prepare: agent lookup (404/400) · credentials (500) · history + user
//!                          message published · ChatEngine::start_turn(Turn)
//!                                ├─ BrowserOsEngine (built-in loop)   ─┐ ChunkStream
//!                                └─ AcpEngine (ACP agents)            ─┘   │
//!   replay log + live tail ◄── UiMessageAssembler ◄────────────────────────┘
//!   on_finish: stored history (verbatim) + filter_valid(user + assistant) → cache; persist if
//!              policy allows; scheduled runs release their engine session and cache; the final
//!              typed history goes back to the run, published with the terminal status
//!
//! DELETE /chat/{id} ─► ChatService::delete (own task) ─► ConversationRuns::delete_with:
//!   panels retired + 409 gate ─ stop, await end ─ cleanup (ACP + built-in forget, cache) ─ remove
//! ```
//!
//! Owned by P8 (`mod`, `runs`, `history`, `request`, `engine`), P14 (`browseros_engine`),
//! P16 (`acp_engine`).

mod acp_engine;
mod browseros_engine;
pub mod engine;
pub mod history;
pub mod request;
pub mod runs;

pub use acp_engine::{AcpEngine, PREPARATION_ERROR};
pub use browseros_engine::{
    BackgroundPages, BrowserOsEngine, BrowserOsEngineDeps, KlavisStatus, LeaseIssuer, ModelFactory,
    NoBrowser, RegistryLeases, SessionLease, klavis_state_name, production_models,
};
pub use engine::{
    ChatEngine, ChatRejection, ResolvedLlm, ScriptedEngine, ScriptedTurn, Turn, TurnTarget,
    stopped_chunk,
};
pub use history::{ConversationHistory, History, TargetKey};
pub use request::{ChatRequest, ChatTurnRequest, ProviderRecord, RequestTarget};
pub use runs::{
    ActiveRun, ConversationRuns, FinishedTurn, OnFinish, Prepared, RunHandle, RunPresentation,
    RunSnapshot, RunStatus, StartedRun, TabGroupPresentation,
};

use crate::{
    db::{
        AcpAgentRow, AgentKind, AgentStore, ConversationStore, Database, DbError, DbResult,
        ProviderStore, SaveRawConversation, StoredConversation,
    },
    http::security::RequestTrust,
    oauth::TokenManager,
};
use browseros_agent::{
    llm::{ChatError, ChatErrorCode},
    messages::filter_valid,
    prompt::{BrowserContextView, TabView},
};
use request::{HydratedProvider, ResolvedBrowserContext};
use std::{
    collections::HashMap,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
};
use ui_message_stream::{ChunkStream, FilePart, Role, TextPart, UiMessage, UiPart};
use uuid::Uuid;

/// Outcome of `GET /chat/{id}/stream`.
pub enum Resume {
    /// 204: no run record for this conversation.
    NoRun,
    /// 409: the caller's `runId` is not the current run.
    RunChanged,
    /// Replay of the run so far, then the live tail.
    Stream(ChunkStream),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteOutcome {
    pub deleted: bool,
    pub session_count: usize,
}

/// `GET /chat/panels` frames: the complete assignment table, sent immediately, on every change
/// and as the 5 s heartbeat (a heartbeat is a full repeat, not a comment line).
pub type PanelStream = Pin<
    Box<dyn futures_core::Stream<Item = server_api::ConversationPanelAssignments> + Send + 'static>,
>;

/// The rows chat reads and writes: provider/agent lookup for hydration, conversation rows for
/// history. A seam with two adapters — [`DbChatRecords`] over the SQLite stores and an in-memory
/// one in tests — mirroring TS's injectable `ChatProviderLookup`/stores.
#[async_trait::async_trait]
pub trait ChatRecords: Send + Sync {
    /// What `id` names in `providers`: an LLM row, an ACP agent row, or nothing.
    async fn provider(&self, id: &str) -> DbResult<Option<ProviderRecord>>;
    /// The selected default provider.
    async fn default_provider(&self) -> DbResult<Option<ProviderRecord>>;
    async fn agent(&self, id: Uuid) -> DbResult<Option<AcpAgentRow>>;
    async fn conversation(&self, id: Uuid) -> DbResult<Option<StoredConversation>>;
    /// Writes the messages verbatim: chat appends each turn to the stored JSON as read.
    async fn save_conversation(&self, input: SaveRawConversation) -> DbResult<()>;
}

/// Production [`ChatRecords`] over the shared SQLite stores.
pub struct DbChatRecords {
    providers: ProviderStore,
    agents: AgentStore,
    conversations: ConversationStore,
}

impl DbChatRecords {
    pub fn new(db: &Database) -> Self {
        Self {
            providers: db.providers(),
            agents: db.agents(),
            conversations: db.conversations(),
        }
    }
}

#[async_trait::async_trait]
impl ChatRecords for DbChatRecords {
    async fn provider(&self, id: &str) -> DbResult<Option<ProviderRecord>> {
        Ok(self.providers.get(id).await?.map(provider_record))
    }

    async fn default_provider(&self) -> DbResult<Option<ProviderRecord>> {
        Ok(self.providers.default().await?.map(provider_record))
    }

    async fn agent(&self, id: Uuid) -> DbResult<Option<AcpAgentRow>> {
        self.agents.get(id).await
    }

    async fn conversation(&self, id: Uuid) -> DbResult<Option<StoredConversation>> {
        self.conversations.get(id).await
    }

    async fn save_conversation(&self, input: SaveRawConversation) -> DbResult<()> {
        self.conversations.save_raw(input).await
    }
}

/// One `providers` row as hydration sees it: ACP agents share the table (`kind = 'acp'`) and may
/// even be the selected default, and naming one on the browseros path is a 400 (TS checks
/// `row.kind !== 'llm'`).
fn provider_record(row: crate::db::ProviderRow) -> ProviderRecord {
    if row.kind == "llm" {
        ProviderRecord::Llm(Box::new(row))
    } else {
        ProviderRecord::CodingAgent { id: row.id }
    }
}

/// Chrome tab id → BrowserOS page id, against the live browser (TS `browser.resolveTabIds`).
/// Implemented over the MCP host's `BrowserService` once it is wired (P15).
#[async_trait::async_trait]
pub trait TabResolver: Send + Sync {
    /// Tabs that do not resolve are simply absent from the map.
    async fn resolve_tab_ids(&self, tab_ids: &[i64]) -> HashMap<i64, u32>;
}

/// What `ChatService` is built from.
pub struct ChatDeps {
    pub runs: Arc<ConversationRuns>,
    pub records: Arc<dyn ChatRecords>,
    pub browseros: Arc<dyn ChatEngine>,
    pub acp: Arc<dyn ChatEngine>,
    pub oauth: Arc<TokenManager>,
    /// `None` until the browser is wired: page ids then pass through as the client sent them.
    pub tabs: Option<Arc<dyn TabResolver>>,
    /// `BROWSEROS_USE_MOCK_LLM=true`: no credential resolution (the engine swaps in the mock).
    pub use_mock_llm: bool,
}

pub struct ChatService {
    runs: Arc<ConversationRuns>,
    history: Arc<ConversationHistory>,
    records: Arc<dyn ChatRecords>,
    browseros: Arc<dyn ChatEngine>,
    acp: Arc<dyn ChatEngine>,
    oauth: Arc<TokenManager>,
    tabs: Option<Arc<dyn TabResolver>>,
    use_mock_llm: bool,
    /// conversation → ACP agent, for every conversation that ran an ACP turn in this process
    /// (TS `acpConversationAgents`): DELETE needs the trusted-app check only for these.
    acp_conversations: Arc<Mutex<HashMap<Uuid, Uuid>>>,
}

/// Which engine a turn goes to, decided before the 409 gate (hydration) and finished after it.
enum Plan {
    BrowserOs(Box<HydratedProvider>),
    Acp { kind: AgentKind, agent_id: Uuid },
}

impl ChatService {
    pub fn new(deps: ChatDeps) -> Arc<Self> {
        Arc::new(Self {
            history: Arc::new(ConversationHistory::new(deps.records.clone())),
            runs: deps.runs,
            records: deps.records,
            browseros: deps.browseros,
            acp: deps.acp,
            oauth: deps.oauth,
            tabs: deps.tabs,
            use_mock_llm: deps.use_mock_llm,
            acp_conversations: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Error order is TS's and is part of the contract: trust + provider-row hydration (403 / 400
    /// text) → run registration (409) → inside prepare: ACP agent lookup (404 / 400 mismatch) →
    /// credentials (OAuth refresh), stored-history read and engine prepare (500 envelope, or 409
    /// ACP busy). History is published before the engine is called. The returned stream is a
    /// subscriber of this run's own record.
    pub async fn start(
        &self,
        request: ChatRequest,
        trust: RequestTrust,
    ) -> Result<ChunkStream, ChatRejection> {
        let ChatRequest {
            conversation_id,
            message,
            target,
            mut input,
        } = request;

        let plan = match target {
            RequestTarget::Acp { kind, agent_id } => {
                if !trust.trusted_app {
                    return Err(ChatRejection::Forbidden);
                }
                Plan::Acp { kind, agent_id }
            }
            RequestTarget::BrowserOs {
                provider_id,
                inline,
            } => {
                let row = match provider_id.as_deref() {
                    Some(id) => self.records.provider(id).await,
                    None => self.records.default_provider().await,
                }
                .map_err(|error| ChatRejection::Internal(db_error(&error)))?;
                let hydrated =
                    request::hydrate_provider(&inline, provider_id.as_deref(), &input, row)
                        .map_err(ChatRejection::BadRequest)?;
                // A request carrying its own key may only spend what the caller already holds.
                // Naming a stored row, or a provider whose token lives on this machine, has the
                // server supply credentials: that needs the trusted app.
                let uses_server_credentials = hydrated.used_stored_provider
                    || request::is_server_credentialed(&hydrated.provider);
                if uses_server_credentials && !trust.trusted_app {
                    return Err(ChatRejection::Forbidden);
                }
                input.context_window_size = hydrated.context_window_size;
                input.supports_images = hydrated.supports_images;
                input.reasoning_effort = hydrated.reasoning_effort.clone();
                input.reasoning_summary = hydrated.reasoning_summary.clone();
                Plan::BrowserOs(Box::new(hydrated))
            }
        };

        let presentation = RunPresentation {
            panel_tab_ids: active_tab_id(input.browser_context.as_ref())
                .into_iter()
                .collect(),
            panels_visible: !input.is_scheduled_task,
            tab_group: (!input.is_scheduled_task).then(|| tab_group(&message, &plan)),
            keep_replay: !input.is_scheduled_task,
        };
        let prepare = self.prepare(conversation_id, message, plan, Arc::new(input));
        Ok(self
            .runs
            .start(conversation_id, presentation, prepare)
            .await?
            .chunks)
    }

    /// Builds the run's `prepare` step. Everything it captures is owned, because the run's task —
    /// not the HTTP request — drives it.
    fn prepare(
        &self,
        conversation_id: Uuid,
        message: String,
        plan: Plan,
        input: Arc<ChatTurnRequest>,
    ) -> impl FnOnce(
        RunHandle,
    )
        -> futures_util::future::BoxFuture<'static, Result<Prepared, ChatRejection>>
    + Send
    + 'static {
        let records = self.records.clone();
        let history = self.history.clone();
        let browseros = self.browseros.clone();
        let acp = self.acp.clone();
        let oauth = self.oauth.clone();
        let tabs = self.tabs.clone();
        let use_mock_llm = self.use_mock_llm;
        let acp_conversations = self.acp_conversations.clone();
        move |run: RunHandle| {
            Box::pin(async move {
                let (target, target_key, engine, persist) = match plan {
                    Plan::Acp { kind, agent_id } => {
                        let agent = records
                            .agent(agent_id)
                            .await
                            .map_err(|error| ChatRejection::Internal(db_error(&error)))?
                            .ok_or(ChatRejection::UnknownAgent)?;
                        if agent.kind != kind {
                            return Err(ChatRejection::AgentTypeMismatch);
                        }
                        // The ACP display copy is always written (TS persists ACP turns
                        // unconditionally; the app sends no `persist` on this branch).
                        (
                            TurnTarget::Acp(agent),
                            TargetKey::Acp { kind, agent_id },
                            acp,
                            true,
                        )
                    }
                    Plan::BrowserOs(hydrated) => {
                        let resolved = request::resolve_llm(
                            &hydrated,
                            &input,
                            conversation_id,
                            &oauth,
                            use_mock_llm,
                        )
                        .await
                        .map_err(ChatRejection::Internal)?;
                        (
                            TurnTarget::BrowserOs(Box::new(resolved)),
                            TargetKey::BrowserOs,
                            browseros,
                            input.should_persist(),
                        )
                    }
                };

                let browser_context =
                    resolve_browser_context(tabs.as_deref(), input.browser_context.as_ref()).await;
                // An unreadable stored history fails the turn (TS: 500) rather than seeding a
                // fresh one this turn's persist would write over the row.
                let prior = history
                    .load_or_seed(
                        conversation_id,
                        &target_key,
                        persist,
                        &input.previous_conversation,
                    )
                    .await
                    .map_err(|error| ChatRejection::Internal(db_error(&error)))?;
                let user = user_message(&message, &input, &target_key);
                let mut with_user = prior.typed.clone();
                with_user.push(user.clone());
                if let TargetKey::Acp { agent_id, .. } = target_key {
                    acp_conversations
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(conversation_id, agent_id);
                }
                // Publish canonical display history before the engine can call loopback MCP or a
                // second panel hydrates this run.
                run.publish_messages(with_user.clone());

                let turn = Turn {
                    conversation_id,
                    run: run.clone(),
                    target,
                    request: input.clone(),
                    history: with_user.clone(),
                    browser_context,
                };
                let chunks = match engine.start_turn(turn).await {
                    Ok(chunks) => chunks,
                    Err(rejection) => {
                        run.publish_messages(prior.typed);
                        return Err(rejection);
                    }
                };

                let on_finish: OnFinish = Box::new(move |finished: FinishedTurn| {
                    Box::pin(async move {
                        // `filter_valid` is per message, so filtering only this turn equals
                        // filtering the whole list; the stored prefix is never rewritten.
                        let turn =
                            filter_valid(std::iter::once(user).chain(finished.assistant).collect());
                        let stored = prior.stored_with(&turn);
                        if !input.is_scheduled_task {
                            history.replace(conversation_id, &target_key, stored.clone());
                        }
                        // A built-in turn is durable only once it produced an assistant reply:
                        // persisting a history ending on the user message makes the next turn
                        // send two user messages in a row, which providers reject. ACP agents
                        // keep their own continuity (acpx) and the row is only their display
                        // copy, which TS writes after every turn, aborted and failed included.
                        let durable = match target_key {
                            TargetKey::Acp { .. } => true,
                            TargetKey::BrowserOs => {
                                turn.last().is_some_and(|last| last.role == Role::Assistant)
                            }
                        };
                        if persist && durable {
                            history
                                .persist(conversation_id, &target_key, &stored, input.origin)
                                .await;
                        }
                        if input.is_scheduled_task {
                            // Each scheduled run uses a fresh conversation id nobody DELETEs:
                            // release what the engine and cache hold for it now (the replay log
                            // goes with its last subscriber, `keep_replay`). The run is still
                            // `running`, so no new turn can race this release.
                            engine.forget(conversation_id).await;
                            history.forget(conversation_id);
                            acp_conversations
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .remove(&conversation_id);
                        }
                        // Published by the run together with its terminal status.
                        let mut messages = filter_valid(prior.typed);
                        messages.extend(turn);
                        Some(messages)
                    })
                });
                Ok(Prepared { chunks, on_finish })
            })
        }
    }

    /// Waits on the prepared gate, so `/state` never shows a half-registered run.
    pub async fn state(&self, conversation_id: Uuid) -> Option<RunSnapshot> {
        self.runs.prepared_snapshot(conversation_id).await
    }

    /// `run_id` is the raw `?runId=` string (compared as TS compares it).
    pub fn resume(&self, conversation_id: Uuid, run_id: Option<&str>) -> Resume {
        self.runs.subscribe(conversation_id, run_id)
    }

    pub async fn stop(&self, conversation_id: Uuid) -> bool {
        self.runs.stop(conversation_id).await
    }

    /// Stops the run and awaits its end, has both engines release what they hold for the
    /// conversation and drops its cached history, then removes the run record — last, so a POST
    /// arriving mid-cleanup is 409 instead of a turn on a session being forgotten.
    ///
    /// The work runs on its own task and this only awaits it: axum drops a handler whose client
    /// hung up, and a delete abandoned part-way would leak the engine session, its lease and MCP
    /// clients, and the ACP session.
    pub async fn delete(self: &Arc<Self>, conversation_id: Uuid) -> DeleteOutcome {
        let service = self.clone();
        let deleting = tokio::spawn(async move {
            let (run_deleted, (acp_held, session_deleted)) = service
                .runs
                .delete_with(conversation_id, async {
                    let acp_agent = service
                        .acp_conversations
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&conversation_id);
                    if acp_agent.is_some() {
                        service.acp.forget(conversation_id).await;
                    }
                    let session_deleted = service.browseros.forget(conversation_id).await;
                    service.history.forget(conversation_id);
                    (acp_agent.is_some(), session_deleted)
                })
                .await;
            DeleteOutcome {
                deleted: run_deleted || acp_held || session_deleted,
                session_count: service.browseros.session_count(),
            }
        });
        match deleting.await {
            Ok(outcome) => outcome,
            // A panic in an engine's `forget`; the record was still removed (`delete_with`).
            Err(error) => {
                tracing::error!(%conversation_id, %error, "conversation delete failed");
                DeleteOutcome {
                    deleted: false,
                    session_count: self.browseros.session_count(),
                }
            }
        }
    }

    /// DELETE requires the trusted-app check only for ACP conversations.
    pub fn is_acp_conversation(&self, conversation_id: Uuid) -> bool {
        self.acp_conversations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&conversation_id)
    }

    pub fn panel_assignments(&self) -> PanelStream {
        self.runs.panel_assignments()
    }

    pub fn remove_panel_tab(&self, tab_id: u32) {
        self.runs.remove_panel_tab(tab_id);
    }
}

/// This turn's user message as the client shows it: raw text (never the model-facing wrapper).
/// Built-in turns carry one text part (attachments are ignored there, recon F9); ACP turns carry
/// the text (when non-empty) then the attachments as data-URL file parts, TS order.
fn user_message(message: &str, input: &ChatTurnRequest, target: &TargetKey) -> UiMessage {
    let text = |text: &str| {
        UiPart::Text(TextPart {
            text: text.to_owned(),
            state: None,
            provider_metadata: None,
        })
    };
    let parts = match target {
        TargetKey::BrowserOs => vec![text(message)],
        TargetKey::Acp { .. } => (!message.is_empty())
            .then(|| text(message))
            .into_iter()
            .chain(input.attachments.iter().map(|attachment| {
                UiPart::File(FilePart {
                    media_type: attachment.media_type.clone(),
                    filename: None,
                    url: if attachment.data.starts_with("data:") {
                        attachment.data.clone()
                    } else {
                        format!("data:{};base64,{}", attachment.media_type, attachment.data)
                    },
                    provider_reference: None,
                    provider_metadata: None,
                })
            }))
            .collect(),
    };
    UiMessage {
        id: Uuid::new_v4().to_string(),
        role: Role::User,
        metadata: None,
        parts,
    }
}

/// Panels a turn wakes immediately: the active tab only (selected/context tabs are input, not
/// panel ownership). Ids that are not tab ids (negative, fractional) are ignored.
fn active_tab_id(context: Option<&request::BrowserContext>) -> Option<u32> {
    let id = context?.active_tab.as_ref()?.id;
    (id.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&id)).then_some(id as u32)
}

/// `conversationTabGroup`: `<target>/<first five words, lowercased, '-'-joined, ≤ 40 UTF-16
/// units>` (or `task`); colored by target identity so one agent's groups share a color.
fn tab_group(message: &str, plan: &Plan) -> TabGroupPresentation {
    let lowered = message.to_lowercase();
    let words: Vec<&str> = lowered
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(5)
        .collect();
    let mut task = String::new();
    let mut units = 0;
    for ch in words.join("-").chars() {
        units += ch.len_utf16();
        if units > 40 {
            break;
        }
        task.push(ch);
    }
    if task.is_empty() {
        task = "task".to_owned();
    }
    let (prefix, color_key) = match plan {
        Plan::BrowserOs(_) => ("browseros", "browseros".to_owned()),
        Plan::Acp { kind, agent_id } => (kind.as_str(), format!("{}:{agent_id}", kind.as_str())),
    };
    TabGroupPresentation {
        title: format!("{prefix}/{task}"),
        color_key,
    }
}

/// `resolveBrowserContextPageIds` + projection to the engines' view. With a resolver, every tab's
/// page id is the resolved one (absent when unresolvable, even if the client sent one).
async fn resolve_browser_context(
    tabs: Option<&dyn TabResolver>,
    context: Option<&request::BrowserContext>,
) -> Option<ResolvedBrowserContext> {
    let context = context?;
    let listed = || {
        context
            .active_tab
            .iter()
            .chain(context.selected_tabs.iter().flatten())
            .chain(context.tabs.iter().flatten())
    };
    let resolved = match tabs {
        Some(tabs) => {
            let mut ids: Vec<i64> = listed().map(|tab| tab.id as i64).collect();
            ids.sort_unstable();
            ids.dedup();
            if ids.is_empty() {
                None
            } else {
                Some(tabs.resolve_tab_ids(&ids).await)
            }
        }
        None => None,
    };
    let view = |tab: &server_api::BrowserTab| {
        let id = tab.id as i64;
        let page_id = match &resolved {
            Some(map) => map.get(&id).copied(),
            None => tab
                .page_id
                .filter(|page| page.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(page))
                .map(|page| page as u32),
        };
        if resolved.is_some() && page_id.is_none() {
            tracing::warn!(tab_id = id, "could not resolve page id for tab");
        }
        TabView {
            id,
            page_id,
            url: tab.url.clone(),
            title: tab.title.clone(),
        }
    };
    Some(BrowserContextView {
        window_id: context.window_id.map(|id| id as i64),
        active_tab: context.active_tab.as_ref().map(view),
        selected_tabs: context.selected_tabs.iter().flatten().map(view).collect(),
        enabled_mcp_servers: context.enabled_mcp_servers.clone().unwrap_or_default(),
        custom_mcp_urls: context
            .custom_mcp_servers
            .iter()
            .flatten()
            .map(|server| server.url.clone())
            .collect(),
    })
}

/// The envelope TS `toChatError` builds for a plain `Error` with this message — used for the texts
/// this layer raises itself (credential/config failures, DB failures, malformed bodies), classified
/// as `fromMessage` does. Not redacted: these texts are the server's own, never user secrets.
pub fn server_error(message: &str) -> ChatError {
    const PROVIDER_CONFIG_PHRASES: [&str; 5] = [
        "requires apikey",
        "requires baseurl",
        "requires oauth",
        "not authenticated with",
        "is required for",
    ];
    let trimmed = message.trim();
    let lower = trimmed.to_lowercase();
    let details = (!trimmed.is_empty()).then(|| truncate_details(trimmed.to_owned()));
    let (code, title, fallback, retryable) = if PROVIDER_CONFIG_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
    {
        (
            ChatErrorCode::ProviderConfig,
            "Provider not configured",
            "This provider is not fully configured.",
            false,
        )
    } else {
        (
            ChatErrorCode::Unknown,
            "Something went wrong",
            "An unexpected error occurred.",
            true,
        )
    };
    ChatError {
        code,
        title: title.to_owned(),
        message: if trimmed.is_empty() {
            fallback.to_owned()
        } else {
            trimmed.to_owned()
        },
        retryable,
        provider: None,
        status_code: None,
        docs_url: None,
        retry_after_seconds: None,
        details,
    }
}

fn db_error(error: &DbError) -> ChatError {
    server_error(&error.to_string())
}

/// `DETAILS_MAX_LENGTH` (20 000 UTF-16 units) with an ellipsis, as `toDetails` caps.
fn truncate_details(text: String) -> String {
    const DETAILS_MAX_LENGTH: usize = 20_000;
    if text.encode_utf16().count() <= DETAILS_MAX_LENGTH {
        return text;
    }
    let mut units = 0;
    let mut cut: String = text
        .chars()
        .take_while(|ch| {
            units += ch.len_utf16();
            units <= DETAILS_MAX_LENGTH
        })
        .collect();
    cut.push('…');
    cut
}
