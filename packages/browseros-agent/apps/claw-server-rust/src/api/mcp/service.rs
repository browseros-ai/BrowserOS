use crate::{
    AppState, VERSION,
    api::mcp::{
        dispatch::{
            ToolCall, ToolIdentity, dispatch_tool_call, linked_cancel_token,
            operator_cancellation_result,
        },
        effects::tab_groups::apply_agent_tab_group_title,
        naming::{build_session_group_title, client_prefix_from_slug, normalize_small_name},
        observers::audit::{LocalToolDispatch, record_local_tool_dispatch},
        prompt::BROWSERCLAW_MCP_INSTRUCTIONS,
    },
    identity::{ClientIdentity, ClientInfo, ProfileView},
    ids::{DispatchId, SessionId},
    services::sessions::Session,
};
use browseros_mcp::{OutputFileAccess, ToolDef, ToolResult, catalog};
use rmcp::{
    ErrorData as McpError, RoleServer,
    handler::server::ServerHandler,
    model::{
        CallToolRequestMethod, CallToolRequestParams, CallToolResult, Implementation,
        InitializeRequestParams, InitializeResult, JsonObject, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, Tool, ToolAnnotations,
    },
    service::{NotificationContext, RequestContext},
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant as StdInstant,
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use ulid::Ulid;

const SERVER_NAME: &str = "browserclaw";
const SERVER_TITLE: &str = "BrowserClaw";
const NAME_SESSION_TOOL_NAME: &str = "name_session";
const NAME_SESSION_DESCRIPTION: &str = "Rename this browser session: a small lowercase 2-3 word label for what this session is doing, e.g. \"invoice processing\". Tabs are grouped as <client>/<name>. Call again to rename.";
const NAME_SESSION_INPUT_MAX_LEN: usize = 64;

/// Owns one MCP transport lifetime. Drop best-effort schedules removal of a started
/// server session, which records its end and begins retained-group handling.
pub struct ClawMcpService {
    state: AppState,
    catalog: Arc<Vec<ToolDef>>,
    name_session_tool: Tool,
    output_files: OutputFileAccess,
    lifecycle: Arc<Mutex<ServiceLifecycle>>,
    fallback_session_id: SessionId,
    closed: AtomicBool,
}

#[derive(Default)]
struct ServiceLifecycle {
    client_info: Option<ClientInfo>,
    session_id: Option<SessionId>,
    started: bool,
}

#[derive(Clone)]
struct StartedSession {
    session: Arc<Session>,
    agent_label: String,
}

impl ClawMcpService {
    /// Creates the BrowserClaw-owned rmcp server over the shared browser tool catalog.
    #[must_use]
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            catalog: Arc::new(catalog()),
            name_session_tool: name_session_tool(),
            output_files: browseros_mcp::output_file::create_browser_output_file_access(),
            lifecycle: Arc::new(Mutex::new(ServiceLifecycle::default())),
            fallback_session_id: SessionId::new(format!("stdio-{}", Ulid::new())),
            closed: AtomicBool::new(false),
        }
    }

    fn find_tool_index(&self, name: &str) -> Option<usize> {
        self.catalog.iter().position(|tool| tool.name == name)
    }

    fn listed_tools(&self) -> Vec<Tool> {
        let mut tools = self
            .catalog
            .iter()
            .map(ToolDef::to_mcp_tool)
            .collect::<Vec<_>>();
        tools.push(self.name_session_tool.clone());
        tools
    }

    async fn call_name_session(
        &self,
        started: &StartedSession,
        raw_args: &Value,
    ) -> CallToolResult {
        let dispatch_id = DispatchId::new();
        let dispatch_cancel = CancellationToken::new();
        if !started
            .session
            .try_register_dispatch(dispatch_id.clone(), dispatch_cancel)
            .await
        {
            return CallToolResult::error(vec![rmcp::model::ContentBlock::text(
                "BrowserClaw session is no longer live",
            )]);
        }
        let started_at = StdInstant::now();
        let rename = match rename_session(Some(started.session.as_ref()), raw_args).await {
            Ok(rename) => rename,
            Err(message) => {
                return finish_local_dispatch(
                    started.session.as_ref(),
                    &dispatch_id,
                    ToolResult::error(message),
                )
                .await
                .into_call_tool_result();
            }
        };
        let browser = self.state.browser.session().await;
        apply_agent_tab_group_title(
            browser.as_ref(),
            &self.state.sessions.ownership(),
            started.session.convo_id(),
            started.session.as_ref(),
            started.session.child_token(),
        )
        .await;
        let result = ToolResult::text(rename.response, None);
        if let Err(error) = record_local_tool_dispatch(
            &self.state,
            LocalToolDispatch {
                session: &started.session,
                agent_label: &started.agent_label,
                tool_name: NAME_SESSION_TOOL_NAME,
                raw_args,
                result: &result,
                duration_ms: i64::try_from(started_at.elapsed().as_millis()).unwrap_or(i64::MAX),
                dispatch_id: dispatch_id.clone(),
            },
        )
        .await
        {
            warn!(error = %error, "local tool audit submission failed");
        }
        finish_local_dispatch(started.session.as_ref(), &dispatch_id, result)
            .await
            .into_call_tool_result()
    }

    async fn set_client_info(&self, request: &InitializeRequestParams) {
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
    }

    async fn ensure_session_started(
        &self,
        session_id: SessionId,
    ) -> Result<StartedSession, McpError> {
        let mut lifecycle = self.lifecycle.lock().await;
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
                    McpError::invalid_request(
                        format!("BrowserClaw session {session_id} is no longer live"),
                        None,
                    )
                })?
        } else if let Some(session) = self.state.sessions.lookup(&session_id).await {
            lifecycle.started = true;
            session
        } else {
            let profiles = self.state.profiles.list_profiles().await.map_err(|error| {
                McpError::internal_error(format!("agent profile lookup failed: {error}"), None)
            })?;
            let profiles = profiles.iter().map(ProfileView::from).collect::<Vec<_>>();
            let agent = ClientIdentity::resolve(&client, &profiles);
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
                agent = %session.convo_id(),
                "mcp session initialized"
            );
            session
        };
        let agent_label = client
            .title
            .as_deref()
            .filter(|value| !value.is_empty())
            .or_else(|| (!client.name.is_empty()).then_some(client.name.as_str()))
            .unwrap_or_else(|| session.agent().slug())
            .to_string();
        Ok(StartedSession {
            session,
            agent_label,
        })
    }

    async fn learn_session_from_request(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Result<StartedSession, McpError> {
        let session_id = session_id_from_extensions(&context.extensions)
            .unwrap_or_else(|| self.fallback_session_id.clone());
        self.ensure_session_started(session_id).await
    }

    async fn learn_session_from_notification(&self, context: &NotificationContext<RoleServer>) {
        let session_id = session_id_from_extensions(&context.extensions)
            .unwrap_or_else(|| self.fallback_session_id.clone());
        if let Err(error) = self.ensure_session_started(session_id).await {
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
        let mut implementation = Implementation::new(SERVER_NAME, VERSION);
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
        self.set_client_info(&request).await;
        let info = self.get_info();
        let Some(session_id) = session_id_from_extensions(&context.extensions) else {
            return Ok(info);
        };
        let _ = self.ensure_session_started(session_id).await?;
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
        std::future::ready(Ok(ListToolsResult::with_all_items(self.listed_tools())))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        if name == NAME_SESSION_TOOL_NAME {
            return Some(self.name_session_tool.clone());
        }
        self.find_tool_index(name)
            .map(|index| self.catalog[index].to_mcp_tool())
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let is_name_session = request.name == NAME_SESSION_TOOL_NAME;
        let tool_index = self.find_tool_index(&request.name);
        if !is_name_session && tool_index.is_none() {
            return Err(McpError::method_not_found::<CallToolRequestMethod>());
        }
        let raw_args = request
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(JsonObject::new()));
        let started = self.learn_session_from_request(&context).await?;
        started.session.touch(tokio::time::Instant::now()).await;
        started.session.mark_used();
        let concurrent_used_sessions = self.state.sessions.used_count().await.max(1);
        let tool_started_at = tokio::time::Instant::now();
        let tool_name = request.name.to_string();

        let result = if is_name_session {
            Ok(self.call_name_session(&started, &raw_args).await)
        } else {
            let Some(tool_index) = tool_index else {
                unreachable!("catalog tool was validated before session resolution");
            };
            let browser_session = self.state.browser.session().await;
            let ownership_key = started.session.convo_id().clone();
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
                Some(identity),
                browser_session,
                cancel,
                context.ct.clone(),
                dispatch_cancel,
                default_tab_group_id,
                self.state.clone(),
                self.output_files.clone(),
            );
            dispatch_tool_call(call).await
        };

        finish_tool_call(
            started.session.as_ref(),
            &tool_name,
            tool_started_at,
            concurrent_used_sessions,
            result,
        )
        .await
    }
}

