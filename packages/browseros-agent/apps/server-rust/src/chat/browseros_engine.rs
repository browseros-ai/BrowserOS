//! `BrowserOsEngine`: the built-in agent behind the [`ChatEngine`] seam. Port of the session half
//! of TS `chat-service.ts` (`processBrowserOsMessage`, `rebuildSession`, `createBrowserOsAgent`,
//! `deleteSession`) and `agent/session-store.ts`.
//!
//! The engine caches one [`Session`] per conversation: a built `browseros_agent::Agent`, the
//! conversation's tool lease for loopback `/mcp`, and for scheduled runs the background
//! `about:blank` page. A session is rebuilt when the MCP set, workspace or mode changes, and that
//! turn's prompt is prefixed with one `[Context: …]` notice per change. The engine never stores
//! display history (`ConversationHistory` owns it). It turns the raw user text into the
//! model-facing `prompt_text`, and the agent derives the sanitized model copy from the display
//! history on every turn.
//!
//! ```text
//! start_turn ─► server-owned task, conversation slot locked
//!   ├─ no session ───────► [scheduled: open about:blank] ─► mint lease ─► connect /mcp (required)
//!   │                        + custom MCP (optional) ─► Agent::build
//!   ├─ inputs changed ───► notices; revoke lease + dispose agent ─► build again (same page)
//!   └─ inputs unchanged ─► reuse
//!   then: refresh the lease's browser context ─► prompt_text = notices + format_user_message
//!         ─► Agent::run_turn(display history, prompt_text) ─► stream (releases the scheduled page)
//! ```

use super::{
    engine::{ChatEngine, ChatRejection, ResolvedLlm, Turn, TurnTarget},
    request::{BrowserContext, ChatMode, ChatTurnRequest, ResolvedBrowserContext},
};
use crate::{
    klavis::ProxyState,
    mcp::{LeaseError, LeaseSource, LeaseSpec, ToolLease, ToolLeases},
};
use browseros_agent::{
    Agent, AgentBuildError, AgentConfig, TurnInput,
    llm::{self, ChatError, HttpStack, LanguageModel, LlmConfig, LlmConfigError, ModelError},
    prompt::{self, BrowserContextView, PromptInputs, SessionInputs, TabView},
    tools::{self, FsScope, McpEndpoint, McpTransport, OutputGrants, SourceSpec, ToolSource},
};
use futures_core::Stream;
use std::{
    collections::HashMap,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};
use ui_message_stream::{ChunkStream, Role, UiChunk, UiMessage, UiPart};
use uuid::Uuid;

/// The header the loopback MCP client authenticates with (TS `BROWSEROS_TOOL_LEASE_HEADER`).
const LEASE_HEADER: &str = "X-BrowserOS-Internal-Lease";
/// Client label of the internal MCP source. A failure to connect it is fatal to the turn.
const INTERNAL_MCP_LABEL: &str = "browseros";
/// `AGENT_LIMITS.MAX_TURNS` (`stepCountIs(100)`).
const MAX_STEPS: u32 = 100;
const SCHEDULED_PAGE_URL: &str = "about:blank";
const SCHEDULED_PAGE_TITLE: &str = "Scheduled Task";

/// Builds one `LanguageModel` per session from resolved credentials.
pub type ModelFactory =
    Arc<dyn Fn(&LlmConfig) -> Result<Arc<dyn LanguageModel>, LlmConfigError> + Send + Sync>;

/// The production model factory. `BROWSEROS_USE_MOCK_LLM=true` swaps every model for
/// `ScriptedModel::mock()`.
pub fn production_models(http: HttpStack, use_mock_llm: bool) -> ModelFactory {
    Arc::new(move |config: &LlmConfig| {
        if use_mock_llm {
            Ok(Arc::new(llm::ScriptedModel::mock()) as Arc<dyn LanguageModel>)
        } else {
            llm::create_language_model(config, &http)
        }
    })
}

/// The live Klavis/Strata proxy state name (`klavis.getProxyStatus().state`). `None` means no
/// Klavis service, which the MCP key renders as `klavis:disabled`.
pub type KlavisStatus = Arc<dyn Fn() -> Option<String> + Send + Sync>;

/// Maps the proxy state onto the TS state names baked into the MCP key. The key is compared
/// across turns, and `klavis:ready` selects a dedicated notice, so these strings must not drift.
pub fn klavis_state_name(state: &ProxyState) -> &'static str {
    match state {
        ProxyState::Disconnected => "disabled",
        ProxyState::Connecting => "connecting",
        ProxyState::Connected { .. } => "ready",
    }
}

/// A conversation's capability on loopback `/mcp`. It is bound to one conversation, read-only
/// policy and browser context, and admission consults it on every MCP request.
pub trait SessionLease: Send + Sync {
    fn token(&self) -> &str;
    /// Refreshed every turn, because the MCP client outlives the active window it was built for.
    fn update_browser_context(&self, ctx: Option<&BrowserContext>);
    /// The tool-output files this lease's `filesystem_read` may open.
    fn output_grants(&self) -> Arc<dyn OutputGrants>;
    /// Takes effect synchronously, so a slow MCP close after it cannot leave the token usable.
    fn revoke(&self);
}

