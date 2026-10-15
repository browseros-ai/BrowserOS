//! The rmcp server every transport session runs: session admission (legacy transport
//! sessions and modern stateless handles, Stop, successors), tool listing with the
//! session/agentName decoration, local-tool routing, and catalog admission into
//! `dispatch`. One instance per transport session; `HostCtx` is shared by all.

use crate::{
    branding::HostBranding,
    ctx::HostCtx,
    identity::{ClientIdentity, ClientInfo},
    ids::SessionId,
    mcp::dispatch::{ToolCall, ToolIdentity, dispatch_tool_call, linked_cancel_token},
    seams::{LocalTool, LocalToolCall},
    sessions::{RetirementCause, Session, TaskTargetStore},
};
use browseros_core::target::BrowserView;
use browseros_mcp::{OutputFileAccess, TargetStore, ToolDef, catalog};
use rmcp::{
    ErrorData as McpError, RoleServer,
    handler::server::ServerHandler,
    model::{
        CacheScope, CallToolRequestMethod, CallToolRequestParams, CallToolResponse, CallToolResult,
        Implementation, InitializeRequestParams, InitializeResult, JsonObject, ListToolsResult,
        MetaObject, PaginatedRequestParams, ProtocolVersion, ServerCapabilities,
        SubscriptionFilter, Tool,
    },
    service::{NotificationContext, RequestContext},
};
use serde_json::{Value, json};
use std::{
    borrow::Cow,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use ulid::Ulid;
use uuid::Uuid;

const AGENT_NAME_ARG: &str = "agentName";

/// Description of the `session` argument every tool gains. Scoped to the path that
/// actually emits a handle: the handle rides only on connections without a transport
/// session id; the other path withholds it and ignores one that is presented, so
/// promising it to every client pointed those agents at a control they do not have and
/// away from the tab group id, which is the signal that works for them. Naming the
/// alternative here matters because this text is read at the moment an agent is
/// deciding how to keep continuity.
#[must_use]
pub fn session_arg_description(session_meta_key: &str) -> String {
    format!(
        "Opaque logical task handle, including its browser_target binding. You receive one only if your client connects without its own transport session id: the server returns it in every tool result's `_meta` under `{session_meta_key}` and in result text. Pass it as `session` on later calls; natural successors retain the task's latest target even across server restart. If you never see one, your continuity across a remade connection is your tab group id instead: pass a known groupId on tabs action=\"new\" to recover its task binding before creation. Existing tabs keep their actual ownership. Omit session on the first call or to start a separate task after Stop; a stopped handle stays stopped. Stale browser targets require explicit browser_target reselection or clear."
    )
}
const AGENT_NAME_ARG_DESCRIPTION: &str = "Optional fallback identifying your client application: claude, codex, cursor, opencode, antigravity, vscode, or zed. For another application, use its short product name. Put your task name in name_session, not here. BrowserOS prefers recognized MCP client metadata for the visible prefix; this fallback is used when that metadata does not identify a known product.";

/// Owns one MCP transport lifetime. Drop best-effort schedules removal of a started
/// server session, which records its end and begins retained-group handling.
pub struct HostMcpService {
    host: Arc<HostCtx>,
    catalog: Arc<Vec<ToolDef>>,
    /// Built once per service from the host's branding.
    session_arg_description: String,
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
    /// Whether this connection has already been told its session went away.
    ///
    /// A legacy connection is identified by its transport session, so unlike a stateless
    /// caller it has no handle it can drop. Refusing it once honours the user's Stop and
    /// gives the agent something to report; refusing it forever wedges the connection until
    /// the client reconnects, which is not a recovery path any agent knows to take.
    stop_reported: bool,
}

#[derive(Clone)]
struct StartedSession {
    session: Arc<Session>,
    agent_label: String,
}

/// The two names have different owners: the declared identity keeps existing
/// profile/conversation attribution, while protocol metadata describes the client
/// product. Resolve them once when minting so later calls cannot relabel a session.
struct SessionClient {
    identity: ClientInfo,
    reported: ClientInfo,
    declared_product: Option<&'static str>,
}

impl SessionClient {
    fn resolve(reported: Option<ClientInfo>, declared: Option<ClientInfo>) -> Self {
        let declared_product = declared
            .as_ref()
            .and_then(|client| crate::identity::client_product_prefix(&client.name));
        let identity = declared
            .or_else(|| reported.clone())
            .unwrap_or_else(default_agent_client_info);
        let reported = reported.unwrap_or_else(|| identity.clone());
        Self {
            identity,
            reported,
            declared_product,
        }
    }
}

impl HostMcpService {
    #[must_use]
    pub fn new(host: Arc<HostCtx>) -> Self {
        Self {
            session_arg_description: session_arg_description(&host.branding.session_meta_key),
            host,
            catalog: Arc::new(catalog()),
            output_files: browseros_mcp::output_file::create_browser_output_file_access(),
            lifecycle: Arc::new(Mutex::new(ServiceLifecycle::default())),
            fallback_session_id: SessionId::new(format!("stdio-{}", Ulid::new())),
            closed: AtomicBool::new(false),
        }
    }

    fn native(&self) -> browseros_core::NativeCapabilities {
        self.host.native()
    }

    fn find_tool_index(&self, name: &str) -> Option<usize> {
        self.catalog.iter().position(|tool| tool.name == name)
    }

    fn find_local_tool(&self, name: &str) -> Option<&Arc<dyn LocalTool>> {
        self.host
            .local_tools
            .iter()
            .find(|tool| tool.definition().name == name)
    }

    fn with_session_arg(&self, tool: Tool) -> Tool {
        with_session_arg(tool, &self.session_arg_description)
    }

    /// The tools `tools/list` returns: available catalog tools, then listed local tools,
    /// each decorated with the optional `session` argument.
    ///
    /// `declare_agent_name` gates the optional `agentName` fallback to clients on
    /// 2026-07-28 or later. Older peers negotiated `initialize`, still carry their
    /// identity in the handshake, and must keep receiving the schema unchanged.
    #[must_use]
    pub fn listed_tools(&self, declare_agent_name: bool) -> Vec<Tool> {
        let decorate = |tool: Tool| {
            let tool = self.with_session_arg(tool);
            if declare_agent_name {
                with_agent_name_arg(tool)
            } else {
                tool
            }
        };
        // Tools whose native feature the connected browser lacks are not offered. The list is
        // fixed for the process (the identity is), and execute_tool refuses cached callers.
        let native = self.native();
        let mut tools = self
            .catalog
            .iter()
            .filter(|tool| tool.available(native))
            .map(|tool| tool.to_mcp_tool(native))
            .map(&decorate)
            .collect::<Vec<_>>();
        tools.extend(
            self.host
                .local_tools
                .iter()
                .filter(|tool| tool.listed())
                .map(|tool| decorate(tool.definition())),
        );
        tools
    }

    async fn set_client_info(&self, request: &InitializeRequestParams) {
        let mut lifecycle = self.lifecycle.lock().await;
        lifecycle.client_info = Some(client_info_from_implementation(&request.client_info));
    }

    /// Looks up an existing store session for `session_id` or mints one under it.
    /// Does not touch `self.lifecycle`, so a caller that must stay out of the
    /// transport-close cleanup can start a session without arming that teardown.
    async fn start_session_in_store(
        &self,
        session_id: SessionId,
        client: SessionClient,
    ) -> Result<StartedSession, McpError> {
        let session = if let Some(session) = self.host.sessions.lookup(&session_id).await {
            session
        } else {
            let profiles = self.host.profiles.profiles().await.map_err(|error| {
                McpError::internal_error(format!("agent profile lookup failed: {error}"), None)
            })?;
            let agent = ClientIdentity::resolve(&client.identity, &profiles);
            let session = self
                .host
                .sessions
                .mint_with_id(
                    session_id.clone(),
                    agent,
                    client.reported.clone(),
                    client.declared_product,
                )
                .await
                .map_err(|error| {
                    McpError::internal_error(format!("mcp session start failed: {error}"), None)
                })?;
            tracing::info!(
                session_id = %session.id(),
                agent = %session.convo_id(),
                "mcp session initialized"
            );
            session
        };
        Ok(started_session_from(session, &client.identity))
    }

    /// Legacy and stdio path. Caches the session id and start flag in
    /// `self.lifecycle`, which the transport-close `Drop` uses to reap the session.
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

        if lifecycle.started {
            if let Some(session) = self.host.sessions.lookup(&session_id).await
                && !self.host.targets.is_stopped(&session_id).await
            {
                return Ok(started_session_from(session, &client));
            }
            let stopped = self.host.targets.is_stopped(&session_id).await
                || self.host.sessions.retirement_of(&session_id).await
                    == Some(RetirementCause::Cancelled);
            if !stopped {
                // An idle/transport successor is still the same task. Inherit before
                // admission, even if native restart will make its saved handle stale.
                let (started, replacement) = self
                    .succeed_retired_session(
                        &session_id,
                        SessionClient::resolve(Some(client), None),
                    )
                    .await?;
                lifecycle.session_id = Some(replacement);
                return Ok(started);
            }
            // The session this connection was working in was stopped. Say so once,
            // so the stop is honoured and the agent has something to
            // tell the user, then let the next call start a new session on the same
            // connection. Refusing every time leaves the agent with nowhere to go, since the
            // handle it would otherwise drop is the transport session itself.
            if !lifecycle.stop_reported {
                lifecycle.stop_reported = true;
                return Err(McpError::invalid_request(
                    format!(
                        "{} session {session_id} was stopped and will not resume. \
                         Call again and a new session will start.",
                        self.host.branding.product_name
                    ),
                    None,
                ));
            }
            // A new session rather than the old id revived, so the work the user stopped
            // stays stopped and closed in the audit instead of gaining a second life.
            let replacement = SessionId::new(Uuid::new_v4().to_string());
            let started = self
                .start_session_in_store(
                    replacement.clone(),
                    SessionClient::resolve(Some(client), None),
                )
                .await?;
            lifecycle.session_id = Some(replacement);
            lifecycle.stop_reported = false;
            return Ok(started);
        }

        if self.host.targets.is_stopped(&session_id).await {
            // The transport handle survived a Claw restart but the task was stopped.
            // Report Stop once, then permit a separately minted, uncorrelated task.
            lifecycle.started = true;
            lifecycle.stop_reported = true;
            return Err(session_was_stopped(&self.host));
        }

        let started = self
            .start_session_in_store(session_id, SessionClient::resolve(Some(client), None))
            .await?;
        lifecycle.started = true;
        Ok(started)
    }

    /// Modern stateless path. Reuses a live server-minted handle or resolves its
    /// canonical successor. Unrecognized handles get server-generated successors,
    /// so a caller cannot choose or seed a session id and concurrent calls
    /// never mint the same id. Does not touch `self.lifecycle`, so the per-request
    /// service `Drop` never reaps it; idle sweeping owns cleanup.
    ///
    /// A handle has three fates, not two, and conflating the last two is what produced
    /// bursts of one-call sessions after a cockpit Stop. A live handle is reused. A handle
    /// the user stopped refuses, because the right answer to "stop" is not to quietly
    /// start again elsewhere. A handle that ended on its own redirects to a single
    /// remembered successor, because nobody asked for it to end and failing would be
    /// hostile. Persistent intent/aliases survive expiration of live session history;
    /// only an uncorrelated task starts unbound.
    ///
    /// The redirect exists because the fix cannot rely on the agent adopting the handle it
    /// is handed back: the server already returns a fresh handle on every call, and agents
    /// have been observed resending the dead one anyway.
    async fn resolve_modern_session(
        &self,
        provided: Option<SessionId>,
        declared: Option<ClientInfo>,
        client: Option<ClientInfo>,
    ) -> Result<(StartedSession, SessionId), McpError> {
        let client = SessionClient::resolve(client, declared);
        if let Some(handle) = &provided {
            if self.host.targets.is_stopped(handle).await {
                return Err(session_was_stopped(&self.host));
            }
            // Persisted aliases remain authoritative after the in-memory retirement
            // table expires. A rebind through a successor is visible to old handles.
            if let Some(current) = self.host.targets.current_session(handle).await
                && let Some(session) = self.host.sessions.lookup(&current).await
            {
                let agent_label = session.agent().label().to_string();
                return Ok((
                    StartedSession {
                        session,
                        agent_label,
                    },
                    current,
                ));
            }
        }
        if let Some(handle) = provided.clone()
            && let Some(session) = self.host.sessions.lookup(&handle).await
        {
            // A reused session keeps the identity it was minted with. This request's
            // inline clientInfo is optional and may be absent or differ, so relabeling
            // from it would flip the same session's audit attribution between the real
            // client and "agent" across dispatches; take the label from the session.
            let agent_label = session.agent().label().to_string();
            return Ok((
                StartedSession {
                    session,
                    agent_label,
                },
                handle,
            ));
        }
        let Some(handle) = provided else {
            // No handle at all is a request to start something new, and the only honest
            // reading of it. Nothing here guesses which session a caller "meant".
            return self.mint_session(client).await;
        };
        if matches!(
            self.host.sessions.retirement_of(&handle).await,
            Some(RetirementCause::Cancelled)
        ) {
            return Err(session_was_stopped(&self.host));
        }
        // Everything else resolves to one stable substitute: a handle whose session ended
        // on its own, and equally a handle this server has no memory of. The second case is
        // the common one and used to be the worst: live maps are in memory, so a restart
        // removes every handle its agents are holding from those maps, and an agent that
        // keeps presenting one gets a new session, and a new tab, on every call.
        self.succeed_retired_session(&handle, client).await
    }

    /// Resolves a handle that does not name a live session to one substitute, shared by
    /// every later call that still presents that handle.
    ///
    /// Mints first and agrees second. Choosing an id and minting it afterwards makes only
    /// the *choice* atomic: concurrent callers settle on one id, all find it absent, and
    /// each then builds a session of its own under it. One insert wins the map and the
    /// rest are orphans, live in their caller's hands, claiming tabs under a conversation
    /// id nothing will ever tear down, each with its own session-start row and no end.
    /// Minting first means every caller owns a real, reapable session before any of them
    /// agree on which one survives, so the loser has something it can cleanly discard.
    async fn succeed_retired_session(
        &self,
        retired: &SessionId,
        client: SessionClient,
    ) -> Result<(StartedSession, SessionId), McpError> {
        // Settled already: take the successor without minting one to throw away. The race
        // happens at most once per dead handle; every resend after it lands here.
        if let Some(existing) = self.host.sessions.replacement_of(retired).await
            && let Some(session) = self.host.sessions.lookup(&existing).await
        {
            self.inherit_target(retired, &existing).await?;
            let agent_label = session.agent().label().to_string();
            return Ok((
                StartedSession {
                    session,
                    agent_label,
                },
                existing,
            ));
        }
        let (mine, mine_handle) = self.mint_session(client).await?;
        let winner = self
            .host
            .sessions
            .adopt_replacement(retired, mine_handle.clone())
            .await;
        if winner == mine_handle {
            self.inherit_target(retired, &mine_handle).await?;
            return Ok((mine, mine_handle));
        }
        // The winner inserted its session before it adopted, and adoption is serialized, so
        // a winner that is still live is visible here. One that is not was torn down in the
        // gap; keep mine rather than chase a chain of tombstones.
        let Some(session) = self.host.sessions.lookup(&winner).await else {
            self.inherit_target(retired, &mine_handle).await?;
            return Ok((mine, mine_handle));
        };
        if let Err(error) = self
            .host
            .sessions
            .remove(&mine_handle, "closed", Some("superseded"))
            .await
        {
            warn!(error = %error, "discarding a superseded replacement session failed");
        }
        let agent_label = session.agent().label().to_string();
        self.inherit_target(retired, &winner).await?;
        Ok((
            StartedSession {
                session,
                agent_label,
            },
            winner,
        ))
    }

    async fn inherit_target(&self, from: &SessionId, to: &SessionId) -> Result<(), McpError> {
        self.host.targets.inherit(from, to).await.map_err(|error| {
            McpError::invalid_request(format!("task target recovery failed: {error}"), None)
        })?;
        Ok(())
    }

    async fn mint_session(
        &self,
        client: SessionClient,
    ) -> Result<(StartedSession, SessionId), McpError> {
        let handle = SessionId::new(Uuid::new_v4().to_string());
        let started = self.start_session_in_store(handle.clone(), client).await?;
        Ok((started, handle))
    }

    /// The legacy admission path (`ensure_session_started`), for a host's tests that
    /// drive its own local tools against a real session. Returns the session and the
    /// agent label its dispatches are attributed to.
    #[doc(hidden)]
    pub async fn ensure_session_started_for_testing(
        &self,
        session_id: SessionId,
    ) -> Result<(Arc<Session>, String), McpError> {
        self.ensure_session_started(session_id)
            .await
            .map(|started| (started.session, started.agent_label))
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
        // Only a connection that actually has a transport session gets one here.
        //
        // This used to fall back to `fallback_session_id`, a fresh `stdio-{Ulid}` minted
        // per service instance. A stateless client sending `notifications/initialized`
        // therefore created a brand-new agent session on every connection, and since
        // such clients often connect several times to list tools and then leave, the
        // audit and the cockpit filled with sessions that never dispatched anything.
        // The observed case was one client producing six empty sessions in a second.
        //
        // A handshake without a transport session is not a unit of work. If such a
        // client goes on to call a tool, `call_tool` mints the session then, which is
        // the first moment there is anything to record.
        let Some(session_id) = session_id_from_extensions(&context.extensions) else {
            return;
        };
        if let Err(error) = self.ensure_session_started(session_id).await {
            warn!(error = %error, "mcp session start failed");
        }
    }
}

