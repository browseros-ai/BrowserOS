//! Runs the SDK connection actor beside the process supervisor. Handles own the
//! connection lifetime; the actor owns only shared routing/process state, avoiding
//! a reference cycle that would otherwise keep stdin and the agent alive forever.

use std::{
    collections::HashSet,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use agent_client_protocol::{
    Agent, ByteStreams, Client, ConnectionTo, Dispatch, Handled, PreparedRequest, SentRequest,
};
use tokio::sync::oneshot;
use tokio_util::{
    compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt},
    sync::CancellationToken,
};

use crate::{
    AcpError, AgentFeatures, ClientOptions, ExitReport, LaunchSpec, PermissionPolicy,
    ProtocolVersion, Session, SessionParams,
    process::{Process, lock},
    routing::Routes,
    schema::{
        self, ClientCapabilities, FileSystemCapabilities, Implementation, InitializeRequest,
        InitializeResponse, LoadSessionRequest, NewSessionRequest, RequestId,
        RequestPermissionRequest, RequestPermissionResponse, ResumeSessionRequest, SessionId,
        SessionNotification,
    },
};

/// Actor-owned state deliberately excludes the public handle, so no Arc cycle
/// can prevent the last handle from closing stdin and supervising process exit.
pub(crate) struct State {
    pub(crate) process: Process,
    pub(crate) routes: Arc<Routes>,
    pub(crate) stop: CancellationToken,
    // Caller teardown must yield Closed even if the supervisor wins the race
    // to reap. Driver failures instead preserve an already-available exit report.
    shutdown_requested: AtomicBool,
    // The SDK uses the same Error type for peer replies and local failures.
    // Record wire provenance by request ID before routing the reply; matching
    // messages or codes would misclassify a genuine agent "oneshot canceled".
    error_responses: Mutex<HashSet<RequestId>>,
}

/// Lifetime lease held only by connection handles and sessions. Prompt tasks
/// retain State instead, so an unresponsive turn cannot prevent final teardown.
pub(crate) struct Inner {
    pub(crate) cx: ConnectionTo<Agent>,
    pub(crate) state: Arc<State>,
    pub(crate) options: ClientOptions,
    init: InitializeResponse,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.state.close(Duration::from_secs(1));
    }
}

/// Cheap clone of one live ACP connection. Explicit shutdown affects every clone
/// and session; dropping the last handle also supervises EOF and eventual kill.
#[derive(Clone)]
pub struct AgentConnection {
    pub(crate) inner: Arc<Inner>,
}