/// Mints session leases. The production adapter is [`RegistryLeases`]; tests record mints.
#[async_trait::async_trait]
pub trait LeaseIssuer: Send + Sync {
    async fn mint(&self, spec: LeaseSpec) -> Result<Box<dyn SessionLease>, LeaseError>;
}

/// Production [`LeaseIssuer`]: the server's `ToolLeases` registry, which `/mcp` admission reads.
pub struct RegistryLeases(pub Arc<ToolLeases>);

struct RegisteredLease {
    lease: ToolLease,
    registry: Arc<ToolLeases>,
}

#[async_trait::async_trait]
impl LeaseIssuer for RegistryLeases {
    async fn mint(&self, spec: LeaseSpec) -> Result<Box<dyn SessionLease>, LeaseError> {
        let lease = self.0.mint(spec).await?;
        Ok(Box::new(RegisteredLease {
            lease,
            registry: self.0.clone(),
        }))
    }
}

impl SessionLease for RegisteredLease {
    fn token(&self) -> &str {
        self.lease.token()
    }

    fn update_browser_context(&self, ctx: Option<&BrowserContext>) {
        self.lease.update_browser_context(ctx);
    }

    fn output_grants(&self) -> Arc<dyn OutputGrants> {
        self.lease.output_grants()
    }

    fn revoke(&self) {
        self.registry.revoke(&self.lease);
    }
}

/// The browser operations a scheduled run needs: one background page per scheduled session,
/// closed when its turn ends or the conversation is deleted. Every failure is non-fatal.
#[async_trait::async_trait]
pub trait BackgroundPages: Send + Sync {
    /// Opens `url` without activating it (TS `browser.newPage(url, { background: true })`) and
    /// returns its MCP page id.
    async fn open_background(&self, url: &str) -> anyhow::Result<u32>;
    /// The window holding `page_id`, from the live page list.
    async fn window_of(&self, page_id: u32) -> anyhow::Result<Option<i64>>;
    async fn close(&self, page_id: u32) -> anyhow::Result<()>;
}

/// The [`BackgroundPages`] adapter used until the shared host's browser is wired in. Scheduled
/// runs then fall back to the request's browser context, as TS does when `newPage` fails.
pub struct NoBrowser;

#[async_trait::async_trait]
impl BackgroundPages for NoBrowser {
    async fn open_background(&self, url: &str) -> anyhow::Result<u32> {
        anyhow::bail!("browser is not connected; cannot open {url}")
    }

    async fn window_of(&self, page_id: u32) -> anyhow::Result<Option<i64>> {
        anyhow::bail!("browser is not connected; cannot look up page {page_id}")
    }

    async fn close(&self, page_id: u32) -> anyhow::Result<()> {
        anyhow::bail!("browser is not connected; cannot close page {page_id}")
    }
}

/// Everything the engine is built from.
pub struct BrowserOsEngineDeps {
    pub models: ModelFactory,
    pub leases: Arc<dyn LeaseIssuer>,
    pub pages: Arc<dyn BackgroundPages>,
    /// This server's port. The internal MCP source is `http://127.0.0.1:<port>/mcp`.
    pub mcp_port: u16,
    pub klavis_status: KlavisStatus,
    /// The Klavis connector catalog names that `suggest_app_connection.appName` enumerates.
    pub connector_names: Vec<String>,
}

pub struct BrowserOsEngine {
    inner: Arc<Inner>,
}

struct Inner {
    deps: BrowserOsEngineDeps,
    /// One slot per conversation. A slot's async lock serializes preparation against `forget`.
    /// Turns of one conversation never overlap, because `ConversationRuns` admits one run at a time.
    slots: Mutex<HashMap<Uuid, Slot>>,
    /// Sessions currently built (TS `sessionStore.count()`, reported by `DELETE /chat/{id}`).
    live: AtomicUsize,
}

type Slot = Arc<tokio::sync::Mutex<Option<Session>>>;

/// TS `AgentSession`: one built agent, the lease it was built with, and its change-detection
/// inputs.
struct Session {
    agent: Agent,
    lease: Box<dyn SessionLease>,
    /// What the agent was built from. A turn whose inputs differ rebuilds it.
    inputs: SessionInputs,
    /// The latest context given to the lease. Scheduled turns reuse it instead of the request's
    /// context, which keeps them on their background page.
    context: TurnContext,
    /// The scheduled run's background page. It is shared with that turn's stream, and whichever
    /// finishes first (the turn or `forget`) closes it, exactly once.
    scheduled_page: PageSlot,
}

/// A browser context in both shapes the engine needs: the contract form (with page ids) that
/// the lease reads, and the resolved view that the prompt renders.
#[derive(Clone, Default)]
struct TurnContext {
    raw: Option<BrowserContext>,
    view: Option<BrowserContextView>,
}

#[derive(Clone, Default)]
struct PageSlot(Arc<Mutex<Option<u32>>>);

impl PageSlot {
    fn holding(page_id: u32) -> Self {
        Self(Arc::new(Mutex::new(Some(page_id))))
    }

