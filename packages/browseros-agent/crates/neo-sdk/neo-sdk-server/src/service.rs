//! Neo: The reusable MCP adapter for the runtime. It owns no Claw sessions or effects: the
//! runtime owns browser calls and cleanup, while page ids alone provide continuity. One
//! shared runtime preserves its per-tab input ordering across MCP transport connections.

use std::borrow::Cow;
use std::mem;
use std::path::{self, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime};

use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
#[expect(
    clippy::disallowed_types,
    reason = "the server owns connection lifecycle and passes browser operations to Runtime"
)]
use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use neo_runtime::{
    AbsolutePaths, CaptchaHost, PageId, RunOutcome, RunRequest, RunStatus, Runtime, StartPage,
};
use neo_trace::RunId;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    Implementation, InitializeResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
    ServerCapabilities, SubscriptionFilter, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, SubscriptionContext};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::runtime::Handle;
use tokio::sync::{Mutex, OnceCell, Semaphore};
use tokio::time::{self, Instant};

use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::Config;

use super::budget::{Budget, DEFAULT_TOKENS, LIMIT_MAX, LIMIT_MIN, MAX_TOKENS, MIN_TOKENS};
use super::output::{RESULTS_DIR, Store};
use super::replies::{REPLY_DELIVERY_GRACE, ReplyTracker};
use super::{artifacts, binary};

/// Neo: the default wall budget includes admission, connection, tab creation and the script.
const DEFAULT_TIMEOUT: u64 = 30_000;
/// Neo: the public run ceiling stays inside the CDP request budget, with cleanup margin.
const MAX_TIMEOUT: u64 = 120_000;
/// Neo: transport and output policies supplement the runtime-owned API description.
const INSTRUCTIONS: &str = concat!(
    "Write Playwright JavaScript. `page` and `context` are provided; await browser calls and return a value. Send an async (page, context, playwright) => { ... } function, a lone expression, or an async function body. Imports are unavailable. Omit `page` to open a background tab; pass a returned `pages[].id` to continue in that tab. `timeout` is milliseconds (default 30000, maximum 120000).",
    " Scope large reads with locator.ariaSnapshot({mode:'ai'}). A response's JSON is held to maxResponseTokens (default 100000, 1000-200000; tokens = UTF-8 bytes / 4, rounded up), settable on any call; image blocks are outside it. If your client caps tool output lower, pass maxResponseTokens. Truncated fields include next arguments for run({result,field,path,offset,limit,maxResponseTokens}) to read the stored original without changing refs; next repeats the maxResponseTokens you set. limit (4000-100000 bytes, at most maxResponseTokens x 4) narrows a window, and truncated.<field>.file names a local file holding the field's full text. A large object or array keeps its type: small keys stay whole, and truncated.value.cut lists each shortened part by JSON Pointer path with its own next.",
    " A `path` option (screenshot, pdf, storageState, download.saveAs) writes into one artifact directory shared by all runs: relative paths resolve there, and an absolute path outside it is written there under its file name; the result's artifacts[] gives each file's absolute `file` path (and `requested` when redirected). setInputFiles reads from the same directory.",
    " A returned Buffer (return await page.screenshot()), alone or nested, is saved there in full and described in value as {type, mimeType, bytes, width, height, file}; a PNG or JPEG is also returned as an image content block, scaled to at most 1568 px and 1.15 megapixels."
);
/// Neo: how often, at most, a run that wrote files prunes the artifact directory again.
const PRUNE_INTERVAL: Duration = Duration::from_mins(1);

struct Browser {
    #[expect(
        clippy::disallowed_types,
        reason = "retain the connection only for readiness and disconnect after runtime cleanup"
    )]
    cdp: CdpClient,
    runtime: Runtime,
}

/// Binds a host-selected solver once to the server-owned connection. The reusable server
/// never reads credentials or constructs a provider; runtime cleanup owns all solver input.
struct CaptchaCapability {
    factory: Arc<CaptchaFactory>,
    instructions: &'static str,
}

#[expect(
    clippy::disallowed_types,
    reason = "the host factory only binds its capability to this server-owned connection"
)]
type CaptchaFactory = dyn Fn(CdpClient) -> Arc<dyn CaptchaHost> + Send + Sync;