impl Drop for HostMcpService {
    fn drop(&mut self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let host = self.host.clone();
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
            if let Err(error) = host
                .sessions
                .remove(&session_id, "closed", Some("transport closed"))
                .await
            {
                warn!(error = %error, session_id = %session_id, "mcp session close failed");
            }
        });
    }
}

// The server serves the modern stateless revision alongside the legacy revisions,
// so 2026-07-28 clients get the sessionless model while older clients keep the
// session model. rmcp picks per request from what a client negotiates.
const SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] = &[
    ProtocolVersion::V_2026_07_28,
    ProtocolVersion::V_2025_11_25,
    ProtocolVersion::V_2025_06_18,
    ProtocolVersion::V_2025_03_26,
    ProtocolVersion::V_2024_11_05,
];

impl ServerHandler for HostMcpService {
    fn get_info(&self) -> InitializeResult {
        let capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_tool_list_changed()
            .build();
        let branding = &self.host.branding;
        let mut implementation = Implementation::new(
            branding.server_name.clone(),
            branding.server_version.clone(),
        );
        implementation.title = Some(branding.server_title.clone());
        InitializeResult::new(capabilities)
            .with_server_info(implementation)
            .with_instructions(branding.instructions(self.native()))
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(SUPPORTED_PROTOCOL_VERSIONS)
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
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        // Two things hang off this one revision check. SEP-2549: 2026-07-28 clients
        // require ttlMs + cacheScope on list results, emitted only for that revision so
        // legacy peers keep the old wire shape (mirrors rmcp's #[tool_handler] macro);
        // ttl 0 = do-not-cache, and the tool catalog is identical across users so the
        // scope is public. Separately, 2026-07-28 dropped `initialize`, so only those
        // clients get the optional `agentName` fallback; legacy peers still carry their
        // identity in the handshake and must see the schema unchanged.
        let is_modern_revision = context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28);
        let mut result = ListToolsResult::with_all_items(self.listed_tools(is_modern_revision));
        if is_modern_revision {
            result = result.with_ttl_ms(0).with_cache_scope(CacheScope::Public);
        }
        std::future::ready(Ok(result))
    }

    // Accept the 2026-07-28 `subscriptions/listen` stream instead of returning
    // method-not-found. rmcp's default `accepted_subscription_filter` returns None,
    // which becomes JSON-RPC -32601 and an HTTP 404 that kills the transport before
    // tools/list can run. Returning the capability-supported subset opens the stream;
    // the default `listen` holds it until the client cancels or the server shuts down.
    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(requested.supported_by(&self.get_info().capabilities))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        // Local tools resolve even while unlisted: rmcp only reads this schema to validate
        // SEP-2243 headers, and none of them declares one.
        if let Some(tool) = self.find_local_tool(name) {
            return Some(self.with_session_arg(tool.definition()));
        }
        // Catalog-wide even for hidden tools: rmcp caches these input schemas by name, and a
        // cached caller should reach execute_tool's explanation, not method-not-found.
        self.find_tool_index(name)
            .map(|index| self.with_session_arg(self.catalog[index].to_mcp_tool(self.native())))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        // Local tools win a name clash with the catalog, as they always have.
        let local_tool = self.find_local_tool(&request.name).cloned();
        let tool_index = self.find_tool_index(&request.name);
        if local_tool.is_none() && tool_index.is_none() {
            return Err(McpError::method_not_found::<CallToolRequestMethod>());
        }
        let mut raw_args = request
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(JsonObject::new()));
        let provided_handle = raw_args
            .get("session")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(SessionId::new);
        // Stripped on every path, including legacy, so no tool ever receives it as an
        // argument even if an agent sends it against a schema that never offered it.
        let declared_agent_name = raw_args
            .get(AGENT_NAME_ARG)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Value::Object(map) = &mut raw_args {
            map.remove("session");
            map.remove(AGENT_NAME_ARG);
        }
        let presented_handle = provided_handle.clone();
        let modern = protocol_version_from_extensions(&context.extensions)
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28)
            && session_id_from_extensions(&context.extensions).is_none();
        let (started, session_handle) = if modern {
            // rmcp extracts wire _meta into RequestContext during decoding;
            // typed request.meta is empty even when the client supplied it.
            let modern_client = context
                .meta
                .client_info()
                .as_ref()
                .map(client_info_from_implementation);
            let declared = declared_agent_name
                .as_deref()
                .map(client_info_from_declared_name);
            let (started, handle) = self
                .resolve_modern_session(provided_handle, declared, modern_client)
                .await?;
            (started, Some(handle))
        } else {
            (self.learn_session_from_request(&context).await?, None)
        };
        if let Some(group) = raw_args
            .get("groupId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|group| !group.is_empty())
            && request.name == "tabs"
            && raw_args.get("action").and_then(Value::as_str) == Some("new")
        {
            // Legacy reconnects already carry groupId. Recover its task intent before
            // capture and mutation; a late grouping effect is too late to route safely.
            // Match the effect's trimming so it cannot adopt a known group only after
            // an uncorrelated, globally routed tab has already been created.
            self.host
                .targets
                .correlate_group(started.session.id(), group)
                .await
                .map_err(|error| McpError::invalid_request(error.to_string(), None))?;
        }
        started.session.touch(tokio::time::Instant::now()).await;
        started.session.mark_used();
        let concurrent_used_sessions = self.host.sessions.used_count().await.max(1);
        let tool_started_at = tokio::time::Instant::now();
        let tool_name = request.name.to_string();

        let result = if let Some(local_tool) = local_tool {
            Ok(local_tool
                .call(LocalToolCall {
                    host: &self.host,
                    session: &started.session,
                    agent_label: &started.agent_label,
                    args: &raw_args,
                    request_cancel: context.ct.clone(),
                })
                .await)
        } else {
            let Some(tool_index) = tool_index else {
                unreachable!("catalog tool was validated before session resolution");
            };
            let browser_session = self.host.browser.session().await;
            let binding = self
                .host
                .targets
                .binding_for_execution(started.session.id())
                .await
                .map_err(|error| McpError::invalid_request(error.to_string(), None))?;
            // Profile management must not load the Default profile as a side effect of admission.
            let manages_target = self.catalog[tool_index].metadata.manages_target;
            let view = match &browser_session {
                Some(browser) if manages_target => {
                    Some(BrowserView::capture_for_management(browser.clone(), binding, None).await)
                }
                Some(browser) => Some(BrowserView::capture(browser.clone(), binding, None).await),
                None => None,
            };
            let target_store: Arc<dyn TargetStore> = Arc::new(TaskTargetStore {
                bindings: self.host.targets.clone(),
                session: started.session.id().clone(),
            });
            let ownership_key = started.session.convo_id().clone();
            let ownership = self.host.sessions.ownership();
            let default_tab_group_id = match view.as_ref().map(BrowserView::scope) {
                Some(Ok(Some(scope))) => ownership
                    .group_for_capture(
                        &ownership_key,
                        &scope.binding.intent.browser_profile_id,
                        scope.placement_window,
                        scope.binding.revision,
                    )
                    .await
                    .and_then(|group| group.group_ref),
                Some(Err(_)) => None,
                _ => ownership.tab_group_ref(&ownership_key).await,
            };
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
                view,
                target_store,
                cancel,
                context.ct.clone(),
                dispatch_cancel,
                default_tab_group_id,
                self.host.clone(),
                self.output_files.clone(),
            );
            dispatch_tool_call(call).await
        };

        let finished = finish_tool_call(
            started.session.as_ref(),
            &tool_name,
            tool_started_at,
            concurrent_used_sessions,
            result,
        )
        .await;
        attach_session_handle(
            &self.host.branding,
            finished,
            session_handle,
            presented_handle,
        )
        .map(Into::into)
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

