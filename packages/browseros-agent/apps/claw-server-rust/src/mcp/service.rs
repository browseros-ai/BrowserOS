use crate::{
    AppState,
    domain::{AgentRef, ClientInfo, Session, SessionId},
    mcp::{
        dispatch::{ToolCall, ToolIdentity, dispatch_tool_call, linked_cancel_token},
        prompt::BROWSERCLAW_MCP_INSTRUCTIONS,
    },
};
use browseros_mcp::{OutputFileAccess, ToolDef, catalog};
use rmcp::{
    ErrorData as McpError, RoleServer,
    handler::server::ServerHandler,
    model::{
        CallToolRequestMethod, CallToolRequestParams, CallToolResult, Implementation,
        InitializeRequestParams, InitializeResult, JsonObject, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, Tool,
    },
    service::{NotificationContext, Peer, RequestContext},
};
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use ulid::Ulid;

const SERVER_NAME: &str = "browserclaw";
const SERVER_TITLE: &str = "BrowserClaw";

pub struct ClawMcpService {
    state: AppState,
    catalog: Arc<Vec<ToolDef>>,
    output_files: OutputFileAccess,
    lifecycle: Arc<Mutex<ServiceLifecycle>>,
    fallback_session_id: SessionId,
    closed: AtomicBool,
}

#[derive(Default)]
struct ServiceLifecycle {
    client_info: Option<ClientInfo>,
    session_id: Option<SessionId>,
    peer: Option<Peer<RoleServer>>,
    naming_started: Arc<AtomicBool>,
    started: bool,
}

#[derive(Clone)]
struct StartedSession {
    session: Arc<Session>,
    peer: Peer<RoleServer>,
    naming_started: Arc<AtomicBool>,
    agent_label: String,
}

impl ClawMcpService {
    /// Creates the BrowserClaw-owned rmcp server over the shared browser tool catalog.
    #[must_use]
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            catalog: Arc::new(catalog()),
            output_files: browseros_mcp::output_file::create_browser_output_file_access(),
            lifecycle: Arc::new(Mutex::new(ServiceLifecycle::default())),
            fallback_session_id: SessionId::new(format!("stdio-{}", Ulid::new())),
            closed: AtomicBool::new(false),
        }
    }

    fn find_tool_index(&self, name: &str) -> Option<usize> {
        self.catalog.iter().position(|tool| tool.name == name)
    }

    async fn set_client_info(&self, request: &InitializeRequestParams, peer: Peer<RoleServer>) {
        let mut lifecycle = self.lifecycle.lock().await;
        lifecycle.client_info = Some(ClientInfo {
            name: clean_client_field(&request.client_info.name, "agent"),
            version: clean_client_field(&request.client_info.version, "unknown"),
            title: request
                .client_info
                .title
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        });
        lifecycle.peer = Some(peer);
    }

    async fn ensure_session_started(
        &self,
        session_id: SessionId,
        peer: Peer<RoleServer>,
    ) -> Result<StartedSession, McpError> {
        let mut lifecycle = self.lifecycle.lock().await;
        lifecycle.peer = Some(peer.clone());
        if lifecycle.session_id.is_none() {
            lifecycle.session_id = Some(session_id.clone());
        }
        let session_id = lifecycle
            .session_id
            .clone()
            .unwrap_or_else(|| session_id.clone());
        let client = lifecycle.client_info.clone().unwrap_or_else(|| ClientInfo {
            name: "agent".to_string(),
            version: "unknown".to_string(),
            title: None,
        });

        let session = if lifecycle.started {
            self.state
                .sessions
                .lookup(&session_id)
                .await
                .ok_or_else(|| {
                    McpError::internal_error(
                        format!("mcp session {session_id} is not registered"),
                        None,
                    )
                })?
        } else if let Some(session) = self.state.sessions.lookup(&session_id).await {
            lifecycle.started = true;
            session
        } else {
            let profiles = self.state.agents.list_profiles().await.map_err(|error| {
                McpError::internal_error(format!("agent profile lookup failed: {error}"), None)
            })?;
            let agent = AgentRef::resolve(&session_id, &client, &profiles);
            let session = self
                .state
                .sessions
                .mint_with_id(session_id.clone(), agent, client.clone())
                .await
                .map_err(|error| {
                    McpError::internal_error(format!("mcp session start failed: {error}"), None)
                })?;
            lifecycle.started = true;
            tracing::info!(
                session_id = %session.id(),
                agent = %session.agent().agent_id(),
                "mcp session initialized"
            );
            session
        };
        let peer = lifecycle.peer.clone().unwrap_or(peer);
        let agent_label = client
            .title
            .as_deref()
            .filter(|value| !value.is_empty())
            .or_else(|| (!client.name.is_empty()).then_some(client.name.as_str()))
            .unwrap_or_else(|| session.agent().slug())
            .to_string();
        Ok(StartedSession {
            session,
            peer,
            naming_started: lifecycle.naming_started.clone(),
            agent_label,
        })
    }

    async fn learn_session_from_request(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Result<StartedSession, McpError> {
        let session_id = session_id_from_extensions(&context.extensions)
            .unwrap_or_else(|| self.fallback_session_id.clone());
        self.ensure_session_started(session_id, context.peer.clone())
            .await
    }

    async fn learn_session_from_notification(&self, context: &NotificationContext<RoleServer>) {
        let session_id = session_id_from_extensions(&context.extensions)
            .unwrap_or_else(|| self.fallback_session_id.clone());
        if let Err(error) = self
            .ensure_session_started(session_id, context.peer.clone())
            .await
        {
            warn!(error = %error, "mcp session start failed");
        }
    }
}