impl AgentConnection {
    pub async fn connect(
        spec: LaunchSpec,
        options: ClientOptions,
        permissions: Arc<dyn PermissionPolicy>,
    ) -> Result<Self, AcpError> {
        let (process, stdin, stdout) = Process::spawn(spec, options.stderr_tail_lines)?;
        let state = Arc::new(State {
            process,
            routes: Arc::default(),
            stop: CancellationToken::new(),
            shutdown_requested: AtomicBool::new(false),
            error_responses: Mutex::default(),
        });
        // If connect is dropped during initialize, no public handle exists yet
        // to own teardown. This guard transfers that ownership only on success.
        let mut startup = ControlGuard::new(&state);
        let (ready, rx) = oneshot::channel();
        let actor_state = state.clone();
        let dispatch_state = state.clone();
        let driver_state = state.clone();
        let permission_state = state.clone();
        tokio::spawn(async move {
            let result = Client.builder()
                .on_receive_dispatch(async move |dispatch: Dispatch, _cx| {
                    match dispatch {
                        Dispatch::Notification(raw) => {
                            if raw.method == "session/update" {
                                // Parse at the raw seam: a typed SDK callback
                                // rejects unknown enum tags before fallback runs.
                                // Log the tag, never the transcript or tool input.
                                let tag = raw.params.pointer("/update/sessionUpdate").and_then(|tag| tag.as_str()).map(str::to_owned);
                                match serde_json::from_value::<SessionNotification>(raw.params) {
                                    Ok(notification) => dispatch_state.routes.update(notification),
                                    Err(error) => tracing::warn!(method = %raw.method, ?tag, error_category = ?error.classify(), "Ignoring unparsed ACP notification"),
                                }
                            } else {
                                tracing::warn!(method = %raw.method, "Ignoring unknown ACP notification");
                            }
                            Ok(Handled::Yes)
                        }
                        Dispatch::Response(result, router) => {
                            if result.is_err() {
                                lock(&dispatch_state.error_responses).insert(router.id().clone());
                            }
                            router.route_with_result(result)?;
                            Ok(Handled::Yes)
                        }
                        other => Ok(Handled::No { message: other, retry: false }),
                    }
                }, agent_client_protocol::on_receive_dispatch!())
                .on_receive_request(async move |request: RequestPermissionRequest, responder, cx| {
                    let policy = permissions.clone();
                    let active = permission_state.routes.permission_turn(&request.session_id);
                    // Some agents ask during new/load before a Turn exists.
                    // Those requests still use policy and connection teardown;
                    // turn-scoped requests additionally serialize with cancel.
                    let connection_stop = permission_state.stop.clone();
                    let stop = active.as_ref().map(|active| active.permission_stop.clone())
                        .unwrap_or_else(|| connection_stop.child_token());
                    cx.spawn(async move {
                        let decision = tokio::select! {
                            biased;
                            () = stop.cancelled() => crate::PermissionDecision::Cancel,
                            () = connection_stop.cancelled() => crate::PermissionDecision::Cancel,
                            decision = policy.decide(&request) => decision,
                        };
                        let result = match active {
                            Some(active) => active.permission_response(&request, decision, responder),
                            None => responder.respond(RequestPermissionResponse::new(crate::permissions::outcome(&request, decision))),
                        };
                        if let Err(error) = result {
                            tracing::debug!(%error, "ACP permission requester already closed");
                        }
                        Ok(())
                    })
                }, agent_client_protocol::on_receive_request!())
                .connect_with(ByteStreams::new(stdin.compat_write(), stdout.compat()), async move |cx: ConnectionTo<Agent>| {
                    let _ = ready.send(cx.clone());
                    tokio::select! {
                        () = actor_state.stop.cancelled() => {},
                        () = cx.incoming_closed() => {},
                    }
                    Ok(())
                }).await;
            if let Err(error) = result {
                // A malformed/broken transport may end dispatch without EOF.
                // Wake prompt and permission waiters even if the child lives.
                driver_state.stop.cancel();
                driver_state.process.stop(Duration::from_secs(1));
                tracing::debug!(%error, "ACP connection driver stopped");
            }
        });
        let cx = rx.await.map_err(|_| AcpError::Closed)?;
        let request = InitializeRequest::new(ProtocolVersion::V1)
            .client_capabilities(
                ClientCapabilities::new()
                    .fs(FileSystemCapabilities::new()
                        .read_text_file(false)
                        .write_text_file(false))
                    .terminal(false),
            )
            .client_info(Implementation::new(
                options.client_name.clone(),
                options.client_version.clone(),
            ));
        let init = control(
            &state,
            options.control_timeout,
            "initialize",
            state.response(&[], cx.send_request(request)),
        )
        .await?;
        if init.protocol_version != ProtocolVersion::V1 {
            return Err(AcpError::ProtocolVersion(init.protocol_version));
        }
        startup.complete = true;
        drop(startup);
        Ok(Self {
            inner: Arc::new(Inner {
                cx,
                state,
                options,
                init,
            }),
        })
    }

    pub fn initialize_response(&self) -> &InitializeResponse {
        &self.inner.init
    }
    pub fn features(&self) -> AgentFeatures {
        let caps = &self.inner.init.agent_capabilities;
        AgentFeatures {
            load_session: caps.load_session,
            resume_session: caps.session_capabilities.resume.is_some(),
            close_session: caps.session_capabilities.close.is_some(),
            mcp_http: caps.mcp_capabilities.http,
            mcp_sse: caps.mcp_capabilities.sse,
            prompt_image: caps.prompt_capabilities.image,
            prompt_audio: caps.prompt_capabilities.audio,
        }
    }
    pub fn pid(&self) -> Option<u32> {
        self.inner.state.process.pid
    }
    pub fn stderr_tail(&self) -> Vec<String> {
        self.inner.state.process.tail()
    }
    pub async fn closed(&self) -> ExitReport {
        self.inner.state.process.closed().await
    }

    /// Close the SDK's stdin writer, then allow `grace` for exit before the
    /// supervisor kills the process group. Other clones become unusable too.
    pub async fn shutdown(self, grace: Duration) -> ExitReport {
        self.inner.state.close(grace);
        self.closed().await
    }

