//! P14 acceptance: `BrowserOsEngine` driving the real built-in agent. A recording
//! `LanguageModel` captures every model request. An rmcp streamable-HTTP server stands in for
//! loopback `/mcp` and records the lease header and query of each request, and fakes record lease
//! mints/revokes and background-page opens/closes. The suite covers session reuse, the one-rebuild-
//! per-change rule with its `[Context: …]` notices, the read-only lease in chat mode, the
//! sanitized prompt copy, the scheduled page lifecycle, nudge removal, and cleanup on every
//! failure path.

use axum::{
    Router,
    extract::{Request, State},
    middleware::{self, Next},
    response::Response,
};
use browseros_agent::{
    ModelInfo,
    llm::{
        LanguageModel, LlmConfig, LlmConfigError, ModelError, ModelEvent, ModelFinishReason,
        ModelRequest, ModelStream, ProviderKind, ReasoningSettings, ReasoningSummary, Usage,
    },
    prompt::{BrowserContextView, TabView},
    tools::OutputGrants,
};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool,
        ToolAnnotations,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        session::local::LocalSessionManager,
        tower::{StreamableHttpServerConfig, StreamableHttpService},
    },
};
use serde_json::{Value, json};
use server_api::{BrowserContext, BrowserTab};
use server_rust::{
    chat::{
        BackgroundPages, BrowserOsEngine, BrowserOsEngineDeps, ChatEngine, ChatRejection,
        ChatTurnRequest, LeaseIssuer, ResolvedLlm, RunHandle, SessionLease, Turn, TurnTarget,
        request::ChatMode,
    },
    mcp::{LeaseError, LeaseSpec},
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use ui_message_stream::{UiChunk, UiMessage};
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

const LEASE_HEADER: &str = "x-browseros-internal-lease";

// ───── the loopback /mcp stand-in ─────

/// One HTTP request seen by the MCP server.
#[derive(Debug, Clone)]
struct McpHit {
    lease: Option<String>,
    query: String,
}

#[derive(Default)]
struct McpProbe {
    hits: Mutex<Vec<McpHit>>,
}

impl McpProbe {
    fn hits(&self) -> Vec<McpHit> {
        self.hits
            .lock()
            .map(|hits| hits.clone())
            .unwrap_or_default()
    }
}

#[derive(Clone)]
struct BrowserTools {
    tools: Arc<Vec<Tool>>,
}

impl ServerHandler for BrowserTools {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("browseros-test", "0.0.0"))
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(
            self.tools.as_ref().clone(),
        )))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        Ok(
            CallToolResult::success(vec![ContentBlock::text(format!("{} ok", request.name))])
                .into(),
        )
    }
}

fn tool(name: &'static str, read_only: bool) -> Tool {
    let schema = json!({ "type": "object", "properties": {} });
    let schema = schema.as_object().cloned().unwrap_or_default();
    Tool::new(name, format!("{name} tool"), schema)
        .with_annotations(ToolAnnotations::new().read_only(read_only))
}

async fn record(State(probe): State<Arc<McpProbe>>, request: Request, next: Next) -> Response {
    let hit = McpHit {
        lease: request
            .headers()
            .get(LEASE_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        query: request.uri().query().unwrap_or_default().to_owned(),
    };
    if let Ok(mut hits) = probe.hits.lock() {
        hits.push(hit);
    }
    next.run(request).await
}

/// Serves `tools` at `http://127.0.0.1:<port>/mcp` and returns the port.
async fn serve_mcp(tools: Vec<Tool>) -> Result<(u16, Arc<McpProbe>), std::io::Error> {
    let probe = Arc::new(McpProbe::default());
    let tools = Arc::new(tools);
    let service = StreamableHttpService::new(
        move || {
            Ok(BrowserTools {
                tools: tools.clone(),
            })
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let router = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(probe.clone(), record));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok((port, probe))
}

fn browser_tools() -> Vec<Tool> {
    vec![tool("tabs", true), tool("act", false)]
}

/// A port nothing listens on, so the required MCP connection is refused.
async fn dead_port() -> Result<u16, std::io::Error> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    Ok(listener.local_addr()?.port())
}

// ───── recording model ─────

#[derive(Default)]
struct RecordingModel {
    requests: Mutex<Vec<ModelRequest>>,
}

impl RecordingModel {
    fn requests(&self) -> Vec<ModelRequest> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default()
    }

    fn last(&self) -> Result<ModelRequest, String> {
        self.requests()
            .pop()
            .ok_or_else(|| "the model was never called".to_owned())
    }
}

