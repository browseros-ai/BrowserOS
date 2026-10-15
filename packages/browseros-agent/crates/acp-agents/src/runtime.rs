//! The persistent ACP process for each agent/conversation pair (port of `AcpAgentRuntime` plus the
//! acpx persistent-session behaviour it relied on). One turn:
//!
//! ```text
//! busy? → Busy (HTTP 409)
//! build_policy ─ restart fingerprint differs from the live process? retire it
//! no live process → connect → record reusable? resume (advertised) | load (replay swallowed)
//!                                              ── fails → mark record reset, new session
//!                           : new session
//! apply controls (model → effort → full-access mode) ── fails after a reattach ─┐
//!                                                                               ▼
//!                       resume recovery, before any output: retire, mark reset, fresh session
//! prompt(has_history ? latest user : transcript seed + latest user) → Turn
//! driver task: updates → UiTranslator → chunks; host stop → session/cancel + `abort`;
//!              deadline → cancel + error; keeps draining until the agent answers (bounded grace)
//! stream end → release the busy lock, arm the idle close
//! ```
//!
//! Locking: one std mutex guards the busy set and the live-process map; it is never held across an
//! await. A turn's busy claim gives it exclusive use of its conversation's live process; `close*`
//! may still retire that process mid-turn, which ends the turn with an error, as in TS.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use acp_client::{
    AcpError, AgentConnection, ClientOptions, PermissionPolicy, Session, SessionParams, Turn,
    TurnEvent,
    schema::{
        ContentBlock, SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory,
        SessionConfigSelectOptions, SessionId, TextContent,
    },
};
use futures_util::FutureExt;
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use ui_message_stream::{ChunkStream, UiChunk};
use uuid::Uuid;

use crate::{
    launch::{HostEnvironment, LaunchError},
    policy::{
        AgentPolicy, SessionControls, SessionKey, TurnContext, build_policy, ensure_workspace,
        launch_fingerprint,
    },
    store::{SESSION_RECORD_SCHEMA, SessionRecord, SessionStore},
    translate::{TurnEnd, UiTranslator},
};

pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// acpx-ai-provider `DEFAULT_TURN_TIMEOUT_MS`.
pub const DEFAULT_TURN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Covers `initialize` on a cold machine, where `bun x` first downloads the adapter (the probe
/// allows 120 s for the same reason).
pub const DEFAULT_CONTROL_TIMEOUT: Duration = Duration::from_secs(120);
/// Reason on the `abort` chunk of a host stop; the same text AI SDK produced for the TS stop.
pub const HOST_STOP_REASON: &str = "AbortError: Conversation stopped";
/// After cancel or deadline, how long the agent may take to answer the prompt before its process is
/// killed (capped by the control timeout). Claude and Codex settle within 60 ms in practice.
const CANCEL_GRACE: Duration = Duration::from_secs(10);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);
const SESSION_CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

pub struct AcpRuntimeConfig {
    pub host: HostEnvironment,
    pub store: Arc<dyn SessionStore>,
    pub permissions: Arc<dyn PermissionPolicy>,
    /// Closes a session's process this long after its last turn; zero disables idle close.
    pub idle_timeout: Duration,
    /// Whole-turn deadline from prompt to response.
    pub turn_timeout: Duration,
    /// Deadline for initialize, session/new|load|resume, set_mode, set_config_option.
    pub control_timeout: Duration,
}

impl AcpRuntimeConfig {
    /// Production defaults: approve-all permissions (acpx `permissionMode: 'approve-all'`).
    pub fn new(host: HostEnvironment, store: Arc<dyn SessionStore>) -> Self {
        Self {
            host,
            store,
            permissions: Arc::new(acp_client::ApproveAll),
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            turn_timeout: DEFAULT_TURN_TIMEOUT,
            control_timeout: DEFAULT_CONTROL_TIMEOUT,
        }
    }
}

