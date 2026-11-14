//! The desktop host's handle on the SDK's Playwright engine. This module owns the
//! re-executed child, its files, loopback MCP wire and reply metadata. Callers never
//! depend on an engine port or the SDK's private metadata keys.

mod child;
#[cfg(unix)]
mod engine_main;
mod host_api;
mod reply;
mod session_tasks;
mod wire;

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use rmcp::model::{CallToolResult, JsonObject};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{sync::watch, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{config::Config, ids::ConvoId};

pub use host_api::{OperationOutcome, RunOperation};
pub use session_tasks::TaskKey;

/// Cheap shared handle. Tests construct a disabled engine, so building AppState
/// never starts another process or connects an SDK to a browser.
#[derive(Clone)]
pub struct NeoEngine {
    inner: Arc<Inner>,
}

struct Inner {
    state: watch::Sender<EngineState>,
    connection: Mutex<Connection>,
    sessions: Mutex<session_tasks::SessionTasks>,
    started: AtomicBool,
    stop: CancellationToken,
    supervisor: Mutex<Option<JoinHandle<()>>>,
}

/// The wire belongs to the current child. The last validated definition survives
/// that child's failure so discovery can still offer the SDK's cached tool.
#[derive(Default)]
struct Connection {
    wire: Option<wire::Wire>,
    host: Option<host_api::HostApi>,
    tool: Option<RunTool>,
}

/// Ready means health and tools/list answered. The SDK itself reports a browser
/// link that is still connecting.
#[derive(Clone, Debug)]
pub enum EngineState {
    Disabled(&'static str),
    Starting,
    Ready,
    Unavailable {
        reason: String,
        retry_at: Option<Instant>,
    },
}

/// Launch facts from the host. The original file supplies the browser's native
/// identity and profile-protection path without exposing SDK config to callers.
#[derive(Clone)]
pub struct EngineLaunch {
    state_dir: PathBuf,
    resources_dir: PathBuf,
    config_path: PathBuf,
    cdp_port: u16,
    browseros_version: Option<String>,
    hosted: bool,
}

impl EngineLaunch {
    #[must_use]
    pub fn from_config(config: &Config, config_path: &Path) -> Self {
        Self {
            state_dir: config.browserclaw_dir.clone(),
            resources_dir: config.resources_dir.clone(),
            config_path: config_path.to_path_buf(),
            cdp_port: config.cdp_port,
            browseros_version: config
                .native
                .capabilities
                .browseros_version()
                .map(|version| version.to_string()),
            hosted: config.mode.is_hosted(),
        }
    }
}

/// The SDK's definition from tools/list or its saved copy. The only locally
/// authored definition is the unavailable fallback, never an SDK description.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTool {
    pub description: String,
    pub input_schema: Arc<JsonObject>,
}

impl RunTool {
    /// rmcp remembers the first schema for a tool name. Reject a stale saved
    /// dialect before discovery can pin it for this host process's lifetime.
    #[cfg(unix)]
    fn has_compatible_schema(&self) -> bool {
        let schema = &self.input_schema;
        let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
            return false;
        };
        schema.get("type") == Some(&json!("object"))
            && schema.get("additionalProperties") == Some(&json!(false))
            && schema.get("required") == Some(&json!(["code"]))
            && properties.len() == 2
            && properties.get("code").and_then(|code| code.get("type")) == Some(&json!("string"))
            && properties
                .get("timeout")
                .and_then(|timeout| timeout.get("type"))
                == Some(&json!("integer"))
    }

    #[must_use]
    pub fn fallback() -> Self {
        Self {
            description: "run is unavailable: the Playwright engine has not started.".to_owned(),
            input_schema: Arc::new(
                json!({
                    "type": "object",
                    "properties": {
                        "code": {"type": "string"},
                        "timeout": {"type": "integer", "minimum": 1, "maximum": 900000}
                    },
                    "required": ["code"],
                    "additionalProperties": false
                })
                .as_object()
                .cloned()
                .unwrap_or_default(),
            ),
        }
    }
}

/// Arguments pass through unchanged for the SDK to validate. A host census has
/// no task; an agent call seeds the session's current SDK task.
pub struct RunCall {
    pub args: JsonObject,
    pub task: Option<TaskKey>,
    pub host_data: Value,
}

/// The agent-visible reply, separated from the engine's run metadata.
pub type AgentReply = CallToolResult;

/// One reply parsed at the wire boundary. Task touches include failed runs and
/// exclude the implicit host-seeded task, which remains in SessionTasks.
pub struct RunOutcome {
    pub reply: AgentReply,
    pub run_id: String,
    pub status: RunStatus,
    pub effects: Effects,
    pub tasks: Vec<TaskTouch>,
}