#[async_trait::async_trait]
impl LanguageModel for RecordingModel {
    fn provider(&self) -> ProviderKind {
        ProviderKind::OpenAiCompatible
    }

    fn model_id(&self) -> &str {
        "recording"
    }

    fn supports_media_in_tool_results(&self) -> bool {
        true
    }

    async fn stream(
        &self,
        request: ModelRequest,
        _cancel: CancellationToken,
    ) -> Result<ModelStream, ModelError> {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(request);
        }
        let id = "t".to_owned();
        Ok(Box::pin(futures_util::stream::iter([
            Ok(ModelEvent::TextStart {
                id: id.clone(),
                provider_metadata: None,
            }),
            Ok(ModelEvent::TextDelta {
                id: id.clone(),
                delta: "done".to_owned(),
            }),
            Ok(ModelEvent::TextEnd {
                id,
                provider_metadata: None,
            }),
            Ok(ModelEvent::Finish {
                reason: ModelFinishReason::Stop,
                usage: Usage::default(),
            }),
        ])))
    }
}

// ───── lease and page fakes ─────

#[derive(Debug, Clone)]
struct Minted {
    token: String,
    read_only: bool,
    client_slug: &'static str,
    task_label: String,
    auto_group: bool,
    browser_context: Option<BrowserContext>,
}

#[derive(Default)]
struct LeaseLog {
    minted: Vec<Minted>,
    revoked: Vec<String>,
    context_updates: Vec<(String, Option<BrowserContext>)>,
}

#[derive(Default)]
struct FakeLeases {
    log: Arc<Mutex<LeaseLog>>,
    fail: bool,
}

impl FakeLeases {
    fn minted(&self) -> Vec<Minted> {
        self.log
            .lock()
            .map(|log| log.minted.clone())
            .unwrap_or_default()
    }

    fn revoked(&self) -> Vec<String> {
        self.log
            .lock()
            .map(|log| log.revoked.clone())
            .unwrap_or_default()
    }

    fn context_updates(&self) -> Vec<(String, Option<BrowserContext>)> {
        self.log
            .lock()
            .map(|log| log.context_updates.clone())
            .unwrap_or_default()
    }
}

struct FakeLease {
    token: String,
    log: Arc<Mutex<LeaseLog>>,
}

struct NoGrants;

impl OutputGrants for NoGrants {
    fn allows(&self, _path: &Path) -> bool {
        false
    }
}

#[async_trait::async_trait]
impl LeaseIssuer for FakeLeases {
    async fn mint(&self, spec: LeaseSpec) -> Result<Box<dyn SessionLease>, LeaseError> {
        if self.fail {
            return Err(LeaseError("lease registry unavailable".to_owned()));
        }
        let mut log = self
            .log
            .lock()
            .map_err(|_| LeaseError("lease log poisoned".to_owned()))?;
        let token = format!("lease-{}", log.minted.len() + 1);
        log.minted.push(Minted {
            token: token.clone(),
            read_only: spec.read_only,
            client_slug: spec.client_slug,
            task_label: spec.task_label,
            auto_group: spec.auto_group,
            browser_context: spec.browser_context,
        });
        Ok(Box::new(FakeLease {
            token,
            log: self.log.clone(),
        }))
    }
}

impl SessionLease for FakeLease {
    fn token(&self) -> &str {
        &self.token
    }