pub struct TurnPrompt {
    /// The newest user message as ACP content: text first (already in convert-prompt's
    /// `User: …` line form), then image/audio blocks.
    pub latest_user: Vec<ContentBlock>,
    /// Prior history as `User: …` / `Assistant: …` / `Tool: …` lines joined by `\n`, sent only when
    /// the agent holds no context of its own (fresh session, or after a resume recovery). It is
    /// joined to the latest user text with `\n`, reproducing TS's whole-transcript prompt.
    pub transcript_seed: Option<String>,
}

pub struct TurnRequest {
    pub ctx: TurnContext,
    pub prompt: TurnPrompt,
    /// UUID for the `start` chunk and the persisted assistant message.
    pub response_message_id: String,
    /// Host stop (`POST /chat/:id/stop`): `session/cancel`, then an `abort` chunk.
    pub cancel: CancellationToken,
}

/// Every variant except `Busy` is rendered by the server as the single preparation error chunk
/// (`"Unable to start the ACP agent."`, TS `AcpAgentPreparationError`).
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// HTTP 409 "An agent turn is already running".
    #[error("an ACP turn is already running for this conversation")]
    Busy,
    #[error("unable to start the ACP agent")]
    Preparation(#[source] AcpError),
    #[error(transparent)]
    Launch(#[from] LaunchError),
    /// DECIDED: new variant. WHY: TS fails the turn when the shared workspace cannot be written,
    /// and neither `AcpError` nor `LaunchError` can carry an I/O error honestly.
    #[error("unable to prepare the ACP workspace")]
    Workspace(#[source] std::io::Error),
}

/// A live agent process holding the conversation's ACP session.
struct Live {
    conn: AgentConnection,
    session: Session,
    fingerprint: String,
    record: Mutex<SessionRecord>,
    /// TS `hasHistory`: whether the next prompt may omit the transcript seed. Set on attach from
    /// the record, and after a turn that neither failed nor was aborted.
    has_history: AtomicBool,
    idle: Mutex<Option<JoinHandle<()>>>,
}

impl Live {
    fn is_alive(&self) -> bool {
        self.conn.closed().now_or_never().is_none()
    }
    fn clear_idle(&self) {
        if let Some(handle) = lock(&self.idle).take() {
            handle.abort();
        }
    }
}

#[derive(Default)]
struct State {
    busy: HashSet<SessionKey>,
    live: HashMap<SessionKey, Arc<Live>>,
    shut_down: bool,
}

pub struct AcpRuntime {
    config: AcpRuntimeConfig,
    state: Mutex<State>,
    workspace: tokio::sync::OnceCell<PathBuf>,
    this: Weak<AcpRuntime>,
}

impl AcpRuntime {
    pub fn new(config: AcpRuntimeConfig) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            config,
            state: Mutex::default(),
            workspace: tokio::sync::OnceCell::new(),
            this: this.clone(),
        })
    }

    /// Acquire (or create/reattach) the conversation's session, re-apply controls, prompt, and
    /// translate. On `Ok` the stream starts with `start{messageId}` and always terminates with
    /// `finish`, `abort`, or `error` + `finish`; the busy lock is held until it ends.
    pub async fn stream(
        self: &Arc<Self>,
        request: TurnRequest,
    ) -> Result<ChunkStream, RuntimeError> {
        let TurnRequest {
            ctx,
            prompt,
            response_message_id,
            cancel,
        } = request;
        let key = SessionKey::new(ctx.agent.id, ctx.conversation_id);
        let busy = self.claim(key.clone())?;
        let host = self.config.host.clone();
        self.workspace
            .get_or_try_init(|| {
                let dir = host.browseros_dir.clone();
                async move {
                    tokio::task::spawn_blocking(move || ensure_workspace(&dir))
                        .await
                        .map_err(std::io::Error::other)?
                }
            })
            .await
            .map_err(RuntimeError::Workspace)?;
        let agent_id = ctx.agent.id;
        let policy = tokio::task::spawn_blocking(move || build_policy(&ctx, &host))
            .await
            .map_err(|error| {
                tracing::error!(%error, "ACP policy task failed");
                RuntimeError::Preparation(AcpError::Closed)
            })??;

        let (live, turn) = match self.start_turn(&policy, prompt).await {
            Ok(started) => started,
            Err(error) => {
                tracing::error!(%agent_id, %error, "ACP agent preparation failed");
                return Err(RuntimeError::Preparation(error));
            }
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = tx.send(UiChunk::Start {
            message_metadata: None,
            message_id: Some(response_message_id),
        });
        let driver = Driver {
            runtime: self.clone(),
            live,
            tx,
            cancel,
            turn_timeout: self.config.turn_timeout,
            grace: CANCEL_GRACE.min(self.config.control_timeout),
            busy,
        };
        tokio::spawn(driver.run(turn));
        Ok(Box::pin(futures_util::stream::unfold(
            rx,
            |mut rx| async move { rx.recv().await.map(|chunk| (chunk, rx)) },
        )))
    }

    /// Closes the conversation's live process (TS `close`). With `discard_persistent_state` the
    /// session record is deleted too, even when no process is live. Returns whether one was live.
    pub async fn close(
        &self,
        agent_id: Uuid,
        conversation_id: Uuid,
        discard_persistent_state: bool,
    ) -> bool {
        let key = SessionKey::new(agent_id, conversation_id);
        let live = lock(&self.state).live.remove(&key);
        let found = live.is_some();
        if let Some(live) = live {
            retire(live).await;
        }
        if discard_persistent_state {
            self.delete_record(&key);
        }
        found
    }

    /// Closes every live process of an agent (agent edit/delete). Like TS, only live sessions'
    /// records are discarded; a cold conversation's record stays until its own close.
    pub async fn close_all_for_agent(
        &self,
        agent_id: Uuid,
        discard_persistent_state: bool,
    ) -> usize {
        let retired: Vec<(SessionKey, Arc<Live>)> = {
            let mut state = lock(&self.state);
            let keys: Vec<SessionKey> = state
                .live
                .keys()
                .filter(|key| key.belongs_to(agent_id))
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|key| state.live.remove(&key).map(|live| (key, live)))
                .collect()
        };
        let count = retired.len();
        futures_util::future::join_all(retired.into_iter().map(|(key, live)| async move {
            retire(live).await;
            if discard_persistent_state {
                self.delete_record(&key);
            }
        }))
        .await;
        count
    }

    /// Retires every live process; later turns fail preparation.
    pub async fn shutdown(&self) {
        let all: Vec<Arc<Live>> = {
            let mut state = lock(&self.state);
            state.shut_down = true;
            state.live.drain().map(|(_, live)| live).collect()
        };
        futures_util::future::join_all(all.into_iter().map(retire)).await;
    }

    fn claim(&self, key: SessionKey) -> Result<BusyGuard, RuntimeError> {
        let mut state = lock(&self.state);
        if state.shut_down {
            return Err(RuntimeError::Preparation(AcpError::Closed));
        }
        if !state.busy.insert(key.clone()) {
            return Err(RuntimeError::Busy);
        }
        Ok(BusyGuard {
            runtime: self.this.clone(),
            key,
        })
    }

    /// Prepare and prompt. A cached process found dead at prompt time (it exited while idle) is
    /// reopened once, still before any output, so the user never sees the stale process.
    async fn start_turn(
        &self,
        policy: &AgentPolicy,
        prompt: TurnPrompt,
    ) -> Result<(Arc<Live>, Turn), AcpError> {
        let (live, created) = self.prepare(policy).await?;
        let content = compose_prompt(&live, &prompt);
        let (live, turn) = match live.session.prompt(content) {
            Ok(turn) => (live, turn),
            Err(error) if !created => {
                tracing::warn!(%error, "Reopening an ACP session whose process ended while idle");
                self.forget(&policy.session_key, &live);
                tokio::spawn(retire(live));
                let (live, _) = self.prepare(policy).await?;
                let turn = live.session.prompt(compose_prompt(&live, &prompt))?;
                (live, turn)
            }
            Err(error) => {
                self.forget(&policy.session_key, &live);
                tokio::spawn(retire(live));
                return Err(error);
            }
        };
        self.mark_user_turn(&live);
        Ok((live, turn))
    }

    /// Returns the live process for the policy and whether it was opened by this call.
    async fn prepare(&self, policy: &AgentPolicy) -> Result<(Arc<Live>, bool), AcpError> {
        let key = &policy.session_key;
        let fingerprint = policy.restart_fingerprint();
        let existing = lock(&self.state).live.get(key).cloned();
        if let Some(live) = existing {
            live.clear_idle();
            if live.fingerprint == fingerprint && live.is_alive() {
                // TS: a control failure on a reused process fails the turn but keeps the process.
                apply_controls(&live.session, &policy.controls).await?;
                return Ok((live, false));
            }
            // Awaited, like TS: the replacement may reattach to the very same agent session.
            tracing::debug!("Retiring ACP process after a policy change or exit");
            self.forget(key, &live);
            retire(live).await;
        }

        let (live, reattached) = self.open(policy, &fingerprint, true).await?;
        match apply_controls(&live.session, &policy.controls).await {
            Ok(()) => {}
            Err(error) if reattached => {
                // acpx SESSION_*_REPLAY_FAILED → TS resetSession: once, and before any output.
                tracing::warn!(%error, "Starting a fresh ACP session after resume failed");
                let record = lock(&live.record).clone();
                retire(live).await;
                self.save_record(SessionRecord {
                    reset_on_next_ensure: true,
                    ..record
                });
                let (fresh, _) = self.open(policy, &fingerprint, false).await?;
                if let Err(error) = apply_controls(&fresh.session, &policy.controls).await {
                    retire(fresh).await;
                    return Err(error);
                }
                lock(&self.state).live.insert(key.clone(), fresh.clone());
                return Ok((fresh, true));
            }
            Err(error) => {
                retire(live).await;
                return Err(error);
            }
        }
        lock(&self.state).live.insert(key.clone(), live.clone());
        Ok((live, true))
    }

    /// Spawn + initialize, then reattach to the recorded session when allowed and reusable, else
    /// start a new one. A failed reattach marks the record reset and falls back to a new session,
    /// reconnecting first when the failure retired the connection. Returns whether it reattached.
    async fn open(
        &self,
        policy: &AgentPolicy,
        fingerprint: &str,
        allow_reattach: bool,
    ) -> Result<(Arc<Live>, bool), AcpError> {
        let key = &policy.session_key;
        let spec = &policy.launch.spec;
        let launch_fp = launch_fingerprint(spec);
        let params = SessionParams {
            cwd: policy.cwd.clone(),
            mcp_servers: policy.mcp_servers.clone(),
            meta: policy.session_meta.clone(),
        };
        let mut conn = self.connect(policy).await?;
        let record = (if allow_reattach {
            self.load_record(key)
        } else {
            None
        })
        .filter(|record| record.reusable_for(&policy.cwd, &launch_fp));
        let mut attached = None;
        if let Some(record) = record {
            let id = SessionId::new(record.agent_session_id.clone());
            let features = conn.features();
            let result = if features.resume_session {
                conn.resume_session(id, params.clone()).await
            } else {
                conn.load_session(id, params.clone()).await
            };
            match result {
                Ok(session) => attached = Some((session, record.has_user_turn)),
                Err(error) => {
                    tracing::warn!(%error, "ACP session reattach failed; starting a new session");
                    self.save_record(SessionRecord {
                        reset_on_next_ensure: true,
                        ..record
                    });
                    if matches!(
                        error,
                        AcpError::Timeout { .. } | AcpError::Closed | AcpError::Exited { .. }
                    ) {
                        conn = self.connect(policy).await?;
                    }
                }
            }
        }
        let reattached = attached.is_some();
        let (session, has_history) = match attached {
            Some(attached) => attached,
            None => (conn.new_session(params).await?, false),
        };
        let record = SessionRecord {
            schema: SESSION_RECORD_SCHEMA.into(),
            session_key: key.as_str().into(),
            agent_session_id: session.id().0.to_string(),
            launch_fingerprint: launch_fp,
            cwd: policy.cwd.clone(),
            has_user_turn: has_history,
            reset_on_next_ensure: false,
            updated_at_ms: now_ms(),
        };
        self.save_record(record.clone());
        Ok((
            Arc::new(Live {
                conn,
                session,
                fingerprint: fingerprint.to_owned(),
                record: Mutex::new(record),
                has_history: AtomicBool::new(has_history),
                idle: Mutex::new(None),
            }),
            reattached,
        ))
    }

    async fn connect(&self, policy: &AgentPolicy) -> Result<AgentConnection, AcpError> {
        let options = ClientOptions {
            client_name: "browseros".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
            control_timeout: self.config.control_timeout,
            stderr_tail_lines: 20,
            shutdown_grace: SHUTDOWN_GRACE,
            ..ClientOptions::default()
        };
        AgentConnection::connect(
            policy.launch.spec.clone(),
            options,
            self.config.permissions.clone(),
        )
        .await
    }

    /// Removes `live` from the map only if it is still the current process for `key`.
    fn forget(&self, key: &SessionKey, live: &Arc<Live>) {
        let mut state = lock(&self.state);
        if state
            .live
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, live))
        {
            state.live.remove(key);
        }
    }

    /// The agent now holds a user prompt: a future process may resume with the latest turn only.
    fn mark_user_turn(&self, live: &Live) {
        let record = {
            let mut record = lock(&live.record);
            if record.has_user_turn {
                return;
            }
            record.has_user_turn = true;
            record.updated_at_ms = now_ms();
            record.clone()
        };
        self.save_record(record);
    }

    fn load_record(&self, key: &SessionKey) -> Option<SessionRecord> {
        self.config.store.load(key).unwrap_or_else(|error| {
            tracing::warn!(%error, "Unable to read the ACP session record; starting fresh");
            None
        })
    }

    /// Record writes are best effort: a lost record costs agent-side context, never the turn.
    fn save_record(&self, record: SessionRecord) {
        if let Err(error) = self.config.store.save(&record) {
            tracing::warn!(%error, "Unable to persist the ACP session record");
        }
    }

    fn delete_record(&self, key: &SessionKey) {
        if let Err(error) = self.config.store.delete(key) {
            tracing::warn!(%error, "Unable to delete the ACP session record");
        }
    }

    /// Closes the conversation's process `idle_timeout` after its last turn, unless a turn started
    /// or the process was replaced meanwhile.
    fn arm_idle(&self, key: &SessionKey) {
        let timeout = self.config.idle_timeout;
        let Some(live) = lock(&self.state).live.get(key).cloned() else {
            return;
        };
        live.clear_idle();
        if timeout.is_zero() {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let runtime = self.this.clone();
        let key = key.clone();
        let target = Arc::downgrade(&live);
        let task = handle.spawn(async move {
            tokio::time::sleep(timeout).await;
            let (Some(runtime), Some(live)) = (runtime.upgrade(), target.upgrade()) else {
                return;
            };
            {
                let mut state = lock(&runtime.state);
                let current = state
                    .live
                    .get(&key)
                    .is_some_and(|current| Arc::ptr_eq(current, &live));
                if state.busy.contains(&key) || !current {
                    return;
                }
                state.live.remove(&key);
            }
            // Detach our own handle first so retire's clear_idle cannot abort this task.
            lock(&live.idle).take();
            tracing::debug!("Closing idle ACP session");
            retire(live).await;
        });
        *lock(&live.idle) = Some(task);
    }
}

