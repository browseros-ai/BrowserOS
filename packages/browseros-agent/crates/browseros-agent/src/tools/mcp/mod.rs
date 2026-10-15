//! MCP tool sources: an rmcp client per endpoint — the required internal
//! `http://127.0.0.1:<port>/mcp?structured=1[&read_only=1]` with the lease header, and optional
//! custom servers — exposing the server's tools the way `@ai-sdk/mcp` does (`schemas:'automatic'`
//! dynamic tools, `mcp-client.ts` `toolsFromDefinitions`), so wire chunks and persisted tool parts
//! match the TS server's.
//!
//! Lifecycle: `connect_mcp` connects and lists tools (each step bounded by 15 s, TS
//! `MCP_CLIENT_CONNECT`) and owns the client until it returns; afterwards the source owns it and
//! `close` ends the session (the agent's `dispose`). A call observes its cancellation token and
//! sends `notifications/cancelled` for the in-flight request before giving up.

mod detect;
mod sse;

use super::{ToolCallError, ToolKind, ToolOutput, ToolSource, ToolSourceError, ToolSpec};
use crate::messages::{ToolContentItem, ToolResultOutput};
use rmcp::{
    RoleClient, ServiceExt,
    model::{
        CallToolRequest, CallToolRequestParams, CancelledNotification,
        CancelledNotificationParam, ClientCapabilities, ClientInfo, ClientRequest, Implementation,
        ServerResult, Tool,
    },
    service::{Peer, PeerRequestOptions, RunningService},
    transport::{
        StreamableHttpClientTransport,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Map, Value};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// `TIMEOUTS.MCP_CLIENT_CONNECT`, applied to connect and to `tools/list` separately.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// The client name `@ai-sdk/mcp` uses when none is given. It is sent in `initialize` and is
/// `toolMetadata.clientName` on every MCP tool chunk and persisted tool part, so keeping it is what
/// makes Rust and TS conversations interchangeable.
const CLIENT_NAME: &str = "ai-sdk-mcp-client";
const CLIENT_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransport {
    StreamableHttp,
    /// `mcp-transport-detect.ts` (1 h cache), falling back to legacy SSE.
    Detect,
}

#[derive(Debug, Clone)]
pub struct McpEndpoint {
    /// Log label and error label: `browseros` for the internal server, `custom-<name>` otherwise.
    pub label: String,
    pub url: url::Url,
    /// Sent with every request (the internal server's `X-BrowserOS-Internal-Lease`).
    pub headers: Vec<(String, String)>,
    pub transport: McpTransport,
}

type Client = RunningService<RoleClient, ClientInfo>;

/// Connects and lists tools within 15 s each. On failure nothing is left running.
pub async fn connect_mcp(endpoint: McpEndpoint) -> Result<Arc<dyn ToolSource>, ToolSourceError> {
    let label = endpoint.label.clone();
    let unavailable = |message: String| ToolSourceError::Unavailable {
        label: label.clone(),
        message,
    };
    let timed_out = || ToolSourceError::Timeout {
        label: label.clone(),
        seconds: CONNECT_TIMEOUT.as_secs(),
    };

    let client = tokio::time::timeout(CONNECT_TIMEOUT, connect(&endpoint))
        .await
        .map_err(|_| timed_out())?
        .map_err(unavailable)?;
    let listed = tokio::time::timeout(CONNECT_TIMEOUT, client.list_all_tools()).await;
    let tools = match listed {
        Ok(Ok(tools)) => tools,
        // A transport can connect and then fail during tools/list; this function still owns the
        // partial client and must end it.
        Ok(Err(error)) => {
            let _ = client.cancel().await;
            return Err(unavailable(error.to_string()));
        }
        Err(_) => {
            let _ = client.cancel().await;
            return Err(timed_out());
        }
    };
    tracing::debug!(label = %endpoint.label, tool_count = tools.len(), "MCP client connected");
    let specs = tools.iter().map(tool_spec).collect();
    Ok(Arc::new(McpToolSource {
        label: endpoint.label,
        peer: client.peer().clone(),
        client: Mutex::new(Some(client)),
        specs,
    }))
}

async fn connect(endpoint: &McpEndpoint) -> Result<Client, String> {
    let transport = match endpoint.transport {
        McpTransport::StreamableHttp => detect::DetectedTransport::StreamableHttp,
        McpTransport::Detect => detect::detect(&endpoint.url).await,
    };
    let info = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new(CLIENT_NAME, CLIENT_VERSION),
    );
    match transport {
        detect::DetectedTransport::StreamableHttp => {
            let mut headers = HashMap::new();
            for (name, value) in &endpoint.headers {
                let name = http::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|error| error.to_string())?;
                let value =
                    http::HeaderValue::from_str(value).map_err(|error| error.to_string())?;
                headers.insert(name, value);
            }
            let config = StreamableHttpClientTransportConfig::with_uri(endpoint.url.as_str())
                .custom_headers(headers);
            info.serve(StreamableHttpClientTransport::from_config(config))
                .await
                .map_err(|error| error.to_string())
        }
        detect::DetectedTransport::LegacySse => {
            let transport = sse::LegacySseTransport::connect(&endpoint.url, &endpoint.headers)
                .await
                .map_err(|error| error.to_string())?;
            info.serve(transport).await.map_err(|error| error.to_string())
        }
    }
}

