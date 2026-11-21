//! `browser`: the browser as tools, from neo-sdk's one tool `run`. Which engine sits behind
//! it, or none, is configuration: every caller compiles and behaves the same on Windows,
//! where the engine cannot be built. Also serves the private MCP endpoint a coding agent
//! uses, which adds the task id and read-only access a child process cannot send itself,
//! and forwards into the same `call`.
//!
//! ```text
//!   agent (own loop) ──Browser::call──┐
//!                                     ├──▶ engine child (`<exe> __neo-engine`), JSON-RPC on loopback
//!   coding agent ──POST /mcp/{task}───┘    or Off: no tools, and the prompt says so
//! ```
//!
//! The engine's JSON, MCP and picture blobs never leave this module (design rule 4).

mod mcp;
#[cfg(unix)]
mod neo;
#[cfg(feature = "test-support")]
mod scripted;

#[cfg(feature = "test-support")]
pub use scripted::{RecordedCall, ScriptedBrowser, ScriptedReply};

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::turn::Mode;

/// Which engine `Browser::start` runs. `app` chooses: `Neo` on Unix, `Off` on Windows, and
/// `BROWSEROS_BROWSER=off` forces `Off` anywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserConfig {
    /// neo-sdk's server, started as a second copy of this executable (`<exe> __neo-engine`).
    Neo {
        cdp_port: u16,
        state_dir: std::path::PathBuf,
        execution_dir: std::path::PathBuf,
        browseros_version: Option<String>,
    },
    /// Windows, `BROWSEROS_BROWSER=off`, or a failed start. The reason is told to the model.
    Off { reason: &'static str },
}

/// A tool as the engine describes it (`tools/list`, cached): today exactly one, `run`.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// What one turn may do. `task` is the conversation id and becomes
/// `_meta["com.browseros.neo/run"].taskId`; `Read` (chat mode) becomes `access: "read"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub task: String,
    pub access: Access,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Full,
}

impl Mode {
    /// Chat mode may read pages and never act on them.
    pub fn access(self) -> Access {
        match self {
            Mode::Chat => Access::Read,
            Mode::Agent => Access::Full,
        }
    }
}

/// The result of one call. `text` is what a model reads: the returned value as JSON with
/// each picture replaced by a placeholder, fenced as untrusted page content. A failed
/// script is a `ToolOutput` with `is_error`, never an `Err`.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub text: String,
    pub pictures: Vec<Picture>,
    pub is_error: bool,
}

/// A picture from a tool result: a model sees it as a user message, the panel never does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

impl ToolOutput {
    /// What the panel, the run log and the stored row get: the text, never the bytes.
    pub fn for_panel(&self) -> Result<serde_json::Value, String> {
        if self.is_error {
            Err(self.text.clone())
        } else {
            Ok(serde_json::Value::String(self.text.clone()))
        }
    }
}

/// Keeps `/mcp/{task}` answering while held. Dropping it closes the endpoint for that task
/// and cancels every browser call still in flight for it: the coding agent's half of stop.
#[derive(Debug)]
pub struct Armed {
    url: String,
    cancel: CancellationToken,
}