/// A cheap transport-local clone of the process's browser/runtime and shutdown owner.
#[derive(Clone)]
pub struct NeoSdkServer {
    cdp_port: u16,
    // A spinning realm holds its Tokio worker until it notices cancellation. Leave workers
    // available to receive cancellation and tools/list while other runs are executing.
    permits: Arc<Semaphore>,
    captcha: Option<Arc<CaptchaCapability>>,
    browser: Arc<OnceCell<Arc<Browser>>>,
    shutdown: CancellationToken,
    // Run cancellation must not cut rmcp's pending first message or SSE reply body.
    // Transport waits for counted reply ownership to end (or the consumer grace), while
    // the host's connection drain handles flushing. A body drop is not client receipt.
    transport: CancellationToken,
    replies: ReplyTracker,
    runs: TaskTracker,
    output: Arc<Mutex<Store>>,
    state_dir: PathBuf,
    /// When the artifact directory was last pruned; `None` before the first prune.
    pruned: Arc<StdMutex<Option<Instant>>>,
}

/// A run's report and the image blocks of the binaries it returned (`binary.rs`).
struct Reply {
    report: Value,
    images: Vec<binary::Image>,
    /// Present once a traced runtime run began; admission and connection errors have no trace.
    run_id: Option<RunId>,
}