/// `toolsFromDefinitions` for `schemas: 'automatic'`: a dynamic tool whose schema gets
/// `properties` defaulted and `additionalProperties: false` (overwriting in place, as the JS
/// spread does), and whose metadata carries the client name, tool name, title and annotations.
fn tool_spec(tool: &Tool) -> ToolSpec {
    let raw = serde_json::to_value(tool).unwrap_or(Value::Null);
    let mut schema: Map<String, Value> = tool.input_schema.as_ref().clone();
    let properties = schema
        .get("properties")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    schema.insert("properties".to_owned(), properties);
    schema.insert("additionalProperties".to_owned(), Value::Bool(false));

    let annotations = raw.get("annotations").and_then(Value::as_object);
    let title = raw
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| annotations.and_then(|a| a.get("title")).and_then(Value::as_str))
        .map(str::to_owned);

    let mut metadata = Map::new();
    metadata.insert("clientName".to_owned(), Value::from(CLIENT_NAME));
    metadata.insert("toolName".to_owned(), Value::from(tool.name.as_ref()));
    if let Some(title) = &title {
        metadata.insert("title".to_owned(), Value::from(title.as_str()));
    }
    if let Some(annotations) = annotations {
        let mut hints = Map::new();
        for key in [
            "title",
            "readOnlyHint",
            "destructiveHint",
            "idempotentHint",
            "openWorldHint",
        ] {
            if let Some(value) = annotations.get(key).filter(|value| !value.is_null()) {
                hints.insert(key.to_owned(), value.clone());
            }
        }
        metadata.insert("annotations".to_owned(), Value::Object(hints));
    }
    if let Some(app) = app_metadata(raw.get("_meta")) {
        metadata.insert("app".to_owned(), app);
    }

    ToolSpec {
        name: tool.name.to_string(),
        description: tool.description.as_deref().unwrap_or_default().to_owned(),
        input_schema: Value::Object(schema),
        kind: ToolKind::Dynamic {
            title,
            metadata: Value::Object(metadata),
        },
    }
}

/// MCP Apps (`getMCPAppToolMeta`): a tool whose `_meta.ui.resourceUri` (or the legacy
/// `_meta["ui/resourceUri"]`) names a `ui://` resource carries it to the app, which can render it.
fn app_metadata(meta: Option<&Value>) -> Option<Value> {
    const MCP_APP_MIME_TYPE: &str = "text/html;profile=mcp-app";
    let meta = meta?.as_object()?;
    let ui = meta.get("ui").and_then(Value::as_object);
    let resource_uri = ui
        .and_then(|ui| ui.get("resourceUri"))
        .or_else(|| meta.get("ui/resourceUri"))
        .and_then(Value::as_str)
        .filter(|uri| uri.starts_with("ui://"))?;
    let mut app = ui.cloned().unwrap_or_default();
    app.insert("resourceUri".to_owned(), Value::from(resource_uri));
    app.insert("mimeType".to_owned(), Value::from(MCP_APP_MIME_TYPE));
    Some(Value::Object(app))
}