    pub async fn new_session(&self, params: SessionParams) -> Result<Session, AcpError> {
        self.check_mcp(&params)?;
        let request = NewSessionRequest::new(params.cwd)
            .mcp_servers(params.mcp_servers)
            .meta(params.meta);
        let routes = self.inner.state.routes.clone();
        let (id, snapshot) = self
            .control_ordered(
                "session/new",
                self.inner.cx.prepare_request(request),
                move |response| {
                    let snapshot = routes.attach(
                        &response.session_id,
                        response.modes,
                        response.config_options.unwrap_or_default(),
                    );
                    (response.session_id, snapshot)
                },
            )
            .await?;
        Ok(Session::new(self.clone(), id, snapshot))
    }

    /// No turn can register while load replay is being dispatched. Replay and
    /// idle transcript updates are discarded; controls after the accepted
    /// attachment refresh the snapshot before later notifications are dispatched.
    pub async fn load_session(
        &self,
        id: SessionId,
        params: SessionParams,
    ) -> Result<Session, AcpError> {
        if !self.features().load_session {
            return Err(AcpError::Unsupported("session/load"));
        }
        self.check_mcp(&params)?;
        let mut loading = self.inner.state.routes.loading(&id)?;
        let request = LoadSessionRequest::new(id.clone(), params.cwd)
            .mcp_servers(params.mcp_servers)
            .meta(params.meta);
        let routes = self.inner.state.routes.clone();
        let session_id = id.clone();
        let snapshot = self
            .control_ordered(
                "session/load",
                self.inner.cx.prepare_request(request),
                move |response| {
                    let snapshot = routes.attach(
                        &session_id,
                        response.modes,
                        response.config_options.unwrap_or_default(),
                    );
                    loading.complete();
                    snapshot
                },
            )
            .await?;
        Ok(Session::new(self.clone(), id, snapshot))
    }

    pub async fn resume_session(
        &self,
        id: SessionId,
        params: SessionParams,
    ) -> Result<Session, AcpError> {
        if !self.features().resume_session {
            return Err(AcpError::Unsupported("session/resume"));
        }
        self.check_mcp(&params)?;
        let mut loading = self.inner.state.routes.loading(&id)?;
        let request = ResumeSessionRequest::new(id.clone(), params.cwd)
            .mcp_servers(params.mcp_servers)
            .meta(params.meta);
        let routes = self.inner.state.routes.clone();
        let session_id = id.clone();
        let snapshot = self
            .control_ordered(
                "session/resume",
                self.inner.cx.prepare_request(request),
                move |response| {
                    let snapshot = routes.attach(
                        &session_id,
                        response.modes,
                        response.config_options.unwrap_or_default(),
                    );
                    loading.complete();
                    snapshot
                },
            )
            .await?;
        Ok(Session::new(self.clone(), id, snapshot))
    }