/// Released when the turn's stream ends (or preparation fails): frees the conversation for the next
/// turn and arms the idle close.
struct BusyGuard {
    runtime: Weak<AcpRuntime>,
    key: SessionKey,
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.upgrade() {
            lock(&runtime.state).busy.remove(&self.key);
            runtime.arm_idle(&self.key);
        }
    }
}

/// Owns one prompt turn after `stream` returns. It keeps reading the ACP turn after the consumer
/// is told the turn ended (stop, deadline, or dropped stream), because the agent still has to answer
/// the prompt before the session can take another one.
struct Driver {
    runtime: Arc<AcpRuntime>,
    live: Arc<Live>,
    tx: mpsc::UnboundedSender<UiChunk>,
    cancel: CancellationToken,
    turn_timeout: Duration,
    grace: Duration,
    busy: BusyGuard,
}

impl Driver {
    async fn run(self, mut turn: Turn) {
        let Self {
            runtime,
            live,
            tx,
            cancel,
            turn_timeout,
            grace: grace_period,
            busy,
        } = self;
        let send = |chunks: Vec<UiChunk>| {
            for chunk in chunks {
                let _ = tx.send(chunk);
            }
        };
        let canceller = turn.canceller();
        let mut translator = UiTranslator::new();
        let deadline = tokio::time::sleep(turn_timeout);
        tokio::pin!(deadline);
        let mut ended = false;
        let mut grace: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;
        let mut succeeded = false;
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled(), if !ended => {
                    canceller.cancel();
                    send(translator.finish(TurnEnd::CancelledByHost { reason: HOST_STOP_REASON.into() }));
                    ended = true;
                    grace = Some(Box::pin(tokio::time::sleep(grace_period)));
                }
                () = &mut deadline, if !ended => {
                    canceller.cancel();
                    let message = format!("ACP turn timed out after {turn_timeout:?}");
                    send(translator.finish(TurnEnd::Failed { message }));
                    ended = true;
                    grace = Some(Box::pin(tokio::time::sleep(grace_period)));
                }
                () = tx.closed(), if !ended => {
                    canceller.cancel();
                    ended = true;
                    grace = Some(Box::pin(tokio::time::sleep(grace_period)));
                }
                () = wait(&mut grace) => {
                    tracing::warn!("ACP agent did not settle after cancellation; closing its process");
                    runtime.forget(&busy.key, &live);
                    tokio::spawn(retire(live.clone()));
                    break;
                }
                event = turn.next() => match event {
                    Some(TurnEvent::Update(update)) => {
                        if !ended {
                            send(translator.on_update(&update));
                        }
                    }
                    Some(TurnEvent::Done(result)) => {
                        if !ended {
                            let end = match result {
                                Ok(outcome) => TurnEnd::Completed(outcome.stop_reason),
                                Err(error) => TurnEnd::Failed { message: error_text(&error) },
                            };
                            let chunks = translator.finish(end);
                            succeeded = !chunks.iter().any(|chunk| matches!(chunk, UiChunk::Error { .. }));
                            send(chunks);
                        }
                        break;
                    }
                    None => {
                        if !ended {
                            send(translator.finish(TurnEnd::Failed { message: error_text(&AcpError::Closed) }));
                        }
                        break;
                    }
                }
            }
        }
        if succeeded {
            live.has_history.store(true, Ordering::SeqCst);
        }
        drop(tx);
        drop(busy);
    }
}