    fn take(&self) -> Option<u32> {
        self.0.lock().ok().and_then(|mut page| page.take())
    }
}

impl BrowserOsEngine {
    pub fn new(deps: BrowserOsEngineDeps) -> Self {
        Self {
            inner: Arc::new(Inner {
                deps,
                slots: Mutex::new(HashMap::new()),
                live: AtomicUsize::new(0),
            }),
        }
    }

    /// Built-in sessions currently cached (TS `sessionStore.count()`).
    pub fn session_count(&self) -> usize {
        self.inner.live.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl ChatEngine for BrowserOsEngine {
    async fn start_turn(&self, turn: Turn) -> Result<ChunkStream, ChatRejection> {
        // Preparation runs in a server-owned task. A JS promise cannot be cancelled, but this
        // future can be dropped (e.g. the POST connection closes mid-prepare). That must not strand
        // a minted lease, an open background page or a half-replaced session.
        let inner = self.inner.clone();
        tokio::spawn(async move { inner.prepare(turn).await })
            .await
            .unwrap_or_else(|error| Err(prepare_error(format!("turn preparation failed: {error}"))))
    }

    async fn forget(&self, conversation_id: Uuid) -> bool {
        let slot = self
            .inner
            .slots
            .lock()
            .ok()
            .and_then(|mut slots| slots.remove(&conversation_id));
        let Some(slot) = slot else {
            return false;
        };
        // Waits for an in-flight preparation, so a session it is still building is disposed too.
        let Some(session) = self.inner.take(&mut *slot.lock().await) else {
            return false;
        };
        if let Some(page_id) = session.scheduled_page.take() {
            close_page_later(self.inner.deps.pages.clone(), page_id, conversation_id);
        }
        dispose(session).await;
        tracing::info!(
            %conversation_id,
            remaining_sessions = self.session_count(),
            "built-in session deleted"
        );
        true
    }
}

impl Inner {
    fn slot(&self, conversation_id: Uuid) -> Slot {
        match self.slots.lock() {
            Ok(mut slots) => slots.entry(conversation_id).or_default().clone(),
            // A poisoned map only loses caching: this turn builds a session nobody else sees.
            Err(_) => Slot::default(),
        }
    }

    fn take(&self, slot: &mut Option<Session>) -> Option<Session> {
        let session = slot.take()?;
        self.live.fetch_sub(1, Ordering::SeqCst);
        Some(session)
    }

    fn put(&self, slot: &mut Option<Session>, session: Session) {
        *slot = Some(session);
        self.live.fetch_add(1, Ordering::SeqCst);
    }

    /// Phase 1 of a turn (TS `processBrowserOsMessage` up to `createAgentUIStream`).
    async fn prepare(self: Arc<Self>, turn: Turn) -> Result<ChunkStream, ChatRejection> {
        let Turn {
            conversation_id,
            run,
            target,
            request,
            history,
            browser_context,
        } = turn;
        let TurnTarget::BrowserOs(llm) = target else {
            return Err(prepare_error(
                "the built-in engine was given an ACP target".to_owned(),
            ));
        };
        let request_context =
            TurnContext::from_request(request.browser_context.as_ref(), browser_context.as_ref());
        let next = SessionInputs {
            mcp_server_key: prompt::mcp_server_key(
                key_view(request.browser_context.as_ref()).as_ref(),
                (self.deps.klavis_status)().as_deref(),
            ),
            working_dir: request.user_working_dir.clone(),
            chat_mode: request.mode == ChatMode::Chat,
        };
        let user_text = last_user_text(&history);

        let slot = self.slot(conversation_id);
        let mut guard = slot.lock().await;

        // One rebuild reflects every change, with one notice per change. The notices compare the
        // inputs the old agent was built from with this turn's inputs, so they must be computed
        // before the old session is replaced.
        let mut notices = Vec::new();
        if guard.as_ref().is_some_and(|session| session.inputs != next)
            && let Some(old) = self.take(&mut *guard)
        {
            notices = prompt::context_notices(&old.inputs, &next);
            tracing::info!(
                %conversation_id,
                mcp_changed = old.inputs.mcp_server_key != next.mcp_server_key,
                workspace_changed = old.inputs.working_dir != next.working_dir,
                mode_changed = old.inputs.chat_mode != next.chat_mode,
                "rebuilding built-in session for mid-conversation input changes"
            );
            let context = if request.is_scheduled_task {
                old.context.clone()
            } else {
                request_context.clone()
            };
            let page = old.scheduled_page.clone();
            dispose(old).await;
            let session = self
                .build_session(
                    conversation_id,
                    &llm,
                    &request,
                    &user_text,
                    next.clone(),
                    context,
                    page,
                )
                .await?;
            self.put(&mut *guard, session);
        }

        if guard.is_none() {
            let (context, page) = if request.is_scheduled_task {
                self.open_scheduled_page(conversation_id, request_context.clone())
                    .await
            } else {
                (request_context.clone(), PageSlot::default())
            };
            let session = self
                .build_session(
                    conversation_id,
                    &llm,
                    &request,
                    &user_text,
                    next,
                    context,
                    page,
                )
                .await?;
            self.put(&mut *guard, session);
        }
        let Some(session) = guard.as_mut() else {
            return Err(prepare_error("built-in session is missing".to_owned()));
        };

        // Scheduled pages already carry MCP page ids from `open_background`. Re-resolving would
        // treat them as Chromium tab ids, so scheduled turns keep the session's context.
        let context = if request.is_scheduled_task {
            session.context.clone()
        } else {
            request_context
        };
        session.lease.update_browser_context(context.raw.as_ref());
        session.context = context;

        let prompt_text = prompt_text(
            &notices,
            &user_text,
            session.context.view.as_ref(),
            &request,
        );
        let chunks = session.agent.run_turn(
            TurnInput {
                history,
                prompt_text,
                message_id: Uuid::new_v4().to_string(),
            },
            run.cancel().clone(),
        );
        Ok(Box::pin(ReleasesPage {
            chunks,
            page: session.scheduled_page.clone(),
            pages: self.deps.pages.clone(),
            conversation_id,
        }))
    }

    /// TS: open `about:blank` in the background and point the context's active tab at it. On
    /// failure the run continues on the request's context.
    async fn open_scheduled_page(
        &self,
        conversation_id: Uuid,
        request_context: TurnContext,
    ) -> (TurnContext, PageSlot) {
        let page_id = match self.deps.pages.open_background(SCHEDULED_PAGE_URL).await {
            Ok(page_id) => page_id,
            Err(error) => {
                tracing::warn!(
                    %conversation_id,
                    error = %error,
                    "failed to create scheduled page, using default browser context"
                );
                return (request_context, PageSlot::default());
            }
        };
        let window_id = match self.deps.pages.window_of(page_id).await {
            Ok(window_id) => window_id,
            Err(error) => {
                tracing::warn!(
                    %conversation_id,
                    page_id,
                    error = %error,
                    "failed to look up scheduled page metadata"
                );
                None
            }
        };
        tracing::info!(%conversation_id, page_id, ?window_id, "created background page for scheduled task");
        (
            scheduled_context(request_context.raw, page_id, window_id),
            PageSlot::holding(page_id),
        )
    }

    /// TS `createBrowserOsAgent`: mints the lease before the loopback client exists, and revokes
    /// it if the agent cannot be built. A failed build also releases the scheduled page.
    #[allow(clippy::too_many_arguments)]
    async fn build_session(
        &self,
        conversation_id: Uuid,
        llm: &ResolvedLlm,
        request: &ChatTurnRequest,
        user_text: &str,
        inputs: SessionInputs,
        context: TurnContext,
        scheduled_page: PageSlot,
    ) -> Result<Session, ChatRejection> {
        let built = async {
            let model = (self.deps.models)(&llm.config)
                .map_err(|error| prepare_error(error.to_string()))?;
            let lease = self
                .deps
                .leases
                .mint(LeaseSpec {
                    conversation_id,
                    read_only: inputs.chat_mode,
                    source: LeaseSource::Chat,
                    client_slug: "browseros",
                    task_label: task_label(user_text),
                    auto_group: !request.is_scheduled_task,
                    browser_context: context.raw.clone(),
                })
                .await
                .map_err(|error| prepare_error(error.to_string()))?;
            match self
                .build_agent(model, llm, request, &context, lease.as_ref())
                .await
            {
                Ok(agent) => Ok((agent, lease)),
                Err(error) => {
                    lease.revoke();
                    Err(error)
                }
            }
        }
        .await;
        match built {
            Ok((agent, lease)) => {
                tracing::info!(
                    %conversation_id,
                    provider = llm.info.provider.as_str(),
                    model = %llm.info.model_id,
                    tool_count = agent.tool_names().len(),
                    "built-in session created"
                );
                Ok(Session {
                    agent,
                    lease,
                    inputs,
                    context,
                    scheduled_page,
                })
            }
            Err(rejection) => {
                if let Some(page_id) = scheduled_page.take() {
                    close_page_later(self.deps.pages.clone(), page_id, conversation_id);
                }
                Err(rejection)
            }
        }
    }

    /// TS `AiSdkAgent.create`'s tool assembly. The sources go in merge order (internal MCP →
    /// custom MCP → filesystem → nudges). `Agent::build` drops custom tools whose names the
    /// required internal source owns.
    async fn build_agent(
        &self,
        model: Arc<dyn LanguageModel>,
        llm: &ResolvedLlm,
        request: &ChatTurnRequest,
        context: &TurnContext,
        lease: &dyn SessionLease,
    ) -> Result<Agent, ChatRejection> {
        let chat_mode = request.mode == ChatMode::Chat;
        let internal = tools::connect_mcp(internal_mcp_endpoint(
            self.deps.mcp_port,
            lease.token(),
            chat_mode,
        )?)
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, "failed to connect required MCP client");
            prepare_error(
                AgentBuildError::RequiredSourceUnavailable(INTERNAL_MCP_LABEL.to_owned())
                    .to_string(),
            )
        })?;