    fn check_mcp(&self, params: &SessionParams) -> Result<(), AcpError> {
        for server in &params.mcp_servers {
            match server {
                schema::McpServer::Http(_) if !self.features().mcp_http => {
                    return Err(AcpError::Unsupported("HTTP MCP servers"));
                }
                schema::McpServer::Sse(_) if !self.features().mcp_sse => {
                    return Err(AcpError::Unsupported("SSE MCP servers"));
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn available(&self) -> Result<(), AcpError> {
        if let Some(report) = self.inner.state.process.report() {
            return Err(exited(report));
        }
        if self.inner.state.stop.is_cancelled() || self.inner.cx.is_incoming_closed() {
            return Err(AcpError::Closed);
        }
        Ok(())
    }

    pub(crate) async fn control<T>(
        &self,
        method: &'static str,
        request: SentRequest<T>,
    ) -> Result<T, AcpError> {
        self.available()?;
        control(
            &self.inner.state,
            self.inner.options.control_timeout,
            method,
            self.inner.state.response(&self.auth_methods(), request),
        )
        .await
    }

    /// Attachment/config replies establish snapshots at their wire position.
    /// Ordered callbacks complete before the next notification is dispatched;
    /// applying after an await could lose or overwrite newer control updates.
    pub(crate) async fn control_ordered<T: Send + 'static, U: Send + 'static>(
        &self,
        method: &'static str,
        request: PreparedRequest<T>,
        apply: impl FnOnce(T) -> U + Send + 'static,
    ) -> Result<U, AcpError> {
        self.available()?;
        let id = request.id().clone();
        let (tx, rx) = oneshot::channel();
        let published = request.on_receiving_result(async move |result| {
            let _ = tx.send(result.map(apply));
            Ok(())
        });
        let response = async {
            published?;
            rx.await.map_err(|_| schema::Error::internal_error())?
        };
        control(
            &self.inner.state,
            self.inner.options.control_timeout,
            method,
            self.inner.state.consume(&self.auth_methods(), id, response),
        )
        .await
    }

    pub(crate) fn auth_methods(&self) -> Vec<String> {
        self.inner
            .init
            .auth_methods
            .iter()
            .map(|method| method.id().to_string())
            .collect()
    }
}

async fn control<T>(
    state: &State,
    after: Duration,
    method: &'static str,
    future: impl Future<Output = Result<T, AcpError>>,
) -> Result<T, AcpError> {
    let mut guard = ControlGuard::new(state);
    let result = tokio::select! {
        biased;
        () = state.stop.cancelled() => Err(state.stopped_error()),
        result = tokio::time::timeout(after, future) => match result {
            Ok(result) => result,
            Err(_) => return Err(AcpError::Timeout { method, after }),
        },
    };
    guard.complete = true;
    result
}

impl State {
    pub(crate) fn close(&self, grace: Duration) {
        self.shutdown_requested.store(true, Ordering::Release);
        self.stop.cancel();
        self.process.stop(grace);
    }

    pub(crate) fn stopped_error(&self) -> AcpError {
        if self.shutdown_requested.load(Ordering::Acquire) {
            AcpError::Closed
        } else {
            self.process
                .report()
                .map(exited)
                .unwrap_or(AcpError::Closed)
        }
    }

    /// Consume a reply outside dispatch, remembering whether its error actually
    /// came from the agent. This helper holds no public connection lease.
    pub(crate) async fn response<T>(
        &self,
        auth: &[String],
        request: SentRequest<T>,
    ) -> Result<T, AcpError> {
        let id = request.id().clone();
        self.consume(auth, id, request.block_task()).await
    }

    async fn consume<T>(
        &self,
        auth: &[String],
        id: RequestId,
        response: impl Future<Output = Result<T, schema::Error>>,
    ) -> Result<T, AcpError> {
        let result = response.await;
        let agent_error = lock(&self.error_responses).remove(&id);
        match result {
            Ok(value) => Ok(value),
            Err(error) => Err(map_error(self, auth, error, agent_error).await),
        }
    }
}

async fn map_error(
    state: &State,
    auth: &[String],
    error: schema::Error,
    agent_error: bool,
) -> AcpError {
    if !agent_error {
        // stdout EOF can beat waitpid by a scheduler tick. Give the sole child
        // owner time to reap and finish the stderr tail before reporting a crash.
        if let Ok(report) =
            tokio::time::timeout(Duration::from_millis(1250), state.process.closed()).await
        {
            return exited(report);
        }
        return AcpError::Closed;
    }
    let code = i32::from(error.code);
    let message = error.message;
    let lower = message.to_lowercase();
    let explicit_auth_failure = lower.contains("failed to authenticate")
        || lower.contains("authentication required")
        || error
            .data
            .as_ref()
            .and_then(|data| data.get("errorKind"))
            .and_then(|kind| kind.as_str())
            == Some("authentication_failed");
    // Invalid params remains a protocol error even when it mentions login.
    // MCP authentication belongs to the named server, not this ACP agent.
    let mcp_error = lower.contains("mcp")
        || error
            .data
            .as_ref()
            .is_some_and(|data| data.to_string().to_lowercase().contains("mcp"));
    if code != -32602 && !mcp_error && (code == -32000 || (code == -32603 && explicit_auth_failure))
    {
        AcpError::AuthRequired {
            methods: auth.to_vec(),
            message,
        }
    } else {
        AcpError::Rpc {
            code,
            message,
            data: error.data,
        }
    }
}

pub(crate) fn exited(report: ExitReport) -> AcpError {
    AcpError::Exited {
        status: report.status,
        stderr_tail: report.stderr_tail,
    }
}

/// Cancellation of a control future is just as ambiguous as its timeout: a
/// late successful load could replay into a new turn. Retire that connection.
struct ControlGuard<'a> {
    state: &'a State,
    complete: bool,
}
impl<'a> ControlGuard<'a> {
    fn new(state: &'a State) -> Self {
        Self {
            state,
            complete: false,
        }
    }
}
impl Drop for ControlGuard<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.state.close(Duration::ZERO);
        }
    }
}