async fn wait(timer: &mut Option<std::pin::Pin<Box<tokio::time::Sleep>>>) {
    match timer {
        Some(timer) => timer.await,
        None => std::future::pending().await,
    }
}

/// `session/close` when advertised (acpx close), then stdin EOF; acp-client retires the process
/// group gracefully within the grace period. A running turn on this process ends with an error.
async fn retire(live: Arc<Live>) {
    live.clear_idle();
    if live.conn.features().close_session {
        match tokio::time::timeout(SESSION_CLOSE_TIMEOUT, live.session.close()).await {
            Ok(Err(error)) => tracing::debug!(%error, "ACP session/close failed"),
            Err(_) => tracing::debug!("ACP session/close timed out"),
            Ok(Ok(())) => {}
        }
    }
    let report = live.conn.clone().shutdown(SHUTDOWN_GRACE).await;
    tracing::debug!(status = ?report.status, "ACP agent process closed");
}

/// The seed goes in front of the latest user text, joined by `\n`, so the agent sees one prompt
/// shaped like TS's `convertPrompt` of the whole transcript.
fn compose_prompt(live: &Live, prompt: &TurnPrompt) -> Vec<ContentBlock> {
    let mut content = prompt.latest_user.clone();
    let seed = prompt
        .transcript_seed
        .as_deref()
        .filter(|seed| !seed.is_empty());
    let Some(seed) = seed.filter(|_| !live.has_history.load(Ordering::SeqCst)) else {
        return content;
    };
    match content.first_mut() {
        Some(ContentBlock::Text(text)) => text.text = format!("{seed}\n{}", text.text),
        _ => content.insert(0, ContentBlock::Text(TextContent::new(seed))),
    }
    content
}