        let mut sources = vec![SourceSpec {
            source: internal,
            required: true,
        }];
        sources.extend(
            connect_custom_mcp(context.raw.as_ref())
                .await
                .into_iter()
                .map(|source| SourceSpec {
                    source,
                    required: false,
                }),
        );
        let workspace = workspace_dir(request.user_working_dir.as_ref(), chat_mode);
        sources.push(SourceSpec {
            source: tools::filesystem_tools(match workspace {
                Some(dir) => FsScope::Workspace(dir.clone()),
                None => FsScope::OutputOnly(lease.output_grants()),
            }),
            required: false,
        });
        // Scheduled and read-only turns get neither the nudge tools nor the nudge prompt section.
        if !request.is_scheduled_task && !chat_mode {
            sources.push(SourceSpec {
                source: tools::nudge_tools(self.deps.connector_names.clone()),
                required: false,
            });
        }

        Agent::build(AgentConfig {
            model,
            info: llm.info.clone(),
            reasoning: llm.reasoning.clone(),
            prompt: PromptInputs {
                chat_mode,
                is_scheduled_task: request.is_scheduled_task,
                origin: request.origin,
                workspace_dir: request.user_working_dir.clone(),
                connected_apps: context
                    .raw
                    .as_ref()
                    .and_then(|ctx| ctx.enabled_mcp_servers.clone())
                    .unwrap_or_default(),
                declined_apps: request.declined_apps.clone(),
                user_system_prompt: request.user_system_prompt.clone(),
                active_tab_page_id: context
                    .view
                    .as_ref()
                    .and_then(|view| view.active_tab.as_ref())
                    .and_then(|tab| tab.page_id),
            },
            sources,
            max_steps: MAX_STEPS,
        })
        .await
        .map_err(|error| prepare_error(error.to_string()))
    }
}

