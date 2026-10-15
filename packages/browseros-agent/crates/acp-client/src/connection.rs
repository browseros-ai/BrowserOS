//! Runs the SDK connection actor beside the process supervisor. Handles own the
//! connection lifetime; the actor owns only shared routing/process state, avoiding
//! a reference cycle that would otherwise keep stdin and the agent alive forever.

use std::{future::Future, sync::Arc, time::Duration};

use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo, Dispatch, Handled};
use tokio::sync::oneshot;
use tokio_util::{
    compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt},
    sync::CancellationToken,
};

use crate::{
    AcpError, AgentFeatures, ClientOptions, ExitReport, LaunchSpec, PermissionPolicy,
    ProtocolVersion, Session, SessionParams,
    process::Process,
    routing::Routes,
    schema::{
        self, ClientCapabilities, FileSystemCapabilities, Implementation, InitializeRequest,
        InitializeResponse, LoadSessionRequest, NewSessionRequest, RequestPermissionRequest,
        RequestPermissionResponse, ResumeSessionRequest, SessionId, SessionNotification,
    },
};

pub(crate) struct State {
    pub(crate) process: Process,
    pub(crate) routes: Arc<Routes>,
    pub(crate) stop: CancellationToken,
}

pub(crate) struct Inner {
    pub(crate) cx: ConnectionTo<Agent>,
    pub(crate) state: Arc<State>,
    pub(crate) options: ClientOptions,
    init: InitializeResponse,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.state.stop.cancel();
        self.state.process.stop(Duration::from_secs(1));
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
        });
        // If connect is dropped during initialize, no public handle exists yet
        // to own teardown. This guard transfers that ownership only on success.
        let mut startup = ControlGuard::new(&state);
        let (ready, rx) = oneshot::channel();
        let actor_state = state.clone();
        let notification_routes = state.routes.clone();
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
                                    Ok(notification) => notification_routes.update(notification),
                                    Err(error) => tracing::warn!(method = %raw.method, ?tag, %error, "Ignoring unparsed ACP notification"),
                                }
                            } else {
                                tracing::warn!(method = %raw.method, "Ignoring unknown ACP notification");
                            }
                            Ok(Handled::Yes)
                        }
                        other => Ok(Handled::No { message: other, retry: false }),
                    }
                }, agent_client_protocol::on_receive_dispatch!())
                .on_receive_request(async move |request: RequestPermissionRequest, responder, cx| {
                    let policy = permissions.clone();
                    let active = permission_state.routes.permission_turn(&request.session_id);
                    let Some(active) = active else {
                        return responder.respond(RequestPermissionResponse::new(schema::RequestPermissionOutcome::Cancelled));
                    };
                    let stop = active.permission_stop.clone();
                    let connection_stop = permission_state.stop.clone();
                    cx.spawn(async move {
                        let decision = tokio::select! {
                            biased;
                            () = stop.cancelled() => crate::PermissionDecision::Cancel,
                            () = connection_stop.cancelled() => crate::PermissionDecision::Cancel,
                            decision = policy.decide(&request) => decision,
                        };
                        if let Err(error) = active.permission_response(&request, decision, responder) {
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
            &[],
            cx.send_request(request).block_task(),
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
        self.inner.state.stop.cancel();
        self.inner.state.process.stop(grace);
        self.closed().await
    }

    pub async fn new_session(&self, params: SessionParams) -> Result<Session, AcpError> {
        self.check_mcp(&params)?;
        let request = NewSessionRequest::new(params.cwd)
            .mcp_servers(params.mcp_servers)
            .meta(params.meta);
        let response = self
            .control(
                "session/new",
                self.inner.cx.send_request(request).block_task(),
            )
            .await?;
        Ok(Session::new(
            self.clone(),
            response.session_id,
            response.modes,
            response.config_options.unwrap_or_default(),
        ))
    }

    /// No turn can register while load replay is being dispatched. Replayed and
    /// idle updates are deliberately discarded, including after failed loads.
    pub async fn load_session(
        &self,
        id: SessionId,
        params: SessionParams,
    ) -> Result<Session, AcpError> {
        if !self.features().load_session {
            return Err(AcpError::Unsupported("session/load"));
        }
        self.check_mcp(&params)?;
        let _loading = self.inner.state.routes.loading(&id)?;
        let request = LoadSessionRequest::new(id.clone(), params.cwd)
            .mcp_servers(params.mcp_servers)
            .meta(params.meta);
        let response = self
            .control(
                "session/load",
                self.inner.cx.send_request(request).block_task(),
            )
            .await?;
        Ok(Session::new(
            self.clone(),
            id,
            response.modes,
            response.config_options.unwrap_or_default(),
        ))
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
        let _loading = self.inner.state.routes.loading(&id)?;
        let request = ResumeSessionRequest::new(id.clone(), params.cwd)
            .mcp_servers(params.mcp_servers)
            .meta(params.meta);
        let response = self
            .control(
                "session/resume",
                self.inner.cx.send_request(request).block_task(),
            )
            .await?;
        Ok(Session::new(
            self.clone(),
            id,
            response.modes,
            response.config_options.unwrap_or_default(),
        ))
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
        future: impl Future<Output = Result<T, schema::Error>>,
    ) -> Result<T, AcpError> {
        self.available()?;
        control(
            &self.inner.state,
            self.inner.options.control_timeout,
            method,
            &self.auth_methods(),
            future,
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

    pub(crate) async fn map_error(&self, error: schema::Error) -> AcpError {
        map_error(&self.inner.state, &self.auth_methods(), error).await
    }
}

async fn control<T>(
    state: &State,
    after: Duration,
    method: &'static str,
    auth: &[String],
    future: impl Future<Output = Result<T, schema::Error>>,
) -> Result<T, AcpError> {
    let mut guard = ControlGuard::new(state);
    let result = tokio::select! {
        biased;
        report = state.process.closed() => Err(exited(report)),
        () = state.stop.cancelled() => Err(AcpError::Closed),
        result = tokio::time::timeout(after, future) => match result {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => Err(map_error(state, auth, error).await),
            Err(_) => return Err(AcpError::Timeout { method, after }),
        },
    };
    guard.complete = true;
    result
}

async fn map_error(state: &State, auth: &[String], error: schema::Error) -> AcpError {
    if agent_client_protocol::is_incoming_transport_closed(&error) {
        // stdout EOF can beat waitpid by a scheduler tick. Give the sole child
        // owner time to reap and finish the stderr tail before reporting a crash.
        if let Ok(report) =
            tokio::time::timeout(Duration::from_millis(250), state.process.closed()).await
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
    if code == -32000
        || explicit_auth_failure
        || (!auth.is_empty()
            && (lower.contains("authentication")
                || lower.contains("not authenticated")
                || lower.contains("sign in")
                || lower.contains("login required")))
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
            self.state.stop.cancel();
            self.state.process.stop(Duration::ZERO);
        }
    }
}
