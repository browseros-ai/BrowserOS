//! `/mcp/{task}`: the private MCP endpoint a coding agent reaches the browser through.
//!
//! ```text
//!   coding agent ─POST /mcp/{task}─▶ refuse Origin / Sec-Fetch-Site (403)
//!                                    refuse an unarmed task (404)
//!                                    put the armed scope in the request's extensions
//!                                  ─▶ rmcp stateless streamable HTTP (initialize, ping,
//!                                     tools/list, tools/call, notifications)
//!                                  ─▶ TaskServer::call_tool ─▶ Browser::call(armed scope)
//! ```
//!
//! The path's task is the conversation id, and the scope is the one the turn armed, so a call
//! from the agent carries the same `taskId` and read-only access as the server's own loop. The
//! endpoint has no token of its own: like the engine's own loopback port, it relies on loopback
//! plus the browser-origin refusal, and it answers only while a turn holds the task's `Armed`.

use std::borrow::Cow;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    Implementation, InitializeResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
    ServerCapabilities, ServerInfo, SubscriptionFilter,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, ServerHandler};
use tower::ServiceExt;

use super::armed::ArmedCall;
use super::{Browser, Tool, ToolOutput};

/// The server's name in `initialize`.
const SERVER_NAME: &str = "browseros";

/// Every revision rmcp serves without a session, newest first, as the engine's own endpoint
/// lists them (`neo-server/src/mcp/mcp_server.rs`).
const SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] = &[
    ProtocolVersion::V_2026_07_28,
    ProtocolVersion::V_2025_11_25,
    ProtocolVersion::V_2025_06_18,
    ProtocolVersion::V_2025_03_26,
    ProtocolVersion::V_2024_11_05,
];

type McpService = StreamableHttpService<TaskServer, NeverSessionManager>;

#[derive(Clone)]
struct Endpoint {
    browser: Browser,
    service: McpService,
}

/// Mounted by `http` under `/mcp`, outside the extension's trust rule.
pub(super) fn router(browser: Browser) -> axum::Router {
    // Stateless: no `Mcp-Session-Id`, so a request needs nothing from an earlier one and the
    // armed check below is the whole of the endpoint's state. The keep-alive is short because a
    // gone client is noticed only on the next write, and noticing it is what stops its call.
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_sse_keep_alive(Some(std::time::Duration::from_secs(5)));
    let handler = TaskServer {
        browser: browser.clone(),
    };
    let service = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    axum::Router::new()
        .route("/{task}", axum::routing::any(endpoint))
        .with_state(Endpoint { browser, service })
}

async fn endpoint(
    State(endpoint): State<Endpoint>,
    Path(task): Path<String>,
    mut request: Request,
) -> Response {
    let headers = request.headers();
    // A browser's fetch carries one of these and a native MCP client does not: this is what
    // keeps a web page, including one the agent itself opened, from calling `run`.
    if headers.contains_key(header::ORIGIN) || headers.contains_key("sec-fetch-site") {
        return (
            StatusCode::FORBIDDEN,
            "browser requests are not served here",
        )
            .into_response();
    }
    let Some(armed) = endpoint.browser.armed_call(&task) else {
        return (StatusCode::NOT_FOUND, "no turn is running for this task").into_response();
    };
    // rmcp copies the request's `http::request::Parts` into each handler's context, so the
    // armed scope travels with the request instead of living in a per-task service.
    request.extensions_mut().insert(armed);
    match endpoint.service.oneshot(request).await {
        Ok(response) => response.map(Body::new),
        Err(never) => match never {},
    }
}

/// The MCP handler. Holds no task: each call reads its scope from the request.
#[derive(Clone)]
struct TaskServer {
    browser: Browser,
}

impl TaskServer {
    fn tools(&self) -> Vec<rmcp::model::Tool> {
        self.browser.tools().into_iter().map(to_mcp_tool).collect()
    }
}

fn to_mcp_tool(tool: Tool) -> rmcp::model::Tool {
    let schema = match tool.input_schema {
        serde_json::Value::Object(schema) => schema,
        _ => serde_json::Map::new(),
    };
    rmcp::model::Tool::new(tool.name, tool.description, schema)
}

impl ServerHandler for TaskServer {
    fn get_info(&self) -> ServerInfo {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, crate::VERSION))
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(SUPPORTED_PROTOCOL_VERSIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let result = ListToolsResult::with_all_items(self.tools());
        // 2026-07-28 clients require a cache hint on list results; older ones keep the old shape.
        let modern = context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28);
        Ok(if modern {
            result.with_ttl_ms(0).with_cache_scope(CacheScope::Public)
        } else {
            result
        })
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        self.tools().into_iter().find(|tool| tool.name == name)
    }

    // A 2026-07-28 client may open `subscriptions/listen`; refusing it would end the transport
    // before `tools/list`. Accept what the capabilities offer (tools only), so it stays quiet.
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
    ) -> Result<CallToolResponse, ErrorData> {
        let Some(armed) = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<ArmedCall>())
            .cloned()
        else {
            return Err(ErrorData::internal_error(
                "the call reached the handler without an armed scope",
                None,
            ));
        };
        let args = request.arguments.unwrap_or_default();
        if request.name == "run" {
            let unknown: Vec<_> = args
                .keys()
                .filter(|key| !matches!(key.as_str(), "code" | "timeout"))
                .map(|key| format!("{key:?}"))
                .collect();
            if !unknown.is_empty() {
                // An MCP tool error reaches the model as a tool result it can correct.
                // This text is generated here and carries no output from the page.
                let output = ToolOutput::failure(format!(
                    "Unknown arguments for run: {}. The run tool takes only code and optional timeout (milliseconds). Put values such as tab IDs and URLs inside the JavaScript in code, then retry.",
                    unknown.join(", ")
                ));
                return Ok(to_mcp_result(output).into());
            }
        }
        let args = serde_json::Value::Object(args);
        // A child of the turn's token: dropping `Armed` stops this call, and so does the agent
        // going away (stateless rmcp's only cancel: it drops `notifications/cancelled`).
        let cancel = armed.cancel.child_token();
        let call = self
            .browser
            .call(&armed.scope, &request.name, args, &cancel);
        let output = tokio::select! {
            output = call => output,
            () = context.ct.cancelled() => {
                cancel.cancel();
                ToolOutput::failure("the caller went away")
            }
        };
        Ok(to_mcp_result(output).into())
    }
}

/// The fenced text first, then each picture as an MCP image, in the order of their `[picture N]`
/// placeholders.
fn to_mcp_result(output: ToolOutput) -> CallToolResult {
    let mut content = vec![ContentBlock::text(output.text)];
    for picture in output.pictures {
        content.push(ContentBlock::image(
            base64::engine::general_purpose::STANDARD.encode(&picture.bytes),
            picture.media_type,
        ));
    }
    if output.is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}