/// Revokes before the asynchronous MCP close (TS `SessionStore.delete`), so a slow close
/// cannot leave the old capability usable. The scheduled page is not touched, because a rebuild
/// keeps it.
async fn dispose(session: Session) {
    session.lease.revoke();
    session.agent.dispose().await;
}

/// `http://127.0.0.1:<port>/mcp?structured=1[&read_only=1]` with the lease header
/// (TS `buildInternalBrowserMcpSpec`).
fn internal_mcp_endpoint(
    port: u16,
    lease_token: &str,
    read_only: bool,
) -> Result<McpEndpoint, ChatRejection> {
    let query = if read_only {
        "structured=1&read_only=1"
    } else {
        "structured=1"
    };
    let url = url::Url::parse(&format!("http://127.0.0.1:{port}/mcp?{query}"))
        .map_err(|error| prepare_error(format!("invalid loopback MCP URL: {error}")))?;
    Ok(McpEndpoint {
        label: INTERNAL_MCP_LABEL.to_owned(),
        url,
        headers: vec![(LEASE_HEADER.to_owned(), lease_token.to_owned())],
        transport: McpTransport::StreamableHttp,
    })
}

/// The context's `customMcpServers`, connected in parallel with transport detection. They are
/// optional: a server that fails is skipped with a warning (TS `createMcpClients`).
async fn connect_custom_mcp(ctx: Option<&BrowserContext>) -> Vec<Arc<dyn ToolSource>> {
    let servers = ctx
        .and_then(|ctx| ctx.custom_mcp_servers.as_deref())
        .unwrap_or_default();
    let connects = servers.iter().map(|server| async move {
        let label = format!("custom-{}", server.name);
        let url = match url::Url::parse(&server.url) {
            Ok(url) => url,
            Err(error) => {
                tracing::warn!(name = %label, error = %error, "failed to connect MCP client, skipping");
                return None;
            }
        };
        let endpoint = McpEndpoint {
            label: label.clone(),
            url,
            headers: Vec::new(),
            transport: McpTransport::Detect,
        };
        match tools::connect_mcp(endpoint).await {
            Ok(source) => Some(source),
            Err(error) => {
                tracing::warn!(name = %label, error = %error, "failed to connect MCP client, skipping");
                None
            }
        }
    });
    futures_util::future::join_all(connects)
        .await
        .into_iter()
        .flatten()
        .collect()
}

/// The full workspace toolset needs a non-empty directory and agent mode. Otherwise only the
/// output-only `filesystem_read` is available (TS `buildAgentFilesystemToolSet`, where `""` is
/// falsy).
fn workspace_dir(dir: Option<&PathBuf>, chat_mode: bool) -> Option<&PathBuf> {
    dir.filter(|dir| !chat_mode && !dir.as_os_str().is_empty())
}