fn clean_client_field(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Maps an MCP `Implementation` (from `initialize` clientInfo or a stateless
/// request's inline `_meta` clientInfo) into the session `ClientInfo`, so both
/// paths derive the same agent name, slug, and tab-group prefix for a client.
fn client_info_from_implementation(source: &Implementation) -> ClientInfo {
    ClientInfo {
        name: clean_client_field(&source.name, "agent"),
        version: clean_client_field(&source.version, "unknown"),
        title: source
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    }
}

/// Builds the session identity from an agent's self-declared `agentName`. Version is
/// unknown by construction: the agent states who it is, not which build it is.
fn client_info_from_declared_name(name: &str) -> ClientInfo {
    ClientInfo {
        name: clean_client_field(name, "agent"),
        version: "unknown".to_string(),
        title: None,
    }
}

/// The anonymous fallback identity for a stateless client that sends no clientInfo.
fn default_agent_client_info() -> ClientInfo {
    ClientInfo {
        name: "agent".to_string(),
        version: "unknown".to_string(),
        title: None,
    }
}

fn started_session_from(session: Arc<Session>, client: &ClientInfo) -> StartedSession {
    let agent_label = client
        .title
        .as_deref()
        .filter(|value| !value.is_empty())
        .or_else(|| (!client.name.is_empty()).then_some(client.name.as_str()))
        .unwrap_or_else(|| session.agent().slug())
        .to_string();
    StartedSession {
        session,
        agent_label,
    }
}

fn with_session_arg(mut tool: Tool, description: &str) -> Tool {
    let mut schema = tool.input_schema.as_ref().clone();
    let properties = schema
        .entry("properties")
        .or_insert_with(|| Value::Object(JsonObject::new()));
    if let Value::Object(properties) = properties {
        properties.insert(
            "session".to_string(),
            json!({ "type": "string", "description": description }),
        );
    }
    tool.input_schema = Arc::new(schema);
    tool
}

/// Offers a product-name fallback to modern clients in one consistent schema.
/// Metadata is optional per request, so tools/list never caches a caller identity
/// or changes the schema based on whether that request happened to identify it.
fn with_agent_name_arg(mut tool: Tool) -> Tool {
    let mut schema = tool.input_schema.as_ref().clone();
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        properties.insert(
            AGENT_NAME_ARG.to_string(),
            json!({ "type": "string", "description": AGENT_NAME_ARG_DESCRIPTION }),
        );
    }
    tool.input_schema = Arc::new(schema);
    tool
}

/// The one answer an agent cannot ignore by accident.
///
/// Stop is an instruction from the user, so this refuses rather than quietly continuing the
/// work somewhere else. It names the exact next move, because an error the agent cannot act
/// on would just become a different way to produce a burst.
fn session_was_stopped(host: &HostCtx) -> McpError {
    McpError::invalid_request(
        format!(
            "This browser session was stopped from the {} cockpit and will not \
             resume. Omit the `session` argument on your next call to start a new one, and do \
             not resend this handle.",
            host.branding.product_name
        ),
        None,
    )
}

fn attach_session_handle(
    branding: &HostBranding,
    result: Result<CallToolResult, McpError>,
    handle: Option<SessionId>,
    presented: Option<SessionId>,
) -> Result<CallToolResult, McpError> {
    let Some(handle) = handle else {
        return result;
    };
    // An agent that presented a handle and got a different one back has changed session
    // without asking to. Saying so is the difference between an agent that adopts the new
    // handle and one that keeps resending a dead one, which is what fills the cockpit with
    // one-call sessions.
    let replaced = presented.is_some_and(|presented| presented != handle);
    let handle = handle.to_string();
    result.map(|mut call_result| {
        // The stateless handle is transport identity, not tool output, so it rides in
        // `_meta` (never colliding with a tool's output_schema). But MCP clients do not
        // surface result `_meta` to the model, so also append it as a content line the
        // model always sees, otherwise the agent never learns the handle to echo back
        // and loses tab ownership across stateless calls.
        call_result
            .meta
            .get_or_insert_with(MetaObject::new)
            .insert(
                branding.session_meta_key.clone(),
                Value::String(handle.clone()),
            );
        let server = &branding.server_name;
        let line = if replaced {
            format!(
                "[{server} session: {handle}. The handle you sent is no longer active, so this call continued in a new session. Pass this exact value as the `session` argument from now on, and stop sending the previous one.]"
            )
        } else {
            format!(
                "[{server} session: {handle}. Pass this exact value as the `session` argument on every following call to keep this browser session and its tab ownership.]"
            )
        };
        call_result
            .content
            .push(rmcp::model::ContentBlock::text(line));
        call_result
    })
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

fn protocol_version_from_extensions(
    extensions: &rmcp::model::Extensions,
) -> Option<ProtocolVersion> {
    extensions
        .get::<axum::http::request::Parts>()
        .and_then(|parts| parts.headers.get("mcp-protocol-version"))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(protocol_version_from_str)
}

fn protocol_version_from_str(value: &str) -> Option<ProtocolVersion> {
    match value {
        "2026-07-28" => Some(ProtocolVersion::V_2026_07_28),
        "2025-11-25" => Some(ProtocolVersion::V_2025_11_25),
        "2025-06-18" => Some(ProtocolVersion::V_2025_06_18),
        "2025-03-26" => Some(ProtocolVersion::V_2025_03_26),
        "2024-11-05" => Some(ProtocolVersion::V_2024_11_05),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        identity::ConversationIdentity, mcp::naming::session_group_title, seams::LocalTool,
    };
    use rmcp::handler::server::ServerHandler;
    use serde_json::json;

    #[tokio::test]
    async fn targeted_successors_and_old_aliases_keep_latest_intent_after_restart()
    -> anyhow::Result<()> {
        use crate::test_support::{ProfileConnection, tool_call};
        let call = tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host.clone());
        let (_, old) = service
            .resolve_modern_session(None, None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let eng = call
            .host
            .targets
            .set(&old, ProfileConnection::binding("eng", 1, 1).intent)
            .await?;
        call.host
            .sessions
            .remove(&old, "closed", Some("transport closed"))
            .await?;
        let (_, successor) = service
            .resolve_modern_session(Some(old.clone()), None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(call.host.targets.get(&successor).await, Some(eng));
        let ops = call
            .host
            .targets
            .set(&successor, ProfileConnection::binding("ops", 2, 2).intent)
            .await?;
        let (_, old_alias_result) = service
            .resolve_modern_session(Some(old.clone()), None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(old_alias_result, successor);
        assert_eq!(call.host.targets.get(&old).await, Some(ops.clone()));
        call.host.targets.sweep(i64::MAX).await?;
        let restarted = crate::test_support::host_at(call.host.helpers_root()).await?;
        let service = HostMcpService::new(restarted.clone());
        let (_, after_restart) = service
            .resolve_modern_session(Some(old.clone()), None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_ne!(after_restart, old);
        assert_eq!(restarted.targets.get(&after_restart).await, Some(ops));
        Ok(())
    }

    #[tokio::test]
    async fn targeted_stop_refuses_every_alias_after_restart() -> anyhow::Result<()> {
        use crate::test_support::{ProfileConnection, tool_call};
        let call = tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host.clone());
        let (_, old) = service
            .resolve_modern_session(None, None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        call.host
            .targets
            .set(&old, ProfileConnection::binding("eng", 1, 1).intent)
            .await?;
        call.host
            .sessions
            .remove(&old, "closed", Some("idle timeout"))
            .await?;
        let (_, successor) = service
            .resolve_modern_session(Some(old.clone()), None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        call.host.sessions.cancel_by_session(&successor).await?;
        let restarted = crate::test_support::host_at(call.host.helpers_root()).await?;
        let service = HostMcpService::new(restarted.clone());
        for handle in [old, successor] {
            let error = service
                .resolve_modern_session(Some(handle), None, None)
                .await
                .err()
                .ok_or_else(|| anyhow::anyhow!("stopped task resumed"))?;
            assert!(error.message.contains("was stopped"), "{error:?}");
        }
        assert_eq!(restarted.sessions.count().await, 0);
        Ok(())
    }

    #[tokio::test]
    async fn legacy_idle_successor_inherits_target_but_stop_starts_a_separate_task()
    -> anyhow::Result<()> {
        use crate::test_support::{ProfileConnection, tool_call};
        let call = tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host.clone());
        let transport = SessionId::new("synthetic-legacy-transport");
        let first = service
            .ensure_session_started(transport.clone())
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let selected = call
            .host
            .targets
            .set(
                first.session.id(),
                ProfileConnection::binding("eng", 1, 1).intent,
            )
            .await?;
        call.host
            .sessions
            .remove(first.session.id(), "closed", Some("idle timeout"))
            .await?;
        let next = service
            .ensure_session_started(transport.clone())
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_ne!(first.session.id(), next.session.id());
        assert_eq!(
            call.host.targets.get(next.session.id()).await,
            Some(selected)
        );
        call.host
            .sessions
            .cancel_by_session(next.session.id())
            .await?;
        assert!(
            service
                .ensure_session_started(transport.clone())
                .await
                .is_err()
        );
        let new_task = service
            .ensure_session_started(transport)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_ne!(new_task.session.id(), next.session.id());
        assert_eq!(call.host.targets.get(new_task.session.id()).await, None);
        assert!(call.host.targets.is_stopped(first.session.id()).await);
        Ok(())
    }

    #[test]
    fn supported_protocol_versions_includes_modern_and_legacy() {
        assert!(SUPPORTED_PROTOCOL_VERSIONS.contains(&ProtocolVersion::V_2026_07_28));
        assert!(SUPPORTED_PROTOCOL_VERSIONS.contains(&ProtocolVersion::V_2025_11_25));
    }

    #[tokio::test]
    async fn subscriptions_listen_is_accepted_not_method_not_found() -> anyhow::Result<()> {
        // rmcp turns a None subscription filter into JSON-RPC -32601 for subscriptions/listen,
        // which its streamable-http transport maps to a 404 that kills the connection. Accepting
        // the capability-supported subset opens the stream instead.
        let call = crate::test_support::tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host);
        let requested = SubscriptionFilter::builder().tools_list_changed().build();
        let accepted = service.accepted_subscription_filter(&requested);
        assert_eq!(
            accepted.and_then(|filter| filter.tools_list_changed),
            Some(true),
            "subscriptions/listen must be accepted (Some), not method-not-found"
        );
        Ok(())
    }

    #[tokio::test]
    async fn with_session_arg_adds_an_optional_session_property_to_every_tool() -> anyhow::Result<()>
    {
        let call = crate::test_support::tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host);

        let injected =
            service.with_session_arg(crate::mcp::name_session::NameSessionTool.definition());
        let schema = Value::Object(injected.input_schema.as_ref().clone());
        assert_eq!(
            schema["properties"]["session"],
            json!({ "type": "string", "description": service.session_arg_description.as_str() })
        );
        assert!(
            !schema["required"]
                .as_array()
                .is_some_and(|required| required.contains(&json!("session")))
        );
        // The promise has to stay conditional and has to keep naming the fallback:
        // an unconditional version of this text is what sent a legacy client after
        // a handle it never receives.
        assert!(
            service
                .session_arg_description
                .as_str()
                .contains("only if your client connects without"),
            "the handle must not be promised unconditionally"
        );
        assert!(
            service
                .session_arg_description
                .as_str()
                .contains("your tab group id instead"),
            "a client that gets no handle must be told what to use"
        );
        assert!(
            !service
                .session_arg_description
                .as_str()
                .contains("and its tab ownership"),
            "tab ownership continuity comes from the group id, not this handle"
        );

        for tool in service.listed_tools(false) {
            let schema = Value::Object(tool.input_schema.as_ref().clone());
            assert_eq!(
                schema["properties"]["session"]["type"],
                json!("string"),
                "tool {} is missing the session property",
                tool.name
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn modern_session_reuses_returned_handles_and_never_honors_client_ids()
    -> anyhow::Result<()> {
        let call = crate::test_support::tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host);

        let (first, minted) = service
            .resolve_modern_session(None, None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;

        let (again, reused) = service
            .resolve_modern_session(Some(minted.clone()), None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(reused.to_string(), minted.to_string());
        assert_eq!(
            again.session.id().to_string(),
            first.session.id().to_string()
        );

        let client_chosen = SessionId::new("client-picked-id");
        let (other, other_handle) = service
            .resolve_modern_session(Some(client_chosen.clone()), None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_ne!(other_handle.to_string(), client_chosen.to_string());
        assert_ne!(
            other.session.id().to_string(),
            first.session.id().to_string()
        );
        Ok(())
    }

    /// The incident, replayed. Dani presses Stop, then asks the agent another question.
    /// The agent resends the handle it was told to resend, and before this change every
    /// such call minted a session of its own, so one question became a burst.
    #[tokio::test]
    async fn a_stopped_handle_refuses_instead_of_minting() -> anyhow::Result<()> {
        let call = crate::test_support::tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host.clone());

        let (_, handle) = service
            .resolve_modern_session(None, None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        call.host.sessions.cancel_by_session(&handle).await?;
        let after_stop = call.host.sessions.count().await;

        let refused = service
            .resolve_modern_session(Some(handle.clone()), None, None)
            .await;
        let error = refused
            .err()
            .unwrap_or_else(|| panic!("a stopped session must refuse, not mint"));
        let message = format!("{error:?}");
        assert!(message.contains("stopped"), "{message}");
        assert!(message.contains("Omit the `session` argument"), "{message}");
        assert_eq!(
            call.host.sessions.count().await,
            after_stop,
            "refusing must not leave a session behind"
        );
        Ok(())
    }

    /// An agent that ignores the refusal and keeps resending the handle must not be able to
    /// produce the burst by persistence alone.
    #[tokio::test]
    async fn a_stopped_handle_refuses_every_time_it_is_presented() -> anyhow::Result<()> {
        let call = crate::test_support::tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host.clone());

        let (_, handle) = service
            .resolve_modern_session(None, None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        call.host.sessions.cancel_by_session(&handle).await?;
        let after_stop = call.host.sessions.count().await;

        for attempt in 0..10 {
            assert!(
                service
                    .resolve_modern_session(Some(handle.clone()), None, None)
                    .await
                    .is_err(),
                "attempt {attempt} was allowed through"
            );
        }
        assert_eq!(call.host.sessions.count().await, after_stop);
        Ok(())
    }

    /// Nobody asked for an idle sweep or a restart, so those redirect rather than refuse.
    /// One successor, however many times the dead handle arrives: the fix cannot depend on
    /// the agent adopting the handle it is handed back, because agents demonstrably do not.
    #[tokio::test]
    async fn a_closed_handle_yields_one_successor_however_often_it_is_resent() -> anyhow::Result<()>
    {
        let call = crate::test_support::tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host.clone());

        let (_, handle) = service
            .resolve_modern_session(None, None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        call.host
            .sessions
            .remove(&handle, "closed", Some("idle timeout"))
            .await?;
        let before = call.host.sessions.count().await;

        let mut successors = std::collections::BTreeSet::new();
        for _ in 0..10 {
            let (_, resolved) = service
                .resolve_modern_session(Some(handle.clone()), None, None)
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            assert_ne!(resolved.to_string(), handle.to_string());
            successors.insert(resolved.to_string());
        }
        assert_eq!(successors.len(), 1, "ten calls produced {successors:?}");
        assert_eq!(
            call.host.sessions.count().await,
            before + 1,
            "ten calls must leave exactly one new session"
        );
        Ok(())
    }

    /// Two calls arriving together on one dead handle must converge, or the burst simply
    /// moves from sequential to concurrent.
    #[tokio::test]
    async fn concurrent_calls_on_one_closed_handle_converge() -> anyhow::Result<()> {
        let call = crate::test_support::tool_call("tabs", json!({})).await?;
        let service = std::sync::Arc::new(HostMcpService::new(call.host.clone()));

        let (_, handle) = service
            .resolve_modern_session(None, None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        call.host
            .sessions
            .remove(&handle, "closed", Some("transport closed"))
            .await?;
        let before = call.host.sessions.count().await;

        let racers = (0..4).map(|_| {
            let service = service.clone();
            let handle = handle.clone();
            tokio::spawn(async move {
                service
                    .resolve_modern_session(Some(handle), None, None)
                    .await
                    .map(|(started, resolved)| {
                        (
                            resolved.to_string(),
                            started.session.convo_id().as_str().to_string(),
                        )
                    })
                    .map_err(|error| anyhow::anyhow!("{error:?}"))
            })
        });
        let mut resolved = std::collections::BTreeSet::new();
        for racer in racers.collect::<Vec<_>>() {
            resolved.insert(racer.await??);
        }
        // Agreeing on the handle is not enough. Two callers can name the same successor and
        // still each build a session of their own under it, and the one that loses the map
        // insert stays live in its caller's hands while nothing can ever tear it down. The
        // conversation identity is what tells those sessions apart, so it is what the
        // assertion has to compare.
        assert_eq!(
            resolved.len(),
            1,
            "concurrent calls split into {resolved:?}"
        );
        let (winner, convo) = resolved
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("no successor"));
        let live = call
            .host
            .sessions
            .lookup(&SessionId::new(winner))
            .await
            .unwrap_or_else(|| panic!("the successor every caller returned is not in the store"));
        assert_eq!(
            live.convo_id().as_str(),
            convo,
            "callers were handed a session the store does not hold"
        );
        assert_eq!(call.host.sessions.count().await, before + 1);
        Ok(())
    }

    /// Taken from a real trace. A server restart empties both maps, so every handle its
    /// agents are holding becomes unrecognised at once. The agent was not at fault: it
    /// presented the same handle on seven consecutive calls, was told each time that the
    /// handle was no longer active, and was handed seven different sessions and seven tabs.
    ///
    /// The rule that fixes it is that the same handle always resolves to the same session,
    /// whatever the server remembers about it.
    #[tokio::test]
    async fn a_handle_this_server_never_minted_still_resolves_to_one_session() -> anyhow::Result<()>
    {
        let call = crate::test_support::tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host.clone());
        let before = call.host.sessions.count().await;

        // A handle from before a restart, which this server has no record of at all.
        let stranger = SessionId::new("b0b6200a-832b-49c8-bfd8-386e0a7f267d");
        let mut resolved = std::collections::BTreeSet::new();
        for _ in 0..7 {
            let (_, handle) = service
                .resolve_modern_session(Some(stranger.clone()), None, None)
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            // The presented handle is a map key, never an identity a caller can seed.
            assert_ne!(handle.to_string(), stranger.to_string());
            resolved.insert(handle.to_string());
        }
        assert_eq!(
            resolved.len(),
            1,
            "seven calls with one handle produced {resolved:?}"
        );
        assert_eq!(
            call.host.sessions.count().await,
            before + 1,
            "seven calls must leave one session, not seven"
        );
        Ok(())
    }

    /// Sending no handle is the one thing that still means "start something new". Nothing
    /// guesses which session a caller meant, which is what keeps this deterministic.
    #[tokio::test]
    async fn no_handle_at_all_starts_something_new_every_time() -> anyhow::Result<()> {
        let call = crate::test_support::tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host.clone());

        let mut minted = std::collections::BTreeSet::new();
        for _ in 0..3 {
            let (_, handle) = service
                .resolve_modern_session(None, None, None)
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            minted.insert(handle.to_string());
        }
        assert_eq!(minted.len(), 3);
        Ok(())
    }

    /// The agent has to be able to see that its session changed. `_meta` is not surfaced to
    /// the model by MCP clients, so the difference has to be in the content.
    #[test]
    fn a_replaced_handle_says_so_in_the_content() -> anyhow::Result<()> {
        let replaced = attach_session_handle(
            &crate::test_support::fixture_branding(),
            Ok(CallToolResult::success(vec![
                rmcp::model::ContentBlock::text("ok"),
            ])),
            Some(SessionId::new("successor")),
            Some(SessionId::new("dead-handle")),
        )
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let text = text_of(&replaced);
        assert!(text.contains("no longer active"), "{text}");
        assert!(text.contains("stop sending the previous one"), "{text}");

        // An agent that presented the handle it got back is not told anything unusual.
        let unchanged = attach_session_handle(
            &crate::test_support::fixture_branding(),
            Ok(CallToolResult::success(vec![
                rmcp::model::ContentBlock::text("ok"),
            ])),
            Some(SessionId::new("same")),
            Some(SessionId::new("same")),
        )
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let text = text_of(&unchanged);
        assert!(!text.contains("no longer active"), "{text}");
        Ok(())
    }

    fn text_of(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|block| match block {
                rmcp::model::ContentBlock::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The legacy transport, which Codex still speaks. Its session is the transport itself,
    /// so when the user stops it there is no handle for the agent to drop: it retried four
    /// times, got the same refusal each time, and told the user to restart the browser.
    ///
    /// Say it once, so the stop is honoured and the agent can report it, then let the next
    /// call through on a new session so the connection is not wedged until the client
    /// reconnects.
    #[tokio::test]
    async fn a_legacy_connection_recovers_after_its_session_is_stopped() -> anyhow::Result<()> {
        let call = crate::test_support::tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host.clone());
        let transport = SessionId::new("mcp-transport-abc");

        let first = service
            .ensure_session_started(transport.clone())
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let stopped = first.session.id().clone();

        call.host.sessions.cancel_by_session(&stopped).await?;

        let refusal = service
            .ensure_session_started(transport.clone())
            .await
            .err()
            .unwrap_or_else(|| panic!("the stop must be reported, not swallowed"));
        let message = format!("{refusal:?}");
        assert!(message.contains("was stopped"), "{message}");
        assert!(message.contains("a new session will start"), "{message}");

        // The next call must work, or the connection is dead until Codex reconnects.
        let recovered = service
            .ensure_session_started(transport.clone())
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_ne!(
            recovered.session.id(),
            &stopped,
            "the stopped session must stay stopped, not come back to life"
        );

        // And the connection settles there rather than churning a session per call.
        let again = service
            .ensure_session_started(transport)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(again.session.id(), recovered.session.id());
        Ok(())
    }

    #[tokio::test]
    async fn modern_session_adopts_the_inline_client_name() -> anyhow::Result<()> {
        let call = crate::test_support::tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host);

        // A stateless client that sends inline clientInfo is named after it, so its
        // tabs group under the real client slug rather than the anonymous "agent".
        let client = ClientInfo {
            name: "test-harness-client".to_string(),
            version: "9.9.9".to_string(),
            title: None,
        };
        let (named, handle) = service
            .resolve_modern_session(None, None, Some(client))
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(named.session.agent().slug(), "test-harness-client");
        assert_eq!(named.agent_label, "test-harness-client");

        // Reusing that handle without clientInfo keeps the minted identity and label,
        // so a session's audit attribution never flips to "agent" mid-conversation.
        let (reused, _) = service
            .resolve_modern_session(Some(handle), None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(reused.session.agent().slug(), "test-harness-client");
        assert_eq!(reused.agent_label, "test-harness-client");

        // A stateless client that sends no clientInfo falls back to "agent".
        let (anon, _) = service
            .resolve_modern_session(None, None, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(anon.session.agent().slug(), "agent");
        Ok(())
    }

    #[test]
    fn attach_session_handle_puts_the_handle_in_meta_not_structured_content() -> anyhow::Result<()>
    {
        // The handle must ride in `_meta`, never in `structured_content`, so it cannot
        // collide with a tool's output_schema or overwrite the tool's real result.
        let modern = attach_session_handle(
            &crate::test_support::fixture_branding(),
            Ok(CallToolResult::success(vec![
                rmcp::model::ContentBlock::text("ok"),
            ])),
            Some(SessionId::new("handle-xyz")),
            None,
        )
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert!(
            modern.structured_content.is_none(),
            "handle must not touch structured_content"
        );
        let handle = modern
            .meta
            .as_ref()
            .and_then(|meta| meta.get("com.browseros.neo/session"))
            .and_then(Value::as_str);
        assert_eq!(handle, Some("handle-xyz"));
        // The handle is also appended to content so the model (which does not see _meta)
        // reads and echoes it.
        let content_has_handle = modern.content.iter().any(|block| {
            block
                .as_text()
                .is_some_and(|text| text.text.contains("handle-xyz"))
        });
        assert!(content_has_handle, "handle must also appear in content");

        // Legacy calls (no handle) get neither _meta nor structured_content touched.
        let legacy = attach_session_handle(
            &crate::test_support::fixture_branding(),
            Ok(CallToolResult::success(vec![
                rmcp::model::ContentBlock::text("ok"),
            ])),
            None,
            None,
        )
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert!(legacy.structured_content.is_none());
        assert!(legacy.meta.is_none());
        Ok(())
    }

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
    async fn agent_name_is_optional_and_only_offered_to_modern_clients() -> anyhow::Result<()> {
        let call = crate::test_support::tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host);

        // Legacy peers negotiated `initialize` and still carry identity in the
        // handshake, so their schema must not gain a required argument.
        for tool in service.listed_tools(false) {
            let schema = Value::Object(tool.input_schema.as_ref().clone());
            assert_eq!(schema["properties"][AGENT_NAME_ARG], Value::Null);
            assert!(
                !schema["required"]
                    .as_array()
                    .is_some_and(|required| required.contains(&json!(AGENT_NAME_ARG)))
            );
        }

        for tool in service.listed_tools(true) {
            let schema = Value::Object(tool.input_schema.as_ref().clone());
            assert_eq!(
                schema["properties"][AGENT_NAME_ARG]["type"],
                json!("string")
            );
            assert!(
                !schema["required"]
                    .as_array()
                    .is_some_and(|required| required.contains(&json!(AGENT_NAME_ARG))),
                "{} must be optional for {}",
                AGENT_NAME_ARG,
                tool.name
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn declared_identity_and_reported_product_remain_distinct() -> anyhow::Result<()> {
        let call = crate::test_support::tool_call("tabs", json!({})).await?;
        let service = HostMcpService::new(call.host);

        let (declared, _) = service
            .resolve_modern_session(
                None,
                Some(client_info_from_declared_name("claude-code")),
                Some(ClientInfo {
                    name: "Codex".to_string(),
                    version: "1".to_string(),
                    title: None,
                }),
            )
            .await?;
        assert_eq!(declared.session.agent().slug(), "claude-code");
        assert_eq!(declared.session.client_name(), "Codex");
        assert_eq!(session_group_title(&declared.session, "task"), "codex/task");
        assert!(
            declared
                .session
                .convo_id()
                .as_str()
                .starts_with("claude-code-")
        );

        // Falls back to inline clientInfo when the agent declares nothing, and to the
        // anonymous identity when neither is present. A missing name never errors.
        let (from_meta, _) = service
            .resolve_modern_session(
                None,
                None,
                Some(ClientInfo {
                    name: "Codex".to_string(),
                    version: "1".to_string(),
                    title: None,
                }),
            )
            .await?;
        assert_eq!(from_meta.session.agent().slug(), "codex");

        let (anonymous, _) = service.resolve_modern_session(None, None, None).await?;
        assert_eq!(anonymous.session.agent().slug(), "agent");
        Ok(())
    }
}