/// Model, then reasoning effort, then the first accepted full-access mode (a model change may
/// reset the mode, so mode goes last).
async fn apply_controls(session: &Session, controls: &SessionControls) -> Result<(), AcpError> {
    if let Some(model) = &controls.model {
        match advertised_model_option(&session.config_options(), model) {
            ModelOption::Select(id) => {
                let echoed = session.set_config_option(&id, model).await?;
                if !matches!(
                    advertised_model_option(&echoed, model),
                    ModelOption::Current
                ) {
                    // An agent can accept the call yet keep its old model.
                    return Err(AcpError::Unsupported("the requested model"));
                }
            }
            ModelOption::Current => {}
            ModelOption::NotAdvertised => {
                tracing::debug!(
                    "ACP agent does not advertise the requested model; leaving launch config"
                );
            }
        }
    }
    if let Some((id, effort)) = &controls.reasoning_effort
        && let Err(error) = session.set_config_option(id, effort).await
    {
        tracing::warn!(%error, config = %id, "ACP reasoning effort was rejected");
    }
    if controls.full_access_modes.is_empty() {
        // Common for custom agents: run in the agent's own default permission mode.
        return Ok(());
    }
    let mut last = None;
    for mode in &controls.full_access_modes {
        match session.set_mode(mode).await {
            Ok(()) => return Ok(()),
            Err(error) => last = Some(error),
        }
    }
    let error = last.unwrap_or(AcpError::Unsupported("full-access mode"));
    tracing::warn!(%error, "Unable to enable full access for the ACP agent");
    Err(error)
}