/// TS: `[Context: …]` lines, a blank line, then `formatUserMessage` (browser-context header,
/// `<selected_text>`, `<USER_QUERY>`). This is the text the model sees for this turn's user message.
fn prompt_text(
    notices: &[String],
    user_text: &str,
    view: Option<&BrowserContextView>,
    request: &ChatTurnRequest,
) -> String {
    let body = prompt::format_user_message(
        user_text,
        view,
        request.selected_text.as_deref(),
        request
            .selected_text_source
            .as_ref()
            .map(|source| (source.url.as_str(), source.title.as_str())),
    );
    if notices.is_empty() {
        return body;
    }
    let prefix = notices
        .iter()
        .map(|notice| format!("[Context: {notice}]"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{prefix}\n\n{body}")
}

/// The raw text of this turn's user message (the last history entry). TS appends exactly one
/// text part holding `request.message`.
fn last_user_text(history: &[UiMessage]) -> String {
    history
        .last()
        .filter(|message| message.role == Role::User)
        .map(|message| {
            message
                .parts
                .iter()
                .filter_map(|part| match part {
                    UiPart::Text(text) => Some(text.text.as_str()),
                    _ => None,
                })
                .collect::<String>()
        })
        .unwrap_or_default()
}

/// The `<task>` half of the session's tab-group title `browseros/<task>`: the first five
/// lowercase letter/number words joined by `-`, cut to 40 UTF-16 units, or `task`. Port of TS
/// `conversationTabGroup`.
fn task_label(message: &str) -> String {
    let lower = message.to_lowercase();
    let words = word_runs(&lower).take(5).collect::<Vec<_>>().join("-");
    let units = words.encode_utf16().take(40).collect::<Vec<_>>();
    let task = String::from_utf16_lossy(&units);
    if task.is_empty() {
        "task".to_owned()
    } else {
        task
    }
}

/// Maximal runs of `[\p{L}\p{N}]`.
fn word_runs(text: &str) -> impl Iterator<Item = &str> {
    text.split(|ch: char| !is_letter_or_number(ch))
        .filter(|word| !word.is_empty())
}

fn is_letter_or_number(ch: char) -> bool {
    // `char::is_alphabetic` also admits marks and letter numbers. Those are rare in task labels,
    // and they only change where the label splits a word.
    ch.is_alphabetic() || ch.is_numeric()
}

/// The MCP key only reads the enabled connector names and custom server URLs, which come from
/// the request as sent (TS `buildMcpServerKey`).
fn key_view(ctx: Option<&BrowserContext>) -> Option<BrowserContextView> {
    ctx.map(|ctx| BrowserContextView {
        enabled_mcp_servers: ctx.enabled_mcp_servers.clone().unwrap_or_default(),
        custom_mcp_urls: ctx
            .custom_mcp_servers
            .iter()
            .flatten()
            .map(|server| server.url.clone())
            .collect(),
        ..BrowserContextView::default()
    })
}

impl TurnContext {
    /// The request's context, with the page ids that `ChatService` resolved copied onto its tabs.
    /// The lease then sees the same ids as the prompt.
    fn from_request(raw: Option<&BrowserContext>, view: Option<&ResolvedBrowserContext>) -> Self {
        let raw = raw.map(|raw| {
            let mut raw = raw.clone();
            if let Some(view) = view {
                let resolved = view
                    .active_tab
                    .iter()
                    .chain(&view.selected_tabs)
                    .filter_map(|tab| Some((tab.id, tab.page_id?)))
                    .collect::<HashMap<_, _>>();
                let fill = |tab: &mut server_api::BrowserTab| {
                    if let Some(page_id) = resolved.get(&(tab.id as i64)) {
                        tab.page_id = Some(f64::from(*page_id));
                    }
                };
                raw.active_tab.iter_mut().for_each(fill);
                raw.selected_tabs.iter_mut().flatten().for_each(fill);
            }
            raw
        });
        Self {
            raw,
            view: view.cloned(),
        }
    }
}

/// TS: the request context with `windowId` set to the page's window, `selectedTabs`/`tabs`
/// cleared, and the active tab replaced by the background page (`id` = `pageId`, title
/// `Scheduled Task`). Enabled and custom MCP servers carry over.
fn scheduled_context(
    raw: Option<BrowserContext>,
    page_id: u32,
    window_id: Option<i64>,
) -> TurnContext {
    let mut raw = raw.unwrap_or_default();
    let mut tab = server_api::BrowserTab::new(f64::from(page_id));
    tab.page_id = Some(f64::from(page_id));
    tab.url = Some(SCHEDULED_PAGE_URL.to_owned());
    tab.title = Some(SCHEDULED_PAGE_TITLE.to_owned());
    raw.window_id = window_id.map(|id| id as f64);
    raw.active_tab = Some(tab);
    raw.selected_tabs = None;
    raw.tabs = None;
    let view = BrowserContextView {
        window_id,
        active_tab: Some(TabView {
            id: i64::from(page_id),
            page_id: Some(page_id),
            url: Some(SCHEDULED_PAGE_URL.to_owned()),
            title: Some(SCHEDULED_PAGE_TITLE.to_owned()),
        }),
        selected_tabs: Vec::new(),
        enabled_mcp_servers: raw.enabled_mcp_servers.clone().unwrap_or_default(),
        custom_mcp_urls: raw
            .custom_mcp_servers
            .iter()
            .flatten()
            .map(|server| server.url.clone())
            .collect(),
    };
    TurnContext {
        raw: Some(raw),
        view: Some(view),
    }
}

/// A turn's chunk stream that releases the scheduled page when the turn ends. That covers a
/// normal end, an error or abort, and the run dropping the stream. TS closes the page in
/// `onFinish`, which a cancelled reader can skip, so this covers more cases than TS does.
struct ReleasesPage {
    chunks: ChunkStream,
    page: PageSlot,
    pages: Arc<dyn BackgroundPages>,
    conversation_id: Uuid,
}

impl ReleasesPage {
    fn release(&self) {
        if let Some(page_id) = self.page.take() {
            close_page_later(self.pages.clone(), page_id, self.conversation_id);
        }
    }
}

impl Stream for ReleasesPage {
    type Item = UiChunk;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<UiChunk>> {
        let next = self.chunks.as_mut().poll_next(cx);
        if let Poll::Ready(None) = next {
            self.release();
        }
        next
    }
}

impl Drop for ReleasesPage {
    fn drop(&mut self) {
        self.release();
    }
}

/// Fire-and-forget close (TS `closeScheduledPage`): a failure is logged, never surfaced.
fn close_page_later(pages: Arc<dyn BackgroundPages>, page_id: u32, conversation_id: Uuid) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        tracing::warn!(%conversation_id, page_id, "no runtime to close scheduled page");
        return;
    };
    runtime.spawn(async move {
        if let Err(error) = pages.close(page_id).await {
            tracing::warn!(%conversation_id, page_id, error = %error, "failed to close scheduled page");
        }
    });
}

