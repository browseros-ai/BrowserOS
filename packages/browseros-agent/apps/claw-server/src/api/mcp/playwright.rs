//! Prototype MCP adapter for the new runtime. It owns no Claw sessions or effects: the
//! runtime owns browser calls and cleanup, while page ids alone provide continuity. One
//! shared runtime preserves its per-tab input ordering across MCP transport connections.

use std::{borrow::Cow, sync::Arc, time::Duration};

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use neo_runtime::{PageId, RunOutcome, RunPage, RunRequest, RunStatus, Runtime};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    model::{
        CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, Implementation,
        InitializeResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
        ServerCapabilities, SubscriptionFilter, Tool, ToolAnnotations,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        session::local::LocalSessionManager,
        tower::{StreamableHttpServerConfig, StreamableHttpService},
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    sync::{Mutex, OnceCell},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::output::Store;

const DEFAULT_TIMEOUT: u64 = 30_000;
const MAX_TIMEOUT: u64 = 120_000;
const INSTRUCTIONS: &str = concat!(
    "Write Playwright JavaScript. `page` and `context` are provided; await browser calls and return a value. Send an async function body without imports or a wrapper. Omit `page` to open a background tab; pass a returned `pages[].id` to continue in that tab. `timeout` is milliseconds (default 30000, maximum 120000).",
    " Scope large reads with locator.ariaSnapshot({mode:'ai'}); responses use a 24000-byte JSON budget and truncated fields include next arguments for run({result,field,offset}) to read the stored original without changing refs."
);

struct Browser {
    cdp: CdpClient,
    runtime: Runtime,
}

/// A cheap transport-local clone of the process's browser/runtime and shutdown owner.
#[derive(Clone)]
pub struct PlaywrightService {
    cdp_port: u16,
    browser: Arc<OnceCell<Arc<Browser>>>,
    shutdown: CancellationToken,
    runs: TaskTracker,
    output: Arc<Mutex<Store>>,
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
    offset: Option<usize>,
}

impl PlaywrightService {
    #[must_use]
    pub fn new(cdp_port: u16, shutdown: CancellationToken) -> Self {
        Self {
            cdp_port,
            browser: Arc::new(OnceCell::new()),
            shutdown,
            runs: TaskTracker::new(),
            output: Arc::new(Mutex::new(Store::default())),
        }
    }

    async fn browser(&self) -> Result<Arc<Browser>, String> {
        if let Some(browser) = self.browser.get() {
            return Ok(browser.clone());
        }
        let browsers = self.browser.clone();
        let port = self.cdp_port;
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
                            Ok(runtime) => Ok(Arc::new(Browser { cdp, runtime })),
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

    /// Readiness tests the runtime's BrowserOS capability too, instead of claiming ready
    /// merely because something is listening on the CDP port.
    pub async fn ready(&self) -> bool {
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

    pub async fn disconnect(&self) {
        self.shutdown.cancel();
        self.runs.close();
        self.runs.wait().await;
        if let Some(browser) = self.browser.get() {
            browser.runtime.drain().await;
            browser.cdp.disconnect().await;
        }
    }

    #[must_use]
    pub fn http_service(&self) -> StreamableHttpService<Self, LocalSessionManager> {
        let service = self.clone();
        let mut config = StreamableHttpServerConfig::default();
        config.cancellation_token = self.shutdown.child_token();
        StreamableHttpService::new(
            move || Ok(service.clone()),
            Arc::new(LocalSessionManager::default()),
            config,
        )
    }

    async fn execute(&self, args: Arguments, cancel: CancellationToken) -> Value {
        let timeout_ms = args.timeout.unwrap_or(DEFAULT_TIMEOUT);
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let browser = tokio::select! {
            () = cancel.cancelled() => return stopped("cancelled", timeout_ms),
            result = tokio::time::timeout_at(deadline, self.browser()) => match result {
                Ok(Ok(browser)) => browser,
                Ok(Err(error)) => return failure("Error", &error),
                Err(_) => return stopped("timeout", timeout_ms),
            },
        };
        let created = args.page.is_none();
        let page = match args.page {
            Some(page) => page,
            None => match create_page(
                &self.runs,
                browser.cdp.clone(),
                deadline,
                timeout_ms,
                cancel.clone(),
            )
            .await
            {
                Ok(page) => page,
                Err(result) => return result,
            },
        };
        let mut outcome = browser
            .runtime
            .run(RunRequest {
                code: args.code,
                page: PageId::new(page.clone()),
                timeout: deadline.saturating_duration_since(Instant::now()),
                cancel,
            })
            .await;
        if created {
            if let Some(entry) = outcome.pages.iter_mut().find(|entry| entry.id == page) {
                entry.created = true;
            } else {
                outcome.pages.insert(
                    0,
                    RunPage {
                        id: page,
                        created: true,
                        closed: false,
                    },
                );
            }
        }
        // The runtime receives the remaining budget after connect/create. Keep the public
        // timeout message in terms of the whole request's budget.
        if outcome.status == RunStatus::Timeout
            && let Some(error) = &mut outcome.error
        {
            error.message = format!("Timeout {timeout_ms}ms exceeded.");
        }
        report(outcome)
    }
}

/// The current runtime requires a starting page. Bootstrap it here without replaying an
/// uncertain creation. If cancellation wins before the acknowledgement, the owned task
/// waits for that acknowledgement and closes the late tab instead of leaking it.
async fn create_page(
    tasks: &TaskTracker,
    cdp: CdpClient,
    deadline: Instant,
    timeout_ms: u64,
    cancel: CancellationToken,
) -> Result<String, Value> {
    let create_cdp = cdp.clone();
    let mut creation = tasks.spawn(async move {
        create_cdp
            .send(
                "Browser.createTab",
                json!({ "url": "about:blank", "background": true }),
                None,
            )
            .await
    });
    let result = tokio::select! {
        result = &mut creation => result,
        () = cancel.cancelled() => {
            close_late_page(tasks, cdp, creation);
            return Err(stopped("cancelled", 0));
        }
        () = tokio::time::sleep_until(deadline) => {
            close_late_page(tasks, cdp, creation);
            return Err(stopped("timeout", timeout_ms));
        }
    };
    match result {
        Ok(Ok(reply)) => page_id(&reply)
            .map(str::to_string)
            .ok_or_else(|| failure("Error", "Browser.createTab returned no target id")),
        Ok(Err(error)) => Err(failure("Error", &error.to_string())),
        Err(error) => Err(failure("Error", &error.to_string())),
    }
}

fn close_late_page(
    tasks: &TaskTracker,
    cdp: CdpClient,
    creation: tokio::task::JoinHandle<Result<Value, browseros_cdp::CdpError>>,
) {
    tasks.spawn(async move {
        if let Ok(Ok(reply)) = creation.await
            && let Some(id) = page_id(&reply)
        {
            let _ = cdp
                .send("Target.closeTarget", json!({"targetId":id}), None)
                .await;
        }
    });
}

fn page_id(reply: &Value) -> Option<&str> {
    reply.get("tab")?.get("targetId")?.as_str()
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

fn tool() -> Tool {
    Tool::new("run", INSTRUCTIONS, Arc::new(serde_json::from_value(json!({
        "type":"object", "additionalProperties":false,
        "oneOf":[
            {"required":["code"],"not":{"anyOf":[{"required":["result"]},{"required":["field"]},{"required":["offset"]}]}},
            {"required":["result"],"not":{"anyOf":[{"required":["code"]},{"required":["page"]},{"required":["timeout"]}]}}
        ],
        "properties": {
            "code":{"type":"string","description":"Playwright async function body; page and context are provided. Return a value."},
            "page":{"type":"string","minLength":1,"description":"A pages[].id from an earlier result. Omit to open a background tab."},
            "timeout":{"type":"integer","minimum":1,"maximum":MAX_TIMEOUT,"default":DEFAULT_TIMEOUT,"description":"Whole-run timeout in milliseconds."},
            "result":{"type":"string","minLength":1,"description":"Stored truncated result ID. Use instead of code, without page or timeout; retrieval never touches the browser. Process-local LRU: 16 results, 64 MiB; cleared on restart."},
            "field":{"type":"string","enum":["value","logs","calls","error"],"default":"value","description":"Original field to retrieve. Strings are raw text; other values and logs/calls/error use pretty JSON. Follow truncated.next verbatim."},
            "offset":{"type":"integer","minimum":0,"default":0,"description":"UTF-8 byte offset into the original field text. Prefer the next arguments returned with truncation metadata."}
        }
    })).unwrap_or_default()))
    .with_title("Run Playwright code")
    .with_annotations(ToolAnnotations::new().read_only(false).destructive(true).open_world(true))
}

impl ServerHandler for PlaywrightService {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("browseros-neo", crate::VERSION))
            .with_instructions(INSTRUCTIONS)
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
        let mut result = ListToolsResult::with_all_items(vec![tool()]);
        if context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28)
        {
            result = result.with_ttl_ms(0).with_cache_scope(CacheScope::Public);
        }
        Ok(result)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        (name == "run").then(tool)
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(requested.supported_by(&self.get_info().capabilities))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
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
            let result = self
                .output
                .lock()
                .await
                .retrieve(
                    &id,
                    args.field.as_deref().unwrap_or("value"),
                    args.offset.unwrap_or(0),
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
        if args.field.is_some() || args.offset.is_some() {
            return Err(McpError::invalid_params(
                "field and offset require result",
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
        // keeps Runtime::run alive until it has drained calls and detached its sessions.
        let result = self
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
        let result = self.output.lock().await.limit(result);
        Ok(if result["status"] == "ok" {
            CallToolResult::structured(result)
        } else {
            CallToolResult::structured_error(result)
        }
        .into())
    }
}