impl From<Value> for Reply {
    fn from(report: Value) -> Self {
        Self {
            report,
            images: Vec::new(),
            run_id: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    code: String,
    page: Option<String>,
    timeout: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestArguments {
    code: Option<String>,
    page: Option<String>,
    timeout: Option<u64>,
    result: Option<String>,
    field: Option<String>,
    path: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
    /// Raw, so `Budget::parse` words every refusal the same way.
    #[serde(rename = "maxResponseTokens")]
    max_response_tokens: Option<Value>,
}

impl NeoSdkServer {
    #[must_use]
    pub fn new(config: Config, shutdown: CancellationToken) -> Self {
        let service = Self {
            cdp_port: config.cdp_port,
            permits: Arc::new(Semaphore::new(
                Handle::try_current()
                    .map_or(1, |handle| (handle.metrics().num_workers() / 2).max(1)),
            )),
            captcha: None,
            browser: Arc::new(OnceCell::new()),
            shutdown,
            transport: CancellationToken::new(),
            replies: ReplyTracker::default(),
            runs: TaskTracker::new(),
            output: Arc::new(Mutex::new(Store::default())),
            state_dir: config.state_dir,
            pruned: Arc::new(StdMutex::new(None)),
        };
        service.prune_artifacts();
        service
    }

    /// Installs a host-selected capability before sharing this server or connecting it.
    /// The factory runs once with the owned connection, so all requests share one solver
    /// identity, while shutdown still drains its runtime before disconnecting CDP.
    #[must_use]
    #[expect(
        clippy::disallowed_types,
        reason = "the callback receives only the connection this server owns for Runtime::connect"
    )]
    pub fn with_captcha<F>(self, factory: F, instructions: &'static str) -> Self
    where
        F: Fn(CdpClient) -> Arc<dyn CaptchaHost> + Send + Sync + 'static,
    {
        Self {
            captcha: Some(Arc::new(CaptchaCapability {
                factory: Arc::new(factory),
                instructions,
            })),
            ..self
        }
    }

    fn instructions(&self) -> String {
        let mut instructions = instructions();
        if let Some(capability) = &self.captcha {
            instructions.push(' ');
            instructions.push_str(capability.instructions);
        }
        instructions
    }

    fn tool(&self) -> Tool {
        let mut tool = tool();
        tool.description = Some(self.instructions().into());
        tool
    }

    /// Every run's artifact directory, `<state_dir>/artifacts` (`artifacts.rs`).
    fn artifact_dir(&self) -> PathBuf {
        artifacts::dir(&self.state_dir)
    }

    /// Prunes the artifact directory in the background (`artifacts::prune`), at boot and then
    /// at most once per [`PRUNE_INTERVAL`]. Outside a tokio runtime (no task to own the walk)
    /// it waits for the next run that writes a file.
    fn prune_artifacts(&self) {
        let Ok(handle) = Handle::try_current() else {
            return;
        };
        {
            let Ok(mut pruned) = self.pruned.lock() else {
                return;
            };
            let now = Instant::now();
            if pruned.is_some_and(|at| now.duration_since(at) < PRUNE_INTERVAL) {
                return;
            }
            *pruned = Some(now);
        }
        let dir = self.artifact_dir();
        // The walk blocks on the file system; the shutdown drain (`runs`) waits for it.
        self.runs.spawn_blocking_on(
            move || match artifacts::prune(&dir, SystemTime::now(), artifacts::BOUNDS) {
                Ok(pruned) if pruned.files > 0 => tracing::info!(
                    files = pruned.files,
                    bytes = pruned.bytes,
                    dir = %dir.display(),
                    "pruned the run artifact directory"
                ),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, dir = %dir.display(), "artifact prune failed"),
            },
            &handle,
        );
    }

    #[expect(
        clippy::disallowed_types,
        reason = "the connection initializer hands the client to Runtime and owns its final disconnect"
    )]
    async fn browser(&self) -> Result<Arc<Browser>, String> {
        // Take ownership before checking shutdown. If shutdown races the handoff to the
        // initializer task, its drain still sees this token until that task owns the work.
        let _connecting = self.runs.token();
        if self.shutdown.is_cancelled() {
            return Err("Neo server is shutting down".to_string());
        }
        if let Some(browser) = self.browser.get() {
            return Ok(Arc::clone(browser));
        }
        let browsers = Arc::clone(&self.browser);
        let port = self.cdp_port;
        let shutdown = self.shutdown.clone();
        let captcha = self.captcha.clone();
        // Connecting starts CDP reader/supervisor tasks. A request timing out must not
        // drop their initializer halfway through; finish it under the process owner so
        // shutdown can disconnect the resulting link even if the caller has gone.
        self.runs
            .spawn(async move {
                browsers
                    .get_or_try_init(|| async {
                        let cdp = CdpClient::connect(ConnectOptions {
                            host: Some("127.0.0.1".to_string()),
                            connect_timeout: Duration::from_secs(2),
                            connect_max_retries: 1,
                            // Script deadlines own user-facing timeouts. Keep the CDP
                            // reply alive through the longest run and late-write cleanup.
                            request_timeout: Duration::from_millis(MAX_TIMEOUT + 5_000),
                            // A browser outage must not exit the sidecar. This client repairs the
                            // link; the runtime continues to share its input turns after recovery.
                            reconnect_policy: ReconnectPolicy::KeepTrying,
                            ..ConnectOptions::new(port)
                        })
                        .await
                        .map_err(|error| error.to_string())?;
                        match Runtime::connect(cdp.clone()).await {
                            // Agents name paths on their own machine; Neo writes them only
                            // inside the artifact directory (`artifacts.rs`).
                            Ok(runtime) => {
                                if shutdown.is_cancelled() {
                                    cdp.disconnect().await;
                                    return Err("Neo server is shutting down".to_string());
                                }
                                let runtime = match &captcha {
                                    Some(capability) => {
                                        runtime.with_captcha((capability.factory)(cdp.clone()))
                                    }
                                    None => runtime,
                                };
                                Ok(Arc::new(Browser {
                                    cdp,
                                    runtime: runtime.with_absolute_paths(AbsolutePaths::Redirect),
                                }))
                            }
                            Err(error) => {
                                cdp.disconnect().await;
                                Err(error.to_string())
                            }
                        }
                    })
                    .await
                    .cloned()
            })
            .await
            .map_err(|error| error.to_string())?
    }

    /// Readiness tests the runtime's `BrowserOS` capability too, instead of claiming ready
    /// merely because something is listening on the CDP port.
    pub async fn ready(&self) -> bool {
        if self.shutdown.is_cancelled() {
            return false;
        }
        self.browser()
            .await
            .is_ok_and(|browser| browser.cdp.is_connected())
    }

    #[must_use]
    pub fn connected(&self) -> bool {
        self.browser
            .get()
            .is_some_and(|browser| browser.cdp.is_connected())
    }

    /// Refuses admission, cancels runs, awaits owned work without a deadline, then drains
    /// reply ownership before ending transport streams. Safe inside a host's HTTP graceful future.
    ///
    /// Covered runs are in flight when shutdown is cancelled. New POSTs can extend the reply
    /// drain within its consumer grace; a refused POST arriving after that drain resolved has
    /// no in-flight delivery guarantee. The grace never bounds run work or runtime cleanup.
    /// Calls after transport cancellation are no-ops.
    pub async fn close_transport(&self) {
        if self.transport.is_cancelled() {
            return;
        }
        self.permits.close();
        self.shutdown.cancel();
        self.runs.close();
        self.runs.wait().await;
        let _delivered = self.replies.drain(REPLY_DELIVERY_GRACE).await;
        self.transport.cancel();
    }

    /// Ends reply ownership and transport, then awaits runtime cleanup before disconnecting.
    /// Cleanup can outlive a script deadline while CDP delivery settles. Disconnecting sooner
    /// would lose late tab-creation acknowledgements and the cleanup that closes those tabs.
    pub async fn shutdown(&self) {
        self.close_transport().await;
        if let Some(browser) = self.browser.get() {
            browser.runtime.drain().await;
            browser.cdp.disconnect().await;
        }
    }

    /// The complete MCP route and its request hygiene. The host keeps its listener and
    /// system endpoints; every mounted copy shares this server's runtime and retained store.
    pub fn router(&self) -> Router {
        Router::new().nest_service(
            "/mcp",
            Router::new()
                .fallback_service(self.http_service())
                // This inner layer wraps the rmcp fallback itself, including unpolled bodies.
                // Other host routes (including capture) never enter MCP reply accounting.
                .layer(middleware::from_fn_with_state(
                    self.replies.clone(),
                    ReplyTracker::track,
                ))
                .layer(middleware::from_fn(request_hygiene)),
        )
    }

    #[must_use]
    fn http_service(&self) -> StreamableHttpService<Self, LocalSessionManager> {
        let service = self.clone();
        let mut config = StreamableHttpServerConfig::default();
        config.cancellation_token = self.transport.clone();
        StreamableHttpService::new(
            move || Ok(service.clone()),
            Arc::new(LocalSessionManager::default()),
            config,
        )
    }

    async fn execute(&self, args: Arguments, cancel: CancellationToken) -> Reply {
        let timeout_ms = args.timeout.unwrap_or(DEFAULT_TIMEOUT);
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        // Admission uses the same deadline as connection and execution. A queued request
        // never opens a tab or outlives the budget the model supplied.
        let permit = tokio::select! {
            result = Arc::clone(&self.permits).acquire_owned() => match result {
                Ok(permit) => permit,
                Err(_) => return failure("Error", "Neo server is shutting down").into(),
            },
            () = cancel.cancelled() => return stopped("cancelled", timeout_ms).into(),
            () = time::sleep_until(deadline) => return stopped("timeout", timeout_ms).into(),
        };
        let browser = tokio::select! {
            () = cancel.cancelled() => return stopped("cancelled", timeout_ms).into(),
            result = time::timeout_at(deadline, self.browser()) => match result {
                Ok(Ok(browser)) => browser,
                Ok(Err(error)) => return failure("Error", &error).into(),
                Err(_) => return stopped("timeout", timeout_ms).into(),
            },
        };
        // The runtime owns both first-tab creation and context.newPage(), including late
        // acknowledgements after cancellation. The adapter only selects the starting policy.
        let page = args
            .page
            .map_or(StartPage::NewTab, |id| StartPage::Page(PageId::new(id)));
        let (run_id, mut outcome) = neo_trace::run(
            &browser.runtime,
            RunRequest {
                code: args.code,
                page,
                timeout: deadline.saturating_duration_since(Instant::now()),
                cancel,
                artifacts: Some(self.artifact_dir()),
            },
        )
        .await;
        // Presentation and disk IO cannot hold a realm slot after the runtime has ended.
        drop(permit);
        if !outcome.artifacts.is_empty() || !outcome.binaries.is_empty() {
            self.prune_artifacts();
        }
        let binaries = mem::take(&mut outcome.binaries);
        // The runtime receives the remaining budget after connect. Keep the public
        // timeout message in terms of the whole request's budget.
        if outcome.status == RunStatus::Timeout
            && let Some(error) = &mut outcome.error
        {
            error.message = format!("Timeout {timeout_ms}ms exceeded.");
        }
        let mut report = report(outcome);
        let dir = self.artifact_dir();
        artifacts::locate(&mut report, &dir);
        if binaries.is_empty() {
            return Reply {
                report,
                images: Vec::new(),
                run_id: Some(run_id),
            };
        }
        // Writing, decoding and scaling block; the shutdown drain (`runs`) waits for them.
        let mut reply = self
            .runs
            .spawn_blocking(move || {
                let images = binary::present(&mut report, binaries, &dir);
                Reply {
                    report,
                    images,
                    run_id: None,
                }
            })
            .await
            .unwrap_or_else(|error| failure("Error", &error.to_string()).into());
        reply.run_id = Some(run_id);
        reply
    }
}