/// A pre-stream failure as the 500 chat envelope. TS throws a plain `Error`, and the app's
/// `onError` classifies it by message without a provider.
fn prepare_error(message: String) -> ChatRejection {
    let error: ChatError = llm::to_chat_error(&ModelError::Config(message), None);
    ChatRejection::Internal(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use server_api::{BrowserTab, CustomMcpServer};

    fn tab(id: f64, title: &str) -> BrowserTab {
        let mut tab = BrowserTab::new(id);
        tab.title = Some(title.to_owned());
        tab
    }

    #[test]
    fn task_label_takes_five_lowercase_words() {
        assert_eq!(
            task_label("Find the BEST flights, to NYC & back!"),
            "find-the-best-flights-to"
        );
        assert_eq!(task_label("  ?? !! "), "task");
        assert_eq!(task_label("Résumé für 2026"), "résumé-für-2026");
        let long = "abcdefghijklmnopqrstuvwxyz abcdefghijklmnopqrstuvwxyz";
        assert_eq!(task_label(long).encode_utf16().count(), 40);
    }

    #[test]
    fn internal_endpoint_carries_lease_and_read_only_query() -> Result<(), String> {
        let endpoint = internal_mcp_endpoint(9100, "tok", false).map_err(|e| e.to_string())?;
        assert_eq!(
            endpoint.url.as_str(),
            "http://127.0.0.1:9100/mcp?structured=1"
        );
        assert_eq!(
            endpoint.headers,
            vec![(LEASE_HEADER.to_owned(), "tok".to_owned())]
        );
        assert_eq!(endpoint.transport, McpTransport::StreamableHttp);
        let read_only = internal_mcp_endpoint(9100, "tok", true).map_err(|e| e.to_string())?;
        assert_eq!(
            read_only.url.as_str(),
            "http://127.0.0.1:9100/mcp?structured=1&read_only=1"
        );
        Ok(())
    }

    #[test]
    fn prompt_text_prefixes_one_context_line_per_notice() {
        let request = ChatTurnRequest::default();
        assert_eq!(
            prompt_text(&[], "hi", None, &request),
            "<USER_QUERY>\nhi\n</USER_QUERY>"
        );
        let notices = vec!["first".to_owned(), "second".to_owned()];
        assert_eq!(
            prompt_text(&notices, "hi", None, &request),
            "[Context: first]\n[Context: second]\n\n<USER_QUERY>\nhi\n</USER_QUERY>"
        );
    }

    #[test]
    fn request_context_gets_resolved_page_ids() {
        let mut raw = BrowserContext::new();
        raw.active_tab = Some(tab(10.0, "A"));
        raw.selected_tabs = Some(vec![tab(11.0, "B"), tab(12.0, "C")]);
        let view = BrowserContextView {
            active_tab: Some(TabView {
                id: 10,
                page_id: Some(3),
                ..TabView::default()
            }),
            selected_tabs: vec![TabView {
                id: 11,
                page_id: Some(4),
                ..TabView::default()
            }],
            ..BrowserContextView::default()
        };
        let context = TurnContext::from_request(Some(&raw), Some(&view));
        let raw = context.raw.unwrap_or_default();
        assert_eq!(raw.active_tab.and_then(|tab| tab.page_id), Some(3.0));
        let selected = raw.selected_tabs.unwrap_or_default();
        assert_eq!(selected[0].page_id, Some(4.0));
        assert_eq!(selected[1].page_id, None);
    }

    #[test]
    fn scheduled_context_points_at_the_background_page() {
        let mut raw = BrowserContext::new();
        raw.window_id = Some(1.0);
        raw.active_tab = Some(tab(10.0, "User tab"));
        raw.selected_tabs = Some(vec![tab(11.0, "B")]);
        raw.tabs = Some(vec![tab(12.0, "C")]);
        raw.enabled_mcp_servers = Some(vec!["Slack".to_owned()]);
        raw.custom_mcp_servers = Some(vec![CustomMcpServer::new(
            "x".to_owned(),
            "http://x/mcp".to_owned(),
        )]);
        let context = scheduled_context(Some(raw), 77, Some(5));
        let raw = context.raw.unwrap_or_default();
        assert_eq!(raw.window_id, Some(5.0));
        assert_eq!(raw.selected_tabs, None);
        assert_eq!(raw.tabs, None);
        let active = raw.active_tab.unwrap_or_else(|| BrowserTab::new(0.0));
        assert_eq!((active.id, active.page_id), (77.0, Some(77.0)));
        assert_eq!(active.title.as_deref(), Some("Scheduled Task"));
        assert_eq!(active.url.as_deref(), Some("about:blank"));
        assert_eq!(raw.enabled_mcp_servers, Some(vec!["Slack".to_owned()]));
        let view = context.view.unwrap_or_default();
        assert_eq!(
            prompt::format_user_message("go", Some(&view), None, None),
            "## Browser Context\n**Window ID:** 5\n**Active Tab:** Tab 77 (Page ID: 77) - \"Scheduled Task\" (about:blank)\n\n---\n\n<USER_QUERY>\ngo\n</USER_QUERY>"
        );
        assert_eq!(view.custom_mcp_urls, vec!["http://x/mcp".to_owned()]);
    }

    #[test]
    fn scheduled_context_without_window_or_request_context() {
        let context = scheduled_context(None, 9, None);
        let raw = context.raw.unwrap_or_default();
        assert_eq!(raw.window_id, None);
        assert_eq!(raw.active_tab.map(|tab| tab.id), Some(9.0));
    }

    #[test]
    fn mcp_key_reads_the_request_as_sent() {
        let mut raw = BrowserContext::new();
        raw.enabled_mcp_servers = Some(vec!["Slack".to_owned(), "Gmail".to_owned()]);
        raw.custom_mcp_servers = Some(vec![CustomMcpServer::new(
            "x".to_owned(),
            "http://x/mcp".to_owned(),
        )]);
        let key = prompt::mcp_server_key(key_view(Some(&raw)).as_ref(), Some("ready"));
        assert_eq!(key, "klavis:ready,Gmail,Slack,http://x/mcp");
        assert_eq!(prompt::mcp_server_key(key_view(None).as_ref(), None), "");
        assert_eq!(
            prompt::mcp_server_key(key_view(Some(&BrowserContext::new())).as_ref(), None),
            ""
        );
    }

    #[test]
    fn last_user_text_reads_only_a_trailing_user_message() {
        let user = UiMessage {
            id: "u".to_owned(),
            role: Role::User,
            metadata: None,
            parts: vec![UiPart::Text(ui_message_stream::TextPart {
                text: "hello".to_owned(),
                state: None,
                provider_metadata: None,
            })],
        };
        assert_eq!(last_user_text(std::slice::from_ref(&user)), "hello");
        let assistant = UiMessage {
            role: Role::Assistant,
            ..user.clone()
        };
        assert_eq!(last_user_text(&[user, assistant]), "");
        assert_eq!(last_user_text(&[]), "");
    }

    #[test]
    fn workspace_needs_a_non_empty_dir_in_agent_mode() {
        let dir = PathBuf::from("/w");
        assert_eq!(workspace_dir(Some(&dir), false), Some(&dir));
        assert_eq!(workspace_dir(Some(&dir), true), None);
        assert_eq!(workspace_dir(Some(&PathBuf::new()), false), None);
        assert_eq!(workspace_dir(None, false), None);
    }

    #[test]
    fn klavis_state_names_match_ts() {
        assert_eq!(klavis_state_name(&ProxyState::Disconnected), "disabled");
        assert_eq!(klavis_state_name(&ProxyState::Connecting), "connecting");
        assert_eq!(
            klavis_state_name(&ProxyState::Connected {
                fingerprint: "f".to_owned()
            }),
            "ready"
        );
    }

    #[tokio::test]
    async fn page_slot_is_released_exactly_once() {
        struct Recorder(Mutex<Vec<u32>>);
        #[async_trait::async_trait]
        impl BackgroundPages for Recorder {
            async fn open_background(&self, _url: &str) -> anyhow::Result<u32> {
                Ok(1)
            }
            async fn window_of(&self, _page_id: u32) -> anyhow::Result<Option<i64>> {
                Ok(None)
            }
            async fn close(&self, page_id: u32) -> anyhow::Result<()> {
                if let Ok(mut closed) = self.0.lock() {
                    closed.push(page_id);
                }
                Ok(())
            }
        }
        let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
        let page = PageSlot::holding(42);
        let stream = ReleasesPage {
            chunks: Box::pin(futures_util::stream::iter([UiChunk::FinishStep])),
            page: page.clone(),
            pages: recorder.clone(),
            conversation_id: Uuid::nil(),
        };
        let chunks = futures_util::StreamExt::collect::<Vec<_>>(stream).await;
        assert_eq!(chunks, vec![UiChunk::FinishStep]);
        tokio::task::yield_now().await;
        assert_eq!(page.take(), None);
        let closed = recorder.0.lock().map(|c| c.clone()).unwrap_or_default();
        assert_eq!(closed, vec![42]);
    }
}