    fn update_browser_context(&self, ctx: Option<&BrowserContext>) {
        if let Ok(mut log) = self.log.lock() {
            log.context_updates.push((self.token.clone(), ctx.cloned()));
        }
    }

    fn output_grants(&self) -> Arc<dyn OutputGrants> {
        Arc::new(NoGrants)
    }

    fn revoke(&self) {
        if let Ok(mut log) = self.log.lock() {
            log.revoked.push(self.token.clone());
        }
    }
}

const SCHEDULED_PAGE: u32 = 77;
const SCHEDULED_WINDOW: i64 = 5;

#[derive(Default)]
struct FakePages {
    opened: Mutex<Vec<String>>,
    closed: Mutex<Vec<u32>>,
}

impl FakePages {
    fn opened(&self) -> Vec<String> {
        self.opened
            .lock()
            .map(|opened| opened.clone())
            .unwrap_or_default()
    }

    fn closed(&self) -> Vec<u32> {
        self.closed
            .lock()
            .map(|closed| closed.clone())
            .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl BackgroundPages for FakePages {
    async fn open_background(&self, url: &str) -> anyhow::Result<u32> {
        if let Ok(mut opened) = self.opened.lock() {
            opened.push(url.to_owned());
        }
        Ok(SCHEDULED_PAGE)
    }

    async fn window_of(&self, page_id: u32) -> anyhow::Result<Option<i64>> {
        Ok((page_id == SCHEDULED_PAGE).then_some(SCHEDULED_WINDOW))
    }

    async fn close(&self, page_id: u32) -> anyhow::Result<()> {
        if let Ok(mut closed) = self.closed.lock() {
            closed.push(page_id);
        }
        Ok(())
    }
}

// ───── harness ─────

struct Harness {
    engine: BrowserOsEngine,
    model: Arc<RecordingModel>,
    leases: Arc<FakeLeases>,
    pages: Arc<FakePages>,
    klavis: Arc<Mutex<Option<String>>>,
}

struct Options {
    mcp_port: u16,
    leases: FakeLeases,
    model_error: Option<String>,
}

impl Options {
    fn port(mcp_port: u16) -> Self {
        Self {
            mcp_port,
            leases: FakeLeases::default(),
            model_error: None,
        }
    }
}

fn harness(options: Options) -> Harness {
    let model = Arc::new(RecordingModel::default());
    let leases = Arc::new(options.leases);
    let pages = Arc::new(FakePages::default());
    let klavis = Arc::new(Mutex::new(None::<String>));
    let factory_model = model.clone();
    let model_error = options.model_error;
    let klavis_state = klavis.clone();
    let engine = BrowserOsEngine::new(BrowserOsEngineDeps {
        models: Arc::new(move |_config: &LlmConfig| match &model_error {
            Some(message) => Err(LlmConfigError(message.clone())),
            None => Ok(factory_model.clone() as Arc<dyn LanguageModel>),
        }),
        leases: leases.clone(),
        pages: pages.clone(),
        mcp_port: options.mcp_port,
        klavis_status: Arc::new(move || klavis_state.lock().ok().and_then(|state| state.clone())),
        connector_names: vec!["Gmail".to_owned(), "Slack".to_owned()],
    });
    Harness {
        engine,
        model,
        leases,
        pages,
        klavis,
    }
}

impl Harness {
    fn set_klavis(&self, state: &str) {
        if let Ok(mut klavis) = self.klavis.lock() {
            *klavis = Some(state.to_owned());
        }
    }

    async fn start(
        &self,
        conversation_id: Uuid,
        request: ChatTurnRequest,
        history: Vec<UiMessage>,
    ) -> Result<Vec<UiChunk>, ChatRejection> {
        let view = view_of(request.browser_context.as_ref());
        let stream = self
            .engine
            .start_turn(Turn {
                conversation_id,
                run: RunHandle::new(Uuid::new_v4(), CancellationToken::new()),
                target: TurnTarget::BrowserOs(Box::new(resolved_llm())),
                request: Arc::new(request),
                history,
                browser_context: view,
            })
            .await?;
        Ok(futures_util::StreamExt::collect(stream).await)
    }
}

fn resolved_llm() -> ResolvedLlm {
    ResolvedLlm {
        config: LlmConfig {
            provider: ProviderKind::OpenAiCompatible,
            model: "recording".to_owned(),
            api_key: None,
            base_url: None,
            headers: None,
            resource_name: None,
            region: None,
            access_key_id: None,
            secret_access_key: None,
            session_token: None,
            account_id: None,
            upstream_provider: None,
            conversation_id: String::new(),
        },
        info: ModelInfo {
            provider: ProviderKind::OpenAiCompatible,
            model_id: "recording".to_owned(),
            context_window: None,
            supports_images: true,
        },
        reasoning: ReasoningSettings {
            supports_reasoning: false,
            effort: None,
            summary: ReasoningSummary::default(),
        },
    }
}

/// What `ChatService` hands the engine: tab ids resolved to page ids (here `page = tab + 100`).
fn view_of(ctx: Option<&BrowserContext>) -> Option<BrowserContextView> {
    let tab_view = |tab: &BrowserTab| TabView {
        id: tab.id as i64,
        page_id: Some(tab.id as u32 + 100),
        url: tab.url.clone(),
        title: tab.title.clone(),
    };
    ctx.map(|ctx| BrowserContextView {
        window_id: ctx.window_id.map(|id| id as i64),
        active_tab: ctx.active_tab.as_ref().map(tab_view),
        selected_tabs: ctx.selected_tabs.iter().flatten().map(tab_view).collect(),
        enabled_mcp_servers: ctx.enabled_mcp_servers.clone().unwrap_or_default(),
        custom_mcp_urls: ctx
            .custom_mcp_servers
            .iter()
            .flatten()
            .map(|server| server.url.clone())
            .collect(),
    })
}

fn user_tab_context() -> BrowserContext {
    let mut tab = BrowserTab::new(10.0);
    tab.title = Some("Inbox".to_owned());
    tab.url = Some("https://mail.example.com/".to_owned());
    let mut ctx = BrowserContext::new();
    ctx.window_id = Some(1.0);
    ctx.active_tab = Some(tab);
    ctx
}

fn agent_request(working_dir: Option<PathBuf>) -> ChatTurnRequest {
    ChatTurnRequest {
        browser_context: Some(user_tab_context()),
        user_working_dir: working_dir,
        supports_images: true,
        ..ChatTurnRequest::default()
    }
}

fn chat_request(working_dir: Option<PathBuf>) -> ChatTurnRequest {
    ChatTurnRequest {
        mode: ChatMode::Chat,
        ..agent_request(working_dir)
    }
}

fn scheduled_request() -> ChatTurnRequest {
    ChatTurnRequest {
        is_scheduled_task: true,
        ..agent_request(None)
    }
}

fn message(value: Value) -> Result<UiMessage, serde_json::Error> {
    serde_json::from_value(value)
}

fn user(id: &str, text: &str) -> Result<UiMessage, serde_json::Error> {
    message(json!({ "id": id, "role": "user", "parts": [{ "type": "text", "text": text }] }))
}

/// An assistant turn that used a static filesystem tool and a dynamic MCP tool.
fn assistant_with_tools() -> Result<UiMessage, serde_json::Error> {
    message(json!({
        "id": "a1",
        "role": "assistant",
        "parts": [
            { "type": "step-start" },
            {
                "type": "tool-filesystem_write",
                "toolCallId": "call-fs",
                "state": "output-available",
                "input": { "path": "notes.md", "content": "x" },
                "output": { "content": [{ "type": "text", "text": "written" }] }
            },
            {
                "type": "dynamic-tool",
                "toolName": "act",
                "toolCallId": "call-act",
                "state": "output-available",
                "input": { "page": 110 },
                "output": { "content": [{ "type": "text", "text": "clicked" }], "isError": false }
            },
            { "type": "text", "text": "Saved and clicked." }
        ]
    }))
}

fn tool_names(request: &ModelRequest) -> Vec<String> {
    let mut names = request
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// Tool-call names in the model transcript, in order.
fn transcript_tool_calls(request: &ModelRequest) -> Result<Vec<String>, serde_json::Error> {
    let messages = serde_json::to_value(&request.messages)?;
    Ok(messages
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|message| message["content"].as_array().cloned().unwrap_or_default())
        .filter(|part| part["type"] == "tool-call")
        .filter_map(|part| part["toolName"].as_str().map(str::to_owned))
        .collect())
}

/// The model-facing text of the transcript's last user message.
fn last_user_prompt(request: &ModelRequest) -> Result<String, serde_json::Error> {
    let messages = serde_json::to_value(&request.messages)?;
    Ok(messages
        .as_array()
        .into_iter()
        .flatten()
        .rfind(|message| message["role"] == "user")
        .and_then(|message| message["content"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|part| part["text"].as_str().map(str::to_owned))
        .collect())
}

const USER_TAB_PROMPT: &str = "## Browser Context\n**Window ID:** 1\n**Active Tab:** Tab 10 (Page ID: 110) - \"Inbox\" (https://mail.example.com/)\n\n---\n\n";

/// Lets fire-and-forget page closes run.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}

// ───── failure paths (engine-only) ─────

#[tokio::test]
async fn model_config_failure_rejects_before_any_side_effect() -> TestResult {
    let h = harness(Options {
        model_error: Some("Anthropic provider requires apiKey".to_owned()),
        ..Options::port(dead_port().await?)
    });
    let result = h
        .start(Uuid::new_v4(), scheduled_request(), vec![user("u1", "go")?])
        .await;
    assert!(matches!(result, Err(ChatRejection::Internal(_))));
    assert!(h.leases.minted().is_empty());
    settle().await;
    // The scheduled page opened before the build failed and is closed again.
    assert_eq!(h.pages.opened(), vec!["about:blank".to_owned()]);
    assert_eq!(h.pages.closed(), vec![SCHEDULED_PAGE]);
    assert_eq!(h.engine.session_count(), 0);
    Ok(())
}

#[tokio::test]
async fn lease_failure_rejects_and_releases_the_scheduled_page() -> TestResult {
    let h = harness(Options {
        leases: FakeLeases {
            fail: true,
            ..FakeLeases::default()
        },
        ..Options::port(dead_port().await?)
    });
    let result = h
        .start(Uuid::new_v4(), scheduled_request(), vec![user("u1", "go")?])
        .await;
    assert!(matches!(result, Err(ChatRejection::Internal(_))));
    settle().await;
    assert_eq!(h.pages.closed(), vec![SCHEDULED_PAGE]);
    assert_eq!(h.engine.session_count(), 0);
    Ok(())
}

#[tokio::test]
async fn unreachable_internal_mcp_rejects_and_revokes_the_lease() -> TestResult {
    let h = harness(Options::port(dead_port().await?));
    let conversation = Uuid::new_v4();
    let result = h
        .start(conversation, agent_request(None), vec![user("u1", "hi")?])
        .await;
    assert!(matches!(result, Err(ChatRejection::Internal(_))));
    let minted = h.leases.minted();
    assert_eq!(minted.len(), 1);
    assert_eq!(h.leases.revoked(), vec![minted[0].token.clone()]);
    assert_eq!(h.engine.session_count(), 0);
    assert!(h.model.requests().is_empty());
    assert!(!h.engine.forget(conversation).await);
    Ok(())
}

#[tokio::test]
async fn forget_without_a_session_holds_nothing() {
    let h = harness(Options::port(1));
    assert!(!h.engine.forget(Uuid::new_v4()).await);
}

// ───── through the real agent ─────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_turn_builds_a_session_over_loopback_mcp() -> TestResult {
    let (port, probe) = serve_mcp(browser_tools()).await?;
    let h = harness(Options::port(port));
    let conversation = Uuid::new_v4();
    let chunks = h
        .start(
            conversation,
            agent_request(None),
            vec![user("u1", "Find the BEST flights to NYC")?],
        )
        .await?;

    assert!(matches!(
        chunks.first(),
        Some(UiChunk::Start { message_id: Some(id), .. }) if !id.is_empty()
    ));
    assert!(matches!(chunks.last(), Some(UiChunk::Finish { .. })));

    let minted = h.leases.minted();
    assert_eq!(minted.len(), 1);
    let lease = &minted[0];
    assert!(!lease.read_only);
    assert!(lease.auto_group);
    assert_eq!(lease.client_slug, "browseros");
    assert_eq!(lease.task_label, "find-the-best-flights-to");
    // The lease sees the page ids ChatService resolved.
    let active = lease
        .browser_context
        .as_ref()
        .and_then(|ctx| ctx.active_tab.as_ref());
    assert_eq!(active.and_then(|tab| tab.page_id), Some(110.0));

    let hits = probe.hits();
    assert!(!hits.is_empty());
    for hit in &hits {
        assert_eq!(hit.lease.as_deref(), Some(lease.token.as_str()));
        assert_eq!(hit.query, "structured=1");
    }

    let request = h.model.last()?;
    assert_eq!(
        tool_names(&request),
        [
            "act",
            "filesystem_read",
            "suggest_app_connection",
            "suggest_schedule",
            "tabs"
        ]
    );
    assert_eq!(
        last_user_prompt(&request)?,
        format!("{USER_TAB_PROMPT}<USER_QUERY>\nFind the BEST flights to NYC\n</USER_QUERY>")
    );
    let system = request.system.unwrap_or_default();
    assert!(system.starts_with("<AGENT_PROMPT>"));
    assert_eq!(h.engine.session_count(), 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unchanged_inputs_reuse_the_session() -> TestResult {
    let (port, _probe) = serve_mcp(browser_tools()).await?;
    let h = harness(Options::port(port));
    let conversation = Uuid::new_v4();
    h.start(conversation, agent_request(None), vec![user("u1", "one")?])
        .await?;
    h.start(
        conversation,
        agent_request(None),
        vec![user("u1", "one")?, user("u2", "two")?],
    )
    .await?;

    assert_eq!(h.leases.minted().len(), 1);
    assert!(h.leases.revoked().is_empty());
    // The lease's context is refreshed every turn, because windows change under a living client.
    assert_eq!(h.leases.context_updates().len(), 2);
    assert_eq!(h.model.requests().len(), 2);
    assert_eq!(
        last_user_prompt(&h.model.last()?)?,
        format!("{USER_TAB_PROMPT}<USER_QUERY>\ntwo\n</USER_QUERY>")
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mode_switch_rebuilds_read_only_and_sanitizes_only_the_prompt_copy() -> TestResult {
    let (port, probe) = serve_mcp(browser_tools()).await?;
    let workspace = tempfile::tempdir()?;
    let h = harness(Options::port(port));
    let conversation = Uuid::new_v4();
    let dir = Some(workspace.path().to_path_buf());
    h.start(
        conversation,
        agent_request(dir.clone()),
        vec![user("u1", "save notes")?],
    )
    .await?;
    assert!(tool_names(&h.model.last()?).contains(&"filesystem_write".to_owned()));

    let history = vec![
        user("u1", "save notes")?,
        assistant_with_tools()?,
        user("u2", "what does the page say?")?,
    ];
    let display_before = history.clone();
    h.start(conversation, chat_request(dir), history).await?;

    let minted = h.leases.minted();
    assert_eq!(minted.len(), 2);
    assert_eq!(h.leases.revoked(), vec![minted[0].token.clone()]);
    assert!(minted[1].read_only);
    let read_only_hits = probe
        .hits()
        .into_iter()
        .filter(|hit| hit.lease.as_deref() == Some(minted[1].token.as_str()))
        .collect::<Vec<_>>();
    assert!(!read_only_hits.is_empty());
    assert!(
        read_only_hits
            .iter()
            .all(|hit| hit.query == "structured=1&read_only=1")
    );

    let request = h.model.last()?;
    let tools = tool_names(&request);
    assert!(!tools.iter().any(|name| name.starts_with("suggest_")));
    assert!(!tools.contains(&"filesystem_write".to_owned()));
    assert!(tools.contains(&"filesystem_read".to_owned()));
    // The obsolete static tool is gone from the model's copy. The dynamic MCP tool part survives,
    // because TS `sanitizeMessagesForToolset` only filters `tool-<name>` parts.
    assert_eq!(transcript_tool_calls(&request)?, ["act"]);
    assert_eq!(
        last_user_prompt(&request)?,
        format!(
            "[Context: The user switched to read-only chat mode during this conversation. You can observe pages but can no longer interact with them, and workspace filesystem tools other than reading BrowserOS output files are no longer available.]\n\n{USER_TAB_PROMPT}<USER_QUERY>\nwhat does the page say?\n</USER_QUERY>"
        )
    );
    // The engine owns no history: what the caller holds still has every tool card.
    assert_eq!(display_before[1].parts.len(), 4);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workspace_and_mcp_changes_get_one_notice_each_from_one_rebuild() -> TestResult {
    let (port, _probe) = serve_mcp(browser_tools()).await?;
    let workspace = tempfile::tempdir()?;
    let h = harness(Options::port(port));
    h.set_klavis("ready");
    let conversation = Uuid::new_v4();
    h.start(conversation, agent_request(None), vec![user("u1", "one")?])
        .await?;

    let mut request = agent_request(Some(workspace.path().to_path_buf()));
    if let Some(ctx) = request.browser_context.as_mut() {
        ctx.enabled_mcp_servers = Some(vec!["Slack".to_owned()]);
    }
    h.start(
        conversation,
        request,
        vec![user("u1", "one")?, user("u2", "two")?],
    )
    .await?;

    assert_eq!(h.leases.minted().len(), 2);
    let model_request = h.model.last()?;
    assert!(tool_names(&model_request).contains(&"filesystem_write".to_owned()));
    assert_eq!(
        last_user_prompt(&model_request)?,
        format!(
            "[Context: The following app integrations were connected: Slack. Their tools are now available.]\n[Context: The user connected a workspace during this conversation. Filesystem tools are now available. Working directory: {}]\n\n{USER_TAB_PROMPT}<USER_QUERY>\ntwo\n</USER_QUERY>",
            workspace.path().display()
        )
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn klavis_becoming_ready_rebuilds_with_its_own_notice() -> TestResult {
    let (port, _probe) = serve_mcp(browser_tools()).await?;
    let h = harness(Options::port(port));
    h.set_klavis("connecting");
    let conversation = Uuid::new_v4();
    let mut request = agent_request(None);
    if let Some(ctx) = request.browser_context.as_mut() {
        ctx.enabled_mcp_servers = Some(vec!["Slack".to_owned()]);
    }
    h.start(conversation, request.clone(), vec![user("u1", "one")?])
        .await?;
    h.set_klavis("ready");
    h.start(
        conversation,
        request,
        vec![user("u1", "one")?, user("u2", "two")?],
    )
    .await?;
    assert_eq!(h.leases.minted().len(), 2);
    assert!(last_user_prompt(&h.model.last()?)?.starts_with(
        "[Context: Klavis app integration tools are now available for the following connected apps: Slack.]\n\n"
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduled_turn_runs_on_its_own_background_page_and_closes_it() -> TestResult {
    let (port, _probe) = serve_mcp(browser_tools()).await?;
    let h = harness(Options::port(port));
    let chunks = h
        .start(
            Uuid::new_v4(),
            scheduled_request(),
            vec![user("u1", "Check the news")?],
        )
        .await?;
    assert!(matches!(chunks.last(), Some(UiChunk::Finish { .. })));

    assert_eq!(h.pages.opened(), vec!["about:blank".to_owned()]);
    let minted = h.leases.minted();
    assert_eq!(minted.len(), 1);
    assert!(!minted[0].auto_group);
    let lease_ctx = minted[0].browser_context.clone().unwrap_or_default();
    assert_eq!(lease_ctx.window_id, Some(SCHEDULED_WINDOW as f64));
    let active = lease_ctx.active_tab.unwrap_or_else(|| BrowserTab::new(0.0));
    assert_eq!(active.page_id, Some(f64::from(SCHEDULED_PAGE)));
    assert_eq!(active.title.as_deref(), Some("Scheduled Task"));

    let request = h.model.last()?;
    assert!(
        !tool_names(&request)
            .iter()
            .any(|name| name.starts_with("suggest_"))
    );
    assert_eq!(
        last_user_prompt(&request)?,
        "## Browser Context\n**Window ID:** 5\n**Active Tab:** Tab 77 (Page ID: 77) - \"Scheduled Task\" (about:blank)\n\n---\n\n<USER_QUERY>\nCheck the news\n</USER_QUERY>"
    );
    let system = request.system.unwrap_or_default();
    assert!(system.contains("You are running as a scheduled background task"));
    assert!(system.contains("Use starting page ID `77` directly"));

    settle().await;
    assert_eq!(h.pages.closed(), vec![SCHEDULED_PAGE]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forget_revokes_disposes_and_closes_the_page_once() -> TestResult {
    let (port, _probe) = serve_mcp(browser_tools()).await?;
    let h = harness(Options::port(port));
    let conversation = Uuid::new_v4();
    // Keep the turn's stream alive, so the page is still held when the conversation is deleted.
    let stream = h
        .engine
        .start_turn(Turn {
            conversation_id: conversation,
            run: RunHandle::new(Uuid::new_v4(), CancellationToken::new()),
            target: TurnTarget::BrowserOs(Box::new(resolved_llm())),
            request: Arc::new(scheduled_request()),
            history: vec![user("u1", "go")?],
            browser_context: None,
        })
        .await?;
    assert_eq!(h.engine.session_count(), 1);

    assert!(h.engine.forget(conversation).await);
    settle().await;
    assert_eq!(h.pages.closed(), vec![SCHEDULED_PAGE]);
    let minted = h.leases.minted();
    assert_eq!(h.leases.revoked(), vec![minted[0].token.clone()]);
    assert_eq!(h.engine.session_count(), 0);

    drop(stream);
    settle().await;
    assert_eq!(h.pages.closed(), vec![SCHEDULED_PAGE]);
    assert!(!h.engine.forget(conversation).await);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn custom_mcp_servers_join_after_the_internal_tools() -> TestResult {
    let (port, _probe) = serve_mcp(browser_tools()).await?;
    let (custom_port, custom_probe) =
        serve_mcp(vec![tool("lookup", true), tool("tabs", true)]).await?;
    let h = harness(Options::port(port));
    let mut request = agent_request(None);
    if let Some(ctx) = request.browser_context.as_mut() {
        ctx.custom_mcp_servers = Some(vec![server_api::CustomMcpServer::new(
            "docs".to_owned(),
            format!("http://127.0.0.1:{custom_port}/mcp"),
        )]);
    }
    h.start(Uuid::new_v4(), request, vec![user("u1", "look it up")?])
        .await?;
    let names = tool_names(&h.model.last()?);
    assert!(names.contains(&"lookup".to_owned()));
    // The custom `tabs` collides with the required internal tool and is dropped.
    assert_eq!(names.iter().filter(|name| *name == "tabs").count(), 1);
    // Custom servers never receive the BrowserOS lease.
    assert!(custom_probe.hits().iter().all(|hit| hit.lease.is_none()));
    Ok(())
}
