//! Generic ACP v1 client: one supervised child process and SDK connection, with
//! session-scoped turns. The SDK stays private to this crate; callers share only
//! its versioned schema and control process lifetime explicitly through shutdown.

mod connection;
mod permissions;
mod process;
mod routing;
mod session;

use std::{collections::BTreeMap, path::PathBuf, process::ExitStatus, time::Duration};

pub use agent_client_protocol::schema::{ProtocolVersion, v1 as schema};
pub use connection::AgentConnection;
pub use permissions::{ApproveAll, PermissionDecision, PermissionPolicy, RejectAll};
pub use session::{Session, Turn, TurnCanceller, TurnEvent, TurnOutcome};

/// Child command and environment overlay. On Unix the child leads a new process
/// group, so shutdown also reaches descendants launched by wrappers such as npx.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LaunchSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
}

/// Connection identity and deadlines for control, shutdown, and pipe draining.
/// Independent phase budgets let hosts tolerate scheduler contention without
/// extending how long an explicitly abandoned connection may keep a child alive.
#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub client_name: String,
    pub client_version: String,
    /// A timed-out control retires the connection: its late side effects cannot
    /// safely be associated with a subsequent session or turn.
    pub control_timeout: Duration,
    pub stderr_tail_lines: usize,
    /// How long the child and its process group may take to exit after stdin
    /// closes on last-handle teardown or driver failure, before SIGTERM and then
    /// SIGKILL. Explicit shutdown can override it for that call.
    pub shutdown_grace: Duration,
    /// Time to drain a final prompt response after observing child exit.
    pub response_grace: Duration,
    /// Time to finish stderr capture after child exit, before publishing its
    /// report. Kept separate from stdout so a short prompt grace cannot lose diagnostics.
    pub stderr_grace: Duration,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            client_name: "acp-client".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
            control_timeout: Duration::from_secs(30),
            stderr_tail_lines: 20,
            shutdown_grace: Duration::from_secs(1),
            response_grace: Duration::from_secs(1),
            stderr_grace: Duration::from_secs(1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentFeatures {
    pub load_session: bool,
    pub resume_session: bool,
    pub close_session: bool,
    pub mcp_http: bool,
    pub mcp_sse: bool,
    pub prompt_image: bool,
    pub prompt_audio: bool,
}

/// Session creation/reattachment inputs; policy and desired controls belong to
/// the caller. Apply configuration first and mode second after every attachment.
#[derive(Debug, Clone)]
pub struct SessionParams {
    pub cwd: PathBuf,
    pub mcp_servers: Vec<schema::McpServer>,
    pub meta: Option<schema::Meta>,
}

#[derive(Debug, Clone)]
pub struct ExitReport {
    pub status: Option<ExitStatus>,
    pub stderr_tail: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("failed to spawn {program}: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("agent process exited ({status:?})")]
    Exited {
        status: Option<ExitStatus>,
        stderr_tail: Vec<String>,
    },
    #[error("{method} timed out after {after:?}")]
    Timeout {
        method: &'static str,
        after: Duration,
    },
    #[error("agent requires authentication: {message}")]
    AuthRequired {
        methods: Vec<String>,
        message: String,
    },
    #[error("agent error {code}: {message}")]
    Rpc {
        code: i32,
        message: String,
        data: Option<serde_json::Value>,
    },
    #[error("agent speaks ACP {0:?}; this client speaks v1")]
    ProtocolVersion(ProtocolVersion),
    #[error("agent does not support {0}")]
    Unsupported(&'static str),
    #[error("a prompt turn is already running on this session")]
    TurnInProgress,
    #[error("connection closed")]
    Closed,
}