/// Reject browser-origin requests before they reach the local privileged tool. This policy
/// travels with the reusable route so another host cannot accidentally omit the boundary.
async fn request_hygiene(request: Request, next: Next) -> Response {
    if request.method() == Method::OPTIONS {
        return StatusCode::NO_CONTENT.into_response();
    }
    let headers = request.headers();
    if headers.contains_key(header::ORIGIN) || headers.contains_key("sec-fetch-site") {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"unsupported request"})),
        )
            .into_response();
    }
    let needs_json = match *request.method() {
        Method::POST | Method::PUT | Method::PATCH => true,
        Method::DELETE => headers.contains_key(header::CONTENT_TYPE),
        _ => false,
    };
    if needs_json
        && !headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("application/json"))
    {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(json!({"error":"unsupported content type"})),
        )
            .into_response();
    }
    next.run(request).await
}

fn report(outcome: RunOutcome) -> Value {
    // RunOutcome already owns the complete runtime report; keep the adapter's output in
    // lockstep with it rather than reimplementing its trace and lifecycle schema.
    serde_json::to_value(outcome).unwrap_or_else(|error| failure("Error", &error.to_string()))
}

fn failure(name: &str, message: &str) -> Value {
    json!({"status":"error", "logs":[], "error":{"name":name,"message":message}, "pages":[], "calls":[]})
}