/// The SDK's terminal run status, decoded at the loopback boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Ok,
    Error,
    Cancelled,
    Timeout,
}

/// Whether the SDK confirmed all writes, sent none, or left a write's outcome unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effects {
    None,
    Done,
    Unknown,
}

/// A script's explicit task selection or closure, separate from the host-seeded task.
#[derive(Clone, Debug)]
pub struct TaskTouch {
    pub id: TaskKey,
    pub name: Option<String>,
    pub kind: TaskTouchKind,
    pub tabs: Vec<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskTouchKind {
    Created,
    Fetched,
    Closed,
}

#[derive(Debug, thiserror::Error)]
#[error("run is unavailable: {reason}")]
pub struct EngineUnavailable {
    pub reason: String,
}

impl EngineUnavailable {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl NeoEngine {
    #[must_use]
    pub fn disabled(why: &'static str) -> Self {
        let (state, _) = watch::channel(EngineState::Disabled(why));
        Self {
            inner: Arc::new(Inner {
                state,
                connection: Mutex::new(Connection::default()),
                sessions: Mutex::new(session_tasks::SessionTasks::default()),
                started: AtomicBool::new(false),
                stop: CancellationToken::new(),
                supervisor: Mutex::new(None),
            }),
        }
    }

    /// Starts once without holding up host boot. A failed start becomes state.
    pub fn start(&self, launch: EngineLaunch) {
        if self.inner.stop.is_cancelled() {
            return;
        }
        if self.inner.started.swap(true, Ordering::AcqRel) {
            return;
        }
        if launch.hosted {
            tracing::info!("the Playwright engine is disabled in hosted mode");
            self.inner
                .state
                .send_replace(EngineState::Disabled("hosted mode uses its existing run"));
            return;
        }
        #[cfg(unix)]
        {
            tracing::info!("the Playwright engine is starting");
            self.inner.state.send_replace(EngineState::Starting);
            let engine = self.clone();
            let task = tokio::spawn(async move {
                child::supervise(&engine, launch).await;
            });
            *self
                .inner
                .supervisor
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(task);
        }
        #[cfg(not(unix))]
        {
            tracing::info!("the Playwright engine is disabled on Windows");
            self.inner.state.send_replace(EngineState::Disabled(
                "the Playwright run is not available on Windows yet",
            ));
        }
    }

    #[must_use]
    pub fn state(&self) -> EngineState {
        self.inner.state.borrow().clone()
    }

    /// Subscribers observe readiness and definition changes without owning the child.
    #[must_use]
    pub fn watch(&self) -> watch::Receiver<EngineState> {
        self.inner.state.subscribe()
    }

    /// Initial discovery waits briefly for the real definition. Other host
    /// endpoints stay responsive, and a slow or failed start still has a fallback.
    pub async fn tool(&self) -> RunTool {
        let mut state = self.watch();
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            while matches!(*state.borrow_and_update(), EngineState::Starting) {
                if state.changed().await.is_err() {
                    break;
                }
            }
        })
        .await;
        self.tool_snapshot()
    }

    /// rmcp's synchronous get_tool hook validates headers and schemas. It cannot
    /// await startup; tools/list uses tool() before reading this same snapshot.
    #[must_use]
    pub fn tool_snapshot(&self) -> RunTool {
        if let Some(tool) = &self
            .inner
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tool
        {
            return tool.clone();
        }
        let mut tool = RunTool::fallback();
        match self.state() {
            EngineState::Disabled(why) => {
                tool.description = format!("run is unavailable: {why}.");
            }
            EngineState::Unavailable { reason, .. } => {
                tool.description = format!("run is unavailable: {reason}.");
            }
            EngineState::Starting | EngineState::Ready => {}
        }
        tool
    }

    /// Dropping this future drops the response stream. Without requestId the
    /// SDK cancels immediately when its stateless HTTP caller disappears.
    pub async fn run(&self, call: RunCall) -> Result<RunOutcome, EngineUnavailable> {
        if self.inner.stop.is_cancelled() || !matches!(self.state(), EngineState::Ready) {
            return Err(self.not_ready());
        }
        let wire = self
            .inner
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .wire
            .clone()
            .ok_or_else(|| self.not_ready())?;
        let mut result = wire.run(call).await;
        if result.is_err() && !matches!(self.state(), EngineState::Ready) {
            return Err(self.not_ready());
        }
        if let Ok(outcome) = &mut result {
            self.hydrate_blobs(outcome).await;
        }
        result
    }

    #[must_use]
    pub fn current_task(&self, convo: &ConvoId) -> TaskKey {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .current(convo)
    }