struct McpToolSource {
    label: String,
    /// Cloned out of `client` so calls never contend for the lock `close` takes.
    peer: Peer<RoleClient>,
    client: Mutex<Option<Client>>,
    specs: Vec<ToolSpec>,
}

#[async_trait::async_trait]
impl ToolSource for McpToolSource {
    fn label(&self) -> &str {
        &self.label
    }

    /// Listed once at connect, as the TS agent does per session.
    async fn tools(&self) -> Result<Vec<ToolSpec>, ToolSourceError> {
        Ok(self.specs.clone())
    }

    async fn call(
        &self,
        name: &str,
        input: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput, ToolCallError> {
        if cancel.is_cancelled() {
            return Err(ToolCallError("This operation was aborted".to_owned()));
        }
        let mut params = CallToolRequestParams::new(name.to_owned());
        if let Value::Object(arguments) = input {
            params = params.with_arguments(arguments);
        }
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        let handle = self
            .peer
            .send_cancellable_request(request, PeerRequestOptions::no_options())
            .await
            .map_err(|error| ToolCallError(error.to_string()))?;
        let request_id = handle.id.clone();
        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                // Tell the server to stop work it may still be doing for us (a click, a
                // navigation); the response, if it ever comes, is dropped by rmcp.
                let notification = CancelledNotification::new(CancelledNotificationParam::new(
                    Some(request_id),
                    Some("AbortError: Conversation stopped".to_owned()),
                ));
                let _ = self.peer.send_notification(notification.into()).await;
                return Err(ToolCallError("This operation was aborted".to_owned()));
            }
            response = handle.await_response() => response,
        };
        match response {
            Ok(ServerResult::CallToolResult(result)) => {
                let ui = serde_json::to_value(&result)
                    .map_err(|error| ToolCallError(error.to_string()))?;
                let model = mcp_to_model_output(&ui);
                Ok(ToolOutput { ui, model })
            }
            Ok(_) => Err(ToolCallError(format!(
                "MCP server \"{}\" returned an unexpected response to tools/call",
                self.label
            ))),
            Err(rmcp::service::ServiceError::McpError(error)) => Err(ToolCallError(error.message.into_owned())),
            Err(error) => Err(ToolCallError(error.to_string())),
        }
    }

    async fn close(&self) {
        if let Some(client) = self.client.lock().await.take() {
            let _ = client.cancel().await;
        }
    }
}

/// `mcpToModelOutput`: text items stay text, images become media, anything else is its JSON text;
/// a result without a `content` array goes back as JSON. `isError` results are ordinary outputs
/// here — the model reads the error text, and the wire shows `output.isError: true`.
fn mcp_to_model_output(result: &Value) -> ToolResultOutput {
    let Some(content) = result.get("content").and_then(Value::as_array) else {
        return ToolResultOutput::Json {
            value: result.clone(),
        };
    };
    let items = content
        .iter()
        .map(|part| {
            let kind = part.get("type").and_then(Value::as_str);
            match (kind, part.get("text"), part.get("data"), part.get("mimeType")) {
                (Some("text"), Some(text), _, _) => ToolContentItem::Text {
                    text: text.as_str().map_or_else(|| text.to_string(), str::to_owned),
                },
                (Some("image"), _, Some(Value::String(data)), Some(Value::String(mime))) => {
                    ToolContentItem::Media {
                        data: data.clone(),
                        media_type: mime.clone(),
                    }
                }
                _ => ToolContentItem::Text {
                    text: part.to_string(),
                },
            }
        })
        .collect();
    ToolResultOutput::Content { value: items }
}