fn stopped(status: &str, timeout: u64) -> Value {
    let mut result = json!({"status":status, "logs":[], "pages":[], "calls":[]});
    if status == "timeout" {
        result["error"] =
            json!({"name":"TimeoutError","message":format!("Timeout {timeout}ms exceeded.")});
    }
    result
}

/// The same realm facts appear in initialize and tools/list so either client entry path
/// teaches the current API. The runtime supplies its own globals and lifecycle differences.
fn instructions() -> String {
    format!("{INSTRUCTIONS}\n\n{}", neo_runtime::api_description())
}

fn tool() -> Tool {
    Tool::new("run", instructions(), Arc::new(Map::from_iter([
        ("type".into(), json!("object")),
        ("additionalProperties".into(), json!(false)),
        ("oneOf".into(), json!([
            {"required":["code"],"not":{"anyOf":[{"required":["result"]},{"required":["field"]},{"required":["path"]},{"required":["offset"]},{"required":["limit"]}]}},
            {"required":["result"],"not":{"anyOf":[{"required":["code"]},{"required":["page"]},{"required":["timeout"]}]}}
        ])),
        ("properties".into(), json!({
            "code":{"type":"string","description":"Playwright code: an async function of page, context and playwright, an async function body, or a lone expression. Return a value."},
            "page":{"type":"string","minLength":1,"description":"A pages[].id from an earlier result. Omit to open a background tab."},
            "timeout":{"type":"integer","minimum":1,"maximum":MAX_TIMEOUT,"default":DEFAULT_TIMEOUT,"description":"Whole-run timeout in milliseconds."},
            "result":{"type":"string","minLength":1,"description":"Stored truncated result ID. Use instead of code, without page or timeout; retrieval never touches the browser. Process-local LRU: 16 results, 64 MiB; cleared on restart."},
            "field":{"type":"string","enum":["value","logs","calls","error"],"default":"value","description":"Original field to retrieve. Strings are raw text; other values and logs/calls/error use pretty JSON. Follow truncated.next verbatim."},
            "path":{"type":"string","description":"JSON Pointer (RFC 6901) to one node inside the field, such as /snap or /items/0; offset then counts into that node's text (raw for a string, pretty JSON otherwise). Copy it from a truncated.value.cut[].next."},
            "offset":{"type":"integer","minimum":0,"default":0,"description":"UTF-8 byte offset into the original field text. Prefer the next arguments returned with truncation metadata."},
            "limit":{"type":"integer","minimum":LIMIT_MIN,"maximum":LIMIT_MAX,"description":"Narrows a retrieval window to this many bytes of compact JSON; at most maxResponseTokens x 4, which is the default window. Later next arguments keep it. For the whole text at once, read truncated.<field>.file."},
            "maxResponseTokens":{"type":"integer","minimum":MIN_TOKENS,"maximum":MAX_TOKENS,"default":DEFAULT_TOKENS,"description":"Token budget of this response's JSON, estimated as UTF-8 bytes / 4 rounded up; larger results are truncated with next arguments. Works with code or result; next arguments repeat it. Lower it if your client caps tool output."}
        })),
    ])))
    .with_title("Run Playwright code")
    .with_annotations(ToolAnnotations::new().read_only(false).destructive(true).open_world(true))
}