async fn finish_tool_call(
    session: &Session,
    tool_name: &str,
    started_at: tokio::time::Instant,
    concurrent_used_sessions: usize,
    result: Result<CallToolResult, McpError>,
) -> Result<CallToolResult, McpError> {
    session
        .record_tool_usage(tool_name, started_at.elapsed(), concurrent_used_sessions)
        .await;
    result
}

#[derive(Debug, PartialEq, Eq)]
struct SessionRename {
    response: String,
}

/// Validates and commits a session rename before any browser-side title synchronization.
async fn rename_session(
    session: Option<&Session>,
    raw_args: &Value,
) -> Result<SessionRename, &'static str> {
    let Some(session) = session else {
        return Err("unable to resolve this session");
    };
    let Some(raw_name) = raw_args.get("name").and_then(Value::as_str) else {
        return Err("name must be a string");
    };
    if raw_name.chars().count() > NAME_SESSION_INPUT_MAX_LEN {
        return Err("name must be at most 64 characters");
    }
    let label = normalize_small_name(raw_name);
    if label.is_empty() {
        return Err("name must contain a usable session name");
    }

    let prefix = client_prefix_from_slug(session.agent().slug());
    let old_label = session.rename(label.clone()).await;
    let old_title = build_session_group_title(prefix, &old_label);
    let new_title = build_session_group_title(prefix, &label);
    Ok(SessionRename {
        response: format!("renamed to {new_title} (was {old_title})"),
    })
}