impl Drop for ClawMcpService {
    fn drop(&mut self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let state = self.state.clone();
        let lifecycle = self.lifecycle.clone();
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        handle.spawn(async move {
            let session_id = {
                let lifecycle = lifecycle.lock().await;
                lifecycle
                    .started
                    .then(|| lifecycle.session_id.clone())
                    .flatten()
            };
            let Some(session_id) = session_id else {
                return;
            };
            if let Err(error) = state
                .sessions
                .remove(&session_id, "closed", Some("transport closed"))
                .await
            {
                warn!(error = %error, session_id = %session_id, "mcp session close failed");
            }
        });
    }
}

impl ServerHandler for ClawMcpService {
    fn get_info(&self) -> InitializeResult {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        let mut implementation = Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION"));
        implementation.title = Some(SERVER_TITLE.to_string());
        InitializeResult::new(capabilities)
            .with_server_info(implementation)
            .with_instructions(BROWSERCLAW_MCP_INSTRUCTIONS)
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        context.peer.set_peer_info(request.clone());
        self.set_client_info(&request, context.peer.clone()).await;
        let info = self.get_info();
        let Some(session_id) = session_id_from_extensions(&context.extensions) else {
            return Ok(info);
        };
        let _ = self
            .ensure_session_started(session_id, context.peer.clone())
            .await?;
        Ok(info)
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        self.learn_session_from_notification(&context).await;
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let tools = self
            .catalog
            .iter()
            .map(ToolDef::to_mcp_tool)
            .collect::<Vec<_>>();
        std::future::ready(Ok(ListToolsResult::with_all_items(tools)))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.find_tool_index(name)
            .map(|index| self.catalog[index].to_mcp_tool())
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let Some(tool_index) = self.find_tool_index(&request.name) else {
            return Err(McpError::method_not_found::<CallToolRequestMethod>());
        };
        let raw_args = request
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(JsonObject::new()));
        let started = self.learn_session_from_request(&context).await?;
        started.session.touch(tokio::time::Instant::now()).await;
        let browser_session = self.state.browser.session().await;
        let ownership_key = started.session.agent().ownership_key();
        let default_tab_group_id = self
            .state
            .sessions
            .ownership()
            .tab_group_ref(&ownership_key)
            .await;
        let dispatch_cancel = CancellationToken::new();
        let cancel = linked_cancel_token(
            started.session.child_token(),
            context.ct.clone(),
            dispatch_cancel.clone(),
        );
        let identity = ToolIdentity {
            session: started.session.clone(),
            agent: started.session.agent().clone(),
            ownership_key,
            agent_label: started.agent_label,
        };
        let call = ToolCall::new(
            self.catalog.clone(),
            tool_index,
            raw_args,
            started.session.id().clone(),
            context.id,
            Some(identity),
            browser_session,
            cancel,
            context.ct.clone(),
            dispatch_cancel,
            default_tab_group_id,
            self.state.clone(),
            self.output_files.clone(),
            Some(started.peer),
            started.naming_started,
        );
        dispatch_tool_call(call).await
    }
}

fn clean_client_field(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn session_id_from_extensions(extensions: &rmcp::model::Extensions) -> Option<SessionId> {
    extensions
        .get::<axum::http::request::Parts>()
        .and_then(|parts| parts.headers.get("mcp-session-id"))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(SessionId::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::handler::server::ServerHandler;
    use serde_json::json;

    #[tokio::test]
    async fn initialize_info_uses_browserclaw_branding_and_prompt() -> anyhow::Result<()> {
        let call = crate::mcp::test_support::tool_call("tabs", json!({})).await?;
        let service = ClawMcpService::new(call.state);
        let info = service.get_info();
        assert_eq!(info.server_info.name, SERVER_NAME);
        assert_eq!(info.server_info.title.as_deref(), Some(SERVER_TITLE));
        assert_eq!(
            info.instructions.as_deref(),
            Some(BROWSERCLAW_MCP_INSTRUCTIONS)
        );
        assert!(info.instructions.as_deref().is_some_and(|instructions| {
            instructions.contains("BrowserClaw — the browser for agents")
        }));
        Ok(())
    }
}