impl ServerHandler for NeoSdkServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "browseros-neo",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(self.instructions())
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[
            ProtocolVersion::V_2026_07_28,
            ProtocolVersion::V_2025_11_25,
            ProtocolVersion::V_2025_06_18,
            ProtocolVersion::V_2025_03_26,
            ProtocolVersion::V_2024_11_05,
        ])
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut result = ListToolsResult::with_all_items(vec![self.tool()]);
        if context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28)
        {
            result = result.with_ttl_ms(0).with_cache_scope(CacheScope::Public);
        }
        Ok(result)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        (name == "run").then(|| self.tool())
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(requested.supported_by(&self.get_info().capabilities))
    }

    /// A listen POST is counted like any reply, so it must finish without transport cancel.
    /// Returning success lets rmcp emit the protocol's final subscription result.
    async fn listen(&self, context: SubscriptionContext) -> Result<(), McpError> {
        tokio::select! {
            () = context.cancelled() => {}
            () = self.shutdown.cancelled() => {}
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the existing run and retained-result modes share one protocol validation and budget boundary"
    )]
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        // The token covers parsing, result retrieval and the execution-task handoff. Closing
        // admission alone does not prevent a cloned handler from arriving during shutdown.
        let _exchange = self.runs.token();
        if self.shutdown.is_cancelled() {
            return Ok(CallToolResult::structured_error(failure(
                "Error",
                "Neo server is shutting down",
            ))
            .into());
        }
        if request.name != "run" {
            return Ok(CallToolResult::structured_error(failure(
                "Error",
                &format!("Unknown tool: {}", request.name),
            ))
            .into());
        }
        let arguments = request.arguments.unwrap_or_default();
        if arguments.values().any(Value::is_null) {
            return Err(McpError::invalid_params(
                "provided run arguments must not be null",
                None,
            ));
        }
        let args = serde_json::from_value::<RequestArguments>(Value::Object(arguments))
            .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
        let budget = Budget::parse(args.max_response_tokens.as_ref())
            .map_err(|error| McpError::invalid_params(error, None))?;
        if let Some(id) = args.result {
            if args.code.is_some()
                || args.page.is_some()
                || args.timeout.is_some()
                || id.trim().is_empty()
            {
                return Err(McpError::invalid_params(
                    "use result instead of code, without page or timeout",
                    None,
                ));
            }
            // Refuse a bad window before waiting for the store.
            budget
                .window(args.limit)
                .map_err(|error| McpError::invalid_params(error, None))?;
            let result = self
                .output
                .lock()
                .await
                .retrieve(
                    &id,
                    args.field.as_deref().unwrap_or("value"),
                    args.path.as_deref(),
                    args.offset.unwrap_or(0),
                    budget,
                    args.limit,
                )
                .map_err(|error| McpError::invalid_params(error, None))?;
            return Ok(if result["status"] == "ok" {
                CallToolResult::structured(result)
            } else {
                CallToolResult::structured_error(result)
            }
            .into());
        }
        let Some(code) = args.code else {
            return Err(McpError::invalid_params(
                "exactly one of code or result is required",
                None,
            ));
        };
        if args.field.is_some()
            || args.path.is_some()
            || args.offset.is_some()
            || args.limit.is_some()
        {
            return Err(McpError::invalid_params(
                "field, path, offset and limit require result",
                None,
            ));
        }
        let args = Arguments {
            code,
            page: args.page,
            timeout: args.timeout,
        };
        if args
            .timeout
            .is_some_and(|timeout| timeout == 0 || timeout > MAX_TIMEOUT)
            || args
                .page
                .as_ref()
                .is_some_and(|page| page.trim().is_empty())
        {
            return Err(McpError::invalid_params(
                "timeout must be 1..120000 milliseconds; page must be nonempty",
                None,
            ));
        }
        let service = self.clone();
        let cancel = context.ct.child_token();
        let _cancel_on_drop = cancel.clone().drop_guard();
        // A transport dropping its handler cancels the request, while this owned task
        // keeps the traced runtime run alive through its final call and run.end events.
        let Reply {
            report,
            images,
            run_id,
        } = self
            .runs
            .spawn(async move {
                let execution = service.execute(args, cancel.clone());
                tokio::pin!(execution);
                tokio::select! {
                    result = &mut execution => result,
                    () = service.shutdown.cancelled() => {
                        cancel.cancel();
                        execution.await
                    }
                }
            })
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        let files = self.state_dir.join(RESULTS_DIR);
        // Links in the reply must stay valid if the client resolves them from another cwd.
        let files = path::absolute(&files).unwrap_or(files);
        let (result, disk) = self.output.lock().await.limit(report, Some(&files), budget);
        // An owned task, so a dropped request still deletes evicted files and finishes writes;
        // the reply waits for it to learn which file links survived.
        let result = self
            .runs
            .spawn(disk.apply(result))
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        let mut result = if result["status"] == "ok" {
            CallToolResult::structured(result)
        } else {
            CallToolResult::structured_error(result)
        };
        if let Some(run_id) = run_id {
            let mut meta = Map::new();
            meta.insert("com.browseros.neo/run".to_string(), json!(run_id));
            result.meta = Some(meta.into());
        }
        // After the text block, which stays the JSON of `structuredContent`.
        result.content.extend(
            images
                .into_iter()
                .map(|image| ContentBlock::image(image.data, image.mime_type)),
        );
        Ok(result.into())
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_TOKENS, INSTRUCTIONS, LIMIT_MAX, LIMIT_MIN, MAX_TOKENS, MIN_TOKENS};

    /// The agent-facing skill, as `services/harness_skills.rs` embeds it.
    const SKILL: &str = include_str!("../../../../resources/skills/browserclaw/PLAYWRIGHT.md");

    #[test]
    fn tests_that_agent_docs_state_the_token_budget_and_its_override() {
        for (name, doc) in [("INSTRUCTIONS", INSTRUCTIONS), ("PLAYWRIGHT.md", SKILL)] {
            // Prose may write ranges with an en dash.
            let doc = doc.replace('\u{2013}', "-");
            for needle in [
                "maxResponseTokens".to_string(),
                format!("default {DEFAULT_TOKENS}"),
                format!("{MIN_TOKENS}-{MAX_TOKENS}"),
                format!("{LIMIT_MIN}-{LIMIT_MAX}"),
            ] {
                assert!(doc.contains(&needle), "{name} lacks {needle:?}");
            }
            for stale in ["24000", "24,000", "1000-100000"] {
                assert!(!doc.contains(stale), "{name} still says {stale:?}");
            }
        }
    }
}

#[cfg(test)]
mod shutdown_tests;