    pub fn observe_tasks(&self, convo: &ConvoId, touches: &[TaskTouch]) {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .observe(convo, touches);
    }

    #[must_use]
    pub fn session_tasks(&self, convo: &ConvoId) -> Vec<TaskKey> {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tasks(convo)
    }

    #[cfg(test)]
    pub(crate) fn for_test_endpoint(port: u16) -> Result<Self, EngineUnavailable> {
        let engine = Self::disabled("test engine");
        engine
            .inner
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .wire = Some(wire::Wire::new(port)?);
        engine.inner.state.send_replace(EngineState::Ready);
        Ok(engine)
    }

    /// Idempotent. The supervisor closes stdin, waits two seconds and kills if
    /// necessary. kill_on_drop also covers cancellation of the supervisor.
    pub async fn shutdown(&self) {
        self.inner.stop.cancel();
        self.unavailable("the Playwright engine was shut down".to_owned());
        let task = self
            .inner
            .supervisor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(mut task) = task
            && tokio::time::timeout(Duration::from_secs(3), &mut task)
                .await
                .is_err()
        {
            task.abort();
            let _ = task.await;
        }
    }

    fn unavailable(&self, reason: String) {
        tracing::info!(%reason, "the Playwright engine is unavailable");
        self.unavailable_until(reason, None);
    }

    fn unavailable_until(&self, reason: String, retry_at: Option<Instant>) {
        let mut connection = self
            .inner
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        connection.wire = None;
        connection.host = None;
        drop(connection);
        self.inner
            .state
            .send_replace(EngineState::Unavailable { reason, retry_at });
    }

    fn not_ready(&self) -> EngineUnavailable {
        // Readiness can race shutdown on another runtime thread. Once stopped,
        // no caller may forward more work even if it saw the previous Ready state.
        if self.inner.stop.is_cancelled() {
            return EngineUnavailable::new("the Playwright engine was shut down");
        }
        EngineUnavailable::new(match self.state() {
            EngineState::Disabled(why) => why.to_owned(),
            EngineState::Starting => "the Playwright engine is starting".to_owned(),
            EngineState::Unavailable { reason, retry_at } => {
                if retry_at.is_some() {
                    format!("{reason}; restarting")
                } else {
                    reason
                }
            }
            EngineState::Ready => "the Playwright engine connection is gone".to_owned(),
        })
    }
}

#[must_use]
pub fn is_engine_invocation() -> bool {
    std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "__neo-sdk")
}

/// Called before the host constructs a Tokio runtime or installs its logger.
#[cfg(unix)]
pub fn engine_main() -> ! {
    std::process::exit(engine_main::run());
}

#[cfg(not(unix))]
pub fn engine_main() -> ! {
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::{EngineState, NeoEngine, RunCall, RunTool};
    use std::time::Duration;

    #[tokio::test(start_paused = true)]
    async fn tool_discovery_is_immediate_when_disabled_and_bounded_while_starting() {
        let engine = NeoEngine::disabled("test engine");
        let began = tokio::time::Instant::now();
        assert!(engine.tool().await.description.contains("test engine"));
        assert_eq!(began.elapsed(), Duration::ZERO);
        engine.inner.state.send_replace(EngineState::Starting);
        let _ = engine.tool().await;
        assert_eq!(began.elapsed(), Duration::from_secs(5));
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_returns_the_sdk_definition_as_soon_as_it_is_ready() -> anyhow::Result<()> {
        let engine = NeoEngine::disabled("test engine");
        engine.inner.state.send_replace(EngineState::Starting);
        let waiting_engine = engine.clone();
        let waiting = tokio::spawn(async move { waiting_engine.tool().await });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        let mut tool = RunTool::fallback();
        tool.description = "SDK definition".to_owned();
        engine
            .inner
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tool = Some(tool);
        engine.inner.state.send_replace(EngineState::Ready);
        assert_eq!(waiting.await?.description, "SDK definition");
        Ok(())
    }

    #[tokio::test]
    async fn stopped_engine_refuses_runs_even_with_a_stale_ready_snapshot() -> anyhow::Result<()> {
        let engine = NeoEngine::for_test_endpoint(9)?;
        engine.inner.stop.cancel();
        let result = engine
            .run(RunCall {
                args: serde_json::from_value(serde_json::json!({"code": "return 1"}))?,
                task: None,
                host_data: serde_json::Value::Null,
            })
            .await;
        let Err(error) = result else {
            anyhow::bail!("a stopped engine accepted a run");
        };
        assert!(error.reason.contains("shut down"));
        Ok(())
    }
}