impl Armed {
    /// The address to put in the agent's MCP config. It is the same for the life of a
    /// conversation, because a live acpx session refuses a changed MCP config.
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for Armed {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// The browser, shared by every turn. Cheap to clone.
#[derive(Clone)]
pub struct Browser {
    inner: Arc<Inner>,
}

struct Inner {
    engine: Engine,
}

/// Chunk `browser` adds the engine child as a `#[cfg(unix)]` variant.
enum Engine {
    Off {
        reason: &'static str,
    },
    #[cfg(feature = "test-support")]
    Scripted(scripted::ScriptedBrowser),
}

/// The browser paragraph of the system prompt where there is no engine (design section 3.5).
const NO_BROWSER_GUIDANCE: &str = "This build cannot see or control the browser. Answer from \
    the conversation and from the page context the user's message carries (the active tab's \
    title and address, selected text). Say so if the user asks you to act on a page.";

impl Browser {
    /// Never fails and returns at once: the server serves while the engine starts, and an
    /// engine that cannot start is `Off`, so chat still works.
    pub async fn start(
        config: BrowserConfig,
        _server_port: u16,
        _shutdown: CancellationToken,
    ) -> Self {
        let reason = match config {
            BrowserConfig::Off { reason } => reason,
            BrowserConfig::Neo { .. } => "the browser engine is not built into this server yet",
        };
        Self::from_engine(Engine::Off { reason })
    }

    fn from_engine(engine: Engine) -> Self {
        Self {
            inner: Arc::new(Inner { engine }),
        }
    }

    /// Empty when there is no engine. With an engine: the cached `run` definition once the
    /// engine answered `tools/list`, and a built-in copy before that, so the first turn after a
    /// cold start still has the tool.
    pub fn tools(&self) -> Vec<Tool> {
        match &self.inner.engine {
            Engine::Off { .. } => Vec::new(),
            #[cfg(feature = "test-support")]
            Engine::Scripted(scripted) => scripted.tools(),
        }
    }

    /// The paragraph of the system prompt about the browser: how to reach the user's active
    /// tab by its id, or that this build cannot see or control the browser.
    pub fn guidance(&self) -> &str {
        match &self.inner.engine {
            Engine::Off { .. } => NO_BROWSER_GUIDANCE,
            #[cfg(feature = "test-support")]
            Engine::Scripted(_) => scripted::GUIDANCE,
        }
    }

    /// Runs one tool call under `scope`. Cancel drops the request to the engine, which
    /// cancels the script (no `requestId` is sent, so there is no orphan grace).
    pub async fn call(
        &self,
        scope: &Scope,
        tool: &str,
        args: serde_json::Value,
        cancel: &CancellationToken,
    ) -> ToolOutput {
        match &self.inner.engine {
            Engine::Off { reason } => {
                let _ = (scope, tool, args, cancel);
                ToolOutput {
                    text: format!("The browser is not available: {reason}."),
                    pictures: Vec::new(),
                    is_error: true,
                }
            }
            #[cfg(feature = "test-support")]
            Engine::Scripted(scripted) => scripted.call(scope, tool, args, cancel).await,
        }
    }

    /// `None` when there is no engine: the coding agent then runs with no browser tools.
    pub fn arm(&self, scope: Scope) -> Option<Armed> {
        match &self.inner.engine {
            Engine::Off { .. } => {
                let _ = scope;
                None
            }
            #[cfg(feature = "test-support")]
            Engine::Scripted(scripted) => Some(scripted.arm(scope)),
        }
    }

    /// The private MCP endpoint, mounted by `http` beside the contract routes.
    pub fn mcp_router(&self) -> axum::Router {
        mcp::router(self.clone())
    }

    /// Tab ids `task` opened since the last take. Filled inside `call`, before it returns, by
    /// comparing the engine's tab census (host API `GET /v1/tabs`, which names the task that
    /// holds each tab) with the tabs already seen for that task. The reply's `_meta.tasks`
    /// cannot be used: it omits the implicit host task and lists tabs, not new tabs.
    /// Always empty when there is no engine.
    pub fn take_opened_tabs(&self, task: &str) -> Vec<i64> {
        match &self.inner.engine {
            Engine::Off { .. } => {
                let _ = task;
                Vec::new()
            }
            #[cfg(feature = "test-support")]
            Engine::Scripted(scripted) => scripted.take_opened_tabs(task),
        }
    }

    /// Whether the engine is linked to the browser: the engine's `GET /system/ready` (503 while
    /// its browser link is starting) or `browser.connected` in its health body. Not "the child
    /// process answers": the engine's health is 200 as soon as it serves. `cdpConnected` in
    /// `/system/health` and `/status`.
    pub fn connected(&self) -> bool {
        match &self.inner.engine {
            Engine::Off { .. } => false,
            #[cfg(feature = "test-support")]
            Engine::Scripted(_) => true,
        }
    }
}

/// The body of `<exe> __neo-engine --config <file>`: exactly neo-server's own `main`. Runs in
/// the engine child process, never in the server's own.
#[cfg(unix)]
pub fn engine_main(config: &std::path::Path) -> std::process::ExitCode {
    neo::engine_main(config)
}
