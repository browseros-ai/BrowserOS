//! Dynamic connector tools for the shared MCP host. Remote JSON schemas/results
//! pass through intact; the local helper produces the same inventory/auth payload
//! as TS. Lease policy is resolved by the product that owns the opaque caller.

use super::Klavis;
use crate::telemetry::metrics::Metrics;
use browseros_mcp_host::{AdmittedCaller, LocalToolProvider, seams::LocalToolCall};
use futures_util::future::BoxFuture;
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock, Tool};
use serde_json::{Value, json};
use std::{sync::Arc, time::Instant};
use tokio_util::sync::CancellationToken;

const HELPER: &str = "connector_mcp_servers";
const DESCRIPTION: &str = "Check or list BrowserOS managed app connectors before using Strata MCP tools. Omit server_name to see available, selected, connected, and proxy status. With server_name, returns connected/auth URL status.";

/// Per-call product policy. `rejection` must be recomputed from the live run,
/// rather than captured when tools/list ran, because leases outlive turns.
#[derive(Default, Clone)]
pub struct ConnectorScope {
    pub selected_server_names: Vec<String>,
    pub rejection: Option<String>,
}
pub type ScopeResolver = Arc<dyn Fn(Option<&AdmittedCaller>) -> ConnectorScope + Send + Sync>;

pub(super) struct Provider {
    pub klavis: Arc<Klavis>,
    pub scope: ScopeResolver,
    pub metrics: Metrics,
}

impl LocalToolProvider for Provider {
    fn tools<'a>(&'a self, _caller: Option<&'a AdmittedCaller>) -> BoxFuture<'a, Vec<Tool>> {
        Box::pin(async move {
            if self.klavis.client.id.is_empty() {
                return vec![];
            }
            let description = Klavis::catalog()
                .iter()
                .map(|s| format!("{} ({})", s.name, s.description))
                .collect::<Vec<_>>()
                .join(", ");
            let schema = json!({"type":"object","properties":{"server_name":{
                "type":"string","enum":Klavis::catalog().iter().map(|s| s.name).collect::<Vec<_>>(),
                "description":format!("The optional service to check. Omit to list connector inventory. Available: {description}")
            }},"additionalProperties":false});
            let mut tools = vec![Tool::new(
                HELPER,
                DESCRIPTION,
                Arc::new(schema.as_object().cloned().unwrap_or_default()),
            )];
            if let Some(session) = &self
                .klavis
                .shared
                .snapshot
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .session
            {
                tools.extend(session.tools.clone());
            }
            tools
        })
    }

    fn call<'a>(&'a self, name: &'a str, call: LocalToolCall<'a>) -> BoxFuture<'a, CallToolResult> {
        Box::pin(async move {
            let scope = (self.scope)(call.caller);
            if let Some(reason) = &scope.rejection {
                return error(reason);
            }
            if call.caller.is_some_and(|c| c.read_only) {
                return error("Managed connector tools are unavailable in read-only mode.");
            }
            let cancel = CancellationToken::new();
            let Some(dispatch) = call.begin(cancel.clone()).await else {
                return call.session_gone();
            };
            let started = Instant::now();
            let result = tokio::select! {
                biased;
                _ = call.request_cancel.cancelled() => error("Klavis tool call cancelled"),
                _ = cancel.cancelled() => error("Klavis tool call cancelled"),
                _ = self.klavis.cancel.cancelled() => error("Klavis proxy stopped"),
                result = self.execute(name, call.args, scope) => result,
            };
            // TS reports connector calls with `source: 'mcp'` whoever the caller is.
            self.metrics.capture(
                "tool_executed",
                json!({
                    "tool_name": name,
                    "source": "mcp",
                    "duration_ms": started.elapsed().as_millis() as u64,
                    "success": result.is_error != Some(true),
                }),
            );
            // Finish bookkeeping without translating the remote result: ToolResult
            // would flatten structuredContent, image blocks, and upstream isError.
            if !call.session.finish_dispatch(&dispatch).await
                && call.session.operator_stop_requested()
            {
                return error("Klavis tool call cancelled by operator");
            }
            result
        })
    }
}

impl Provider {
    async fn execute(&self, name: &str, args: &Value, scope: ConnectorScope) -> CallToolResult {
        if name == HELPER {
            return match self.payload(args, scope).await {
                Ok(payload) => {
                    CallToolResult::success(vec![ContentBlock::text(payload.to_string())])
                }
                Err(message) => error(&message),
            };
        }
        let session = self
            .klavis
            .shared
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .session
            .clone();
        let Some(session) = session else {
            return error("Klavis proxy unavailable");
        };
        let mut params = CallToolRequestParams::new(name.to_owned());
        params.arguments = args.as_object().cloned();
        match tokio::time::timeout(super::client::TIMEOUT, session.peer.call_tool(params)).await {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => error(&e.to_string()),
            Err(_) => error(&format!("Klavis callTool({name}) timed out")),
        }
    }

    async fn payload(&self, args: &Value, scope: ConnectorScope) -> Result<Value, String> {
        if let Some(name) = args
            .get("server_name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            && !Klavis::catalog().iter().any(|s| s.name == name)
        {
            return Err(format!("Invalid server: {name}"));
        }
        let integrations = self
            .klavis
            .user_integrations()
            .await
            .map_err(|e| e.to_string())?;
        let proxy = self.klavis.proxy_status();
        if let Some(name) = args
            .get("server_name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            if integrations
                .iter()
                .any(|i| i.name == name && i.is_authenticated)
            {
                return Ok(json!({"connected":true,"server_name":name,"proxy":proxy}));
            }
            let intent = self
                .klavis
                .add_server(name)
                .await
                .map_err(|e| e.to_string())?;
            let auth_url = intent.oauth_url.or(intent.api_key_url);
            let message = auth_url.as_ref().map(|url| format!("{name} is not connected. Ask the user to open this URL to authenticate: {url}"))
                .unwrap_or_else(|| format!("{name} is not connected. Could not retrieve auth URL."));
            return Ok(
                json!({"connected":false,"server_name":name,"authUrl":auth_url,"proxy":proxy,"message":message}),
            );
        }
        let mut selected = Vec::new();
        for name in scope.selected_server_names {
            if !selected.contains(&name) {
                selected.push(name);
            }
        }
        // Inventory's connected rows use the service's camelCase auth field;
        // the HTTP integrations endpoint separately normalizes to snake_case.
        let connected: Vec<_> = integrations
            .iter()
            .filter(|i| i.is_authenticated)
            .map(|i| json!({"name":i.name,"isAuthenticated":true}))
            .collect();
        Ok(
            json!({"available":Klavis::catalog(),"connected":connected,"selected":selected,"proxy":proxy}),
        )
    }
}
fn error(message: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}