enum ModelOption {
    /// Advertised and selectable under this config id, but not current.
    Select(String),
    Current,
    NotAdvertised,
}

/// The model select option (category `model`, else id `model`) and whether `model` is current or
/// one of its values.
fn advertised_model_option(options: &[SessionConfigOption], model: &str) -> ModelOption {
    let option = options
        .iter()
        .find(|option| option.category == Some(SessionConfigOptionCategory::Model))
        .or_else(|| {
            options
                .iter()
                .find(|option| option.id.0.as_ref() == "model")
        });
    let Some(SessionConfigOption {
        id,
        kind: SessionConfigKind::Select(select),
        ..
    }) = option
    else {
        return ModelOption::NotAdvertised;
    };
    if select.current_value.0.as_ref() == model {
        return ModelOption::Current;
    }
    let listed = match &select.options {
        SessionConfigSelectOptions::Ungrouped(values) => {
            values.iter().any(|value| value.value.0.as_ref() == model)
        }
        SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| &group.options)
            .any(|value| value.value.0.as_ref() == model),
        _ => false,
    };
    if listed {
        ModelOption::Select(id.0.to_string())
    } else {
        ModelOption::NotAdvertised
    }
}

/// UI text for a failed turn: the agent's own message where it has one.
fn error_text(error: &AcpError) -> String {
    match error {
        AcpError::AuthRequired { message, .. } | AcpError::Rpc { message, .. } => message.clone(),
        AcpError::Exited {
            status,
            stderr_tail,
        } => {
            let mut text = match status.and_then(|status| status.code()) {
                Some(code) => format!("The ACP agent exited with code {code}."),
                None => "The ACP agent exited unexpectedly.".to_owned(),
            };
            if let Some(line) = stderr_tail
                .iter()
                .rev()
                .map(|line| line.trim())
                .find(|line| !line.is_empty())
            {
                text.push(' ');
                text.extend(line.chars().take(300));
            }
            text
        }
        AcpError::Timeout { method, after } => {
            format!(
                "The ACP agent timed out during {method} after {}s.",
                after.as_secs()
            )
        }
        AcpError::Closed => "The ACP agent connection closed.".to_owned(),
        other => other.to_string(),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
