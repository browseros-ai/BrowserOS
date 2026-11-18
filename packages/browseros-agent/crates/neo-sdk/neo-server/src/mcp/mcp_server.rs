//! rmcp's `ServerHandler`: the one tool, `run`.

use std::borrow::Cow;
use std::future::{self, Future};
use std::sync::Arc;

use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    Implementation, InitializeResult, JsonObject, ListToolsResult, PaginatedRequestParams,
    ProtocolVersion, ServerCapabilities, ServerInfo, SubscriptionFilter, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler};
use serde_json::Value;

use super::{RunArgs, RunTool};
use crate::dispatch::{OperationError, Registry};
use crate::run::{RUN_META_KEY, RunMeta, RunRequest, RunService, RunTimeout};

/// The server's name in `initialize` and `server/discover`.
const SERVER_NAME: &str = "browseros-claw-sdk";

/// The stateless 2026-07-28 revision first, then the legacy ones. rmcp picks per request from
/// what a client negotiates, and serves all of them without a session.
const SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] = &[
    ProtocolVersion::V_2026_07_28,
    ProtocolVersion::V_2025_11_25,
    ProtocolVersion::V_2025_06_18,
    ProtocolVersion::V_2025_03_26,
    ProtocolVersion::V_2024_11_05,
];

/// The handler. Stateless rmcp builds one per request (the service factory runs each time), so
/// it is two `Arc`s, and the tool's definition is built once, at startup.
#[derive(Clone)]
pub(crate) struct McpServer {
    runs: Arc<RunService>,
    tool: Arc<RunTool>,
}

/// A call refused at the boundary, with what is known of it, so the refusal is recorded.
struct Refused {
    code: String,
    meta: RunMeta,
    error: OperationError,
}

impl McpServer {
    /// The server, with `run`'s description built from the frozen registry.
    #[must_use]
    pub(crate) fn new(runs: Arc<RunService>, registry: &Registry) -> Self {
        Self {
            runs,
            tool: Arc::new(RunTool::new(registry)),
        }
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(SUPPORTED_PROTOCOL_VERSIONS)
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        let result = ListToolsResult::with_all_items(vec![self.tool.tool()]);
        // 2026-07-28 clients require a cache hint on list results (SEP-2549); older peers keep
        // the old wire shape. The one tool is the same for every caller.
        let modern = context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28);
        future::ready(Ok(if modern {
            result.with_ttl_ms(0).with_cache_scope(CacheScope::Public)
        } else {
            result
        }))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        // rmcp validates 2026-07-28's per-parameter headers against this schema.
        (name == RunTool::NAME).then(|| self.tool.tool())
    }

    // A 2026-07-28 client may open `subscriptions/listen`; rmcp's default refuses it with a
    // 404 that ends the transport before `tools/list` runs. Accept what the capabilities
    // offer (nothing but tools), so the stream opens and stays quiet.
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
        if request.name != RunTool::NAME {
            return Err(ErrorData::invalid_params(
                format!(
                    "unknown tool `{}`; claw-sdk serves one tool, `run`",
                    request.name
                ),
                None,
            ));
        }
        // `_meta` arrives on the context: rmcp moves it out of the request params.
        let reply = match parse(request.arguments, context.meta.get(RUN_META_KEY)) {
            Ok(run) => tokio::select! {
                reply = self.runs.run(run) => reply,
                // The client's response stream is gone, which is the only cancel stateless rmcp
                // delivers (it drops `notifications/cancelled`). Dropping the run's future is
                // leaving it: a run without a requestId stops at once, one with a requestId
                // waits out its orphan grace for a retry. Nobody reads this result.
                () = context.ct.cancelled() => {
                    return Ok(CallToolResult::error(vec![ContentBlock::text("the caller went away")]).into());
                }
            },
            Err(refused) => {
                let Refused { code, meta, error } = *refused;
                self.runs.refuse(code, meta, &error)
            }
        };
        Ok(RunTool::result(&reply).into())
    }
}

/// The call in domain types. The host's `_meta` is read first, so a call refused for its
/// arguments still records the task it came from.
fn parse(arguments: Option<JsonObject>, meta: Option<&Value>) -> Result<RunRequest, Box<Refused>> {
    let meta = match RunMeta::parse(meta) {
        Ok(meta) => meta,
        Err(error) => {
            let code = RunArgs::parse(arguments)
                .map(|args| args.code)
                .unwrap_or_default();
            return Err(Box::new(Refused {
                code,
                meta: RunMeta::default(),
                error,
            }));
        }
    };
    let args = match RunArgs::parse(arguments) {
        Ok(args) => args,
        Err(error) => {
            return Err(Box::new(Refused {
                code: String::new(),
                meta,
                error,
            }));
        }
    };
    match RunTimeout::from_millis(args.timeout) {
        Ok(timeout) => Ok(RunRequest {
            code: args.code,
            timeout,
            meta,
        }),
        Err(error) => Err(Box::new(Refused {
            code: args.code,
            meta,
            error,
        })),
    }
}