fn name_session_tool() -> Tool {
    let Value::Object(input_schema) = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "maxLength": NAME_SESSION_INPUT_MAX_LEN }
        },
        "required": ["name"]
    }) else {
        unreachable!();
    };
    Tool::new(
        NAME_SESSION_TOOL_NAME,
        NAME_SESSION_DESCRIPTION,
        input_schema,
    )
    .with_annotations(
        ToolAnnotations::with_title("Name session")
            .read_only(false)
            .destructive(false)
            .idempotent(true),
    )
}

fn clean_client_field(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

async fn finish_local_dispatch(
    session: &Session,
    dispatch_id: &DispatchId,
    result: ToolResult,
) -> ToolResult {
    if !session.finish_dispatch(dispatch_id).await && session.operator_stop_requested() {
        operator_cancellation_result()
    } else {
        result
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
    use crate::identity::ConversationIdentity;
    use rmcp::handler::server::ServerHandler;
    use serde_json::json;

    fn usage_session() -> Arc<Session> {
        Session::new(
            SessionId::new("usage-session"),
            ClientIdentity::Ephemeral {
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            ConversationIdentity::new("codex", "usage-test".to_string()),
            "Codex".to_string(),
            tokio::time::Instant::now(),
        )
    }

    #[tokio::test(start_paused = true)]
    async fn completed_browser_tool_success_is_recorded_and_returned_unchanged() {
        let session = usage_session();
        assert_eq!(session.usage_snapshot().await.dispatch_count, 0);
        let result = CallToolResult::success(vec![rmcp::model::ContentBlock::text("ok")]);
        let expected = result.clone();
        let started_at = tokio::time::Instant::now();
        tokio::time::advance(std::time::Duration::from_millis(40)).await;

        let returned = finish_tool_call(session.as_ref(), "tabs", started_at, 2, Ok(result)).await;

        assert_eq!(returned, Ok(expected));
        let snapshot = session.usage_snapshot().await;
        assert_eq!(snapshot.dispatch_count, 1);
        assert_eq!(snapshot.max_concurrent_used_sessions, 2);
        assert_eq!(snapshot.tools[0].tool_name, "tabs");
        assert_eq!(snapshot.tools[0].total_duration_ms, 40);
    }

    #[tokio::test(start_paused = true)]
    async fn completed_local_tool_error_result_is_recorded_and_returned_unchanged() {
        let session = usage_session();
        let result = CallToolResult::error(vec![rmcp::model::ContentBlock::text("invalid")]);
        let expected = result.clone();
        let started_at = tokio::time::Instant::now();
        tokio::time::advance(std::time::Duration::from_millis(12)).await;

        let returned =
            finish_tool_call(session.as_ref(), "name_session", started_at, 1, Ok(result)).await;

        assert_eq!(returned, Ok(expected));
        let snapshot = session.usage_snapshot().await;
        assert_eq!(snapshot.dispatch_count, 1);
        assert_eq!(snapshot.tools[0].tool_name, "name_session");
        assert_eq!(snapshot.tools[0].max_duration_ms, 12);
    }

    #[tokio::test]
    async fn local_tool_returns_cancellation_when_operator_stop_wins() -> anyhow::Result<()> {
        let session = usage_session();
        let dispatch_id = DispatchId::new();
        assert!(
            session
                .try_register_dispatch(dispatch_id.clone(), CancellationToken::new())
                .await
        );
        session.request_operator_stop();
        assert_eq!(session.stop_dispatches().await, 1);

        let result = finish_local_dispatch(
            session.as_ref(),
            &dispatch_id,
            ToolResult::text("renamed", None),
        )
        .await;

        assert!(result.is_error);
        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value["cancellationKind"].as_str()),
            Some("cockpit.operator-cancelled")
        );
        assert_eq!(
            session.pending_operator_cancellation_audits().await,
            [dispatch_id]
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn completed_protocol_error_is_recorded_and_returned_unchanged() {
        let session = usage_session();
        let error = McpError::internal_error("dispatch failed", None);
        let expected = error.clone();
        let started_at = tokio::time::Instant::now();
        tokio::time::advance(std::time::Duration::from_millis(7)).await;

        let returned =
            finish_tool_call(session.as_ref(), "navigate", started_at, 3, Err(error)).await;

        assert_eq!(returned, Err(expected));
        let snapshot = session.usage_snapshot().await;
        assert_eq!(snapshot.dispatch_count, 1);
        assert_eq!(snapshot.max_concurrent_used_sessions, 3);
        assert_eq!(snapshot.tools[0].tool_name, "navigate");
        assert_eq!(snapshot.tools[0].total_duration_ms, 7);
    }

    #[tokio::test]
    async fn initialize_info_uses_browserclaw_branding_and_prompt() -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call("tabs", json!({})).await?;
        let service = ClawMcpService::new(call.state);
        let info = service.get_info();
        assert_eq!(info.server_info.name, SERVER_NAME);
        assert_eq!(info.server_info.version, VERSION);
        assert_eq!(info.server_info.title.as_deref(), Some(SERVER_TITLE));
        assert_eq!(
            info.instructions.as_deref(),
            Some(BROWSERCLAW_MCP_INSTRUCTIONS)
        );
        let instructions = info
            .instructions
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("BrowserClaw instructions missing"))?;
        assert!(instructions.contains("BrowserClaw — the browser for agents"));
        assert!(instructions.contains(
            "- Rename your session early with name_session using a 2-3 word task label;\n  tabs group as <client>/<name>."
        ));
        assert!(instructions.contains(
            "- If the user points you at a tab you don't own, open its URL with\n  tabs action=\"new\" and work on that copy; leave the original untouched."
        ));
        assert!(!instructions.contains("touch a tab"));
        assert!(!instructions.contains("close them when done"));
        Ok(())
    }

    #[tokio::test]
    async fn name_session_schema_and_annotations_are_registered_locally() -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call("tabs", json!({})).await?;
        let service = ClawMcpService::new(call.state);
        let listed = service
            .listed_tools()
            .into_iter()
            .find(|tool| tool.name == NAME_SESSION_TOOL_NAME)
            .ok_or_else(|| anyhow::anyhow!("name_session missing from list"))?;
        let fetched = service
            .get_tool(NAME_SESSION_TOOL_NAME)
            .ok_or_else(|| anyhow::anyhow!("name_session missing from get_tool"))?;

        assert_eq!(listed, fetched);
        assert_eq!(
            listed.description.as_deref(),
            Some(NAME_SESSION_DESCRIPTION)
        );
        assert_eq!(
            Value::Object(listed.input_schema.as_ref().clone()),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "maxLength": 64 }
                },
                "required": ["name"]
            })
        );
        assert_eq!(
            listed.annotations,
            Some(
                ToolAnnotations::with_title("Name session")
                    .read_only(false)
                    .destructive(false)
                    .idempotent(true)
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn name_session_validates_and_renames_without_a_browser() -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call("tabs", json!({})).await?;
        let session = call
            .identity
            .as_ref()
            .map(|identity| identity.session.clone())
            .ok_or_else(|| anyhow::anyhow!("session missing"))?;
        let generated = session.generated_label().to_string();

        let first = rename_session(
            Some(session.as_ref()),
            &json!({ "name": "  Invoice Processing!!!  " }),
        )
        .await
        .map_err(anyhow::Error::msg)?;
        assert_eq!(
            first.response,
            format!("renamed to codex/invoice-processing (was codex/{generated})")
        );
        assert_eq!(session.label().await, "invoice-processing");

        let second = rename_session(
            Some(session.as_ref()),
            &json!({ "name": "Quarterly Reporting" }),
        )
        .await
        .map_err(anyhow::Error::msg)?;
        assert_eq!(
            second.response,
            "renamed to codex/quarterly-reporting (was codex/invoice-processing)"
        );

        let current = session.label().await;
        assert_eq!(
            rename_session(Some(session.as_ref()), &json!({ "name": "!!!" })).await,
            Err("name must contain a usable session name")
        );
        assert_eq!(
            rename_session(Some(session.as_ref()), &json!({ "name": "x".repeat(65) })).await,
            Err("name must be at most 64 characters")
        );
        assert_eq!(session.label().await, current);
        assert_eq!(
            rename_session(None, &json!({ "name": "invoice processing" })).await,
            Err("unable to resolve this session")
        );
        Ok(())
    }
}
