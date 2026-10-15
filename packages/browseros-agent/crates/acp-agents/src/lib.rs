//! BrowserOS's ACP agent runtime: everything BrowserOS-specific about driving Claude, Codex and
//! custom ACP agents, on top of the generic `acp-client`. It replaces npm `acpx`, `acp-probe`,
//! `@browseros/acpx-ai-provider` and the TS server's `lib/agents/{acp,host-acp}`.
//!
//! ```text
//!  apps/server-rust:  AcpEngine (POST /chat)                  POST /acpx/probe
//!                          │ TurnRequest                            │ ProbeRequest
//!                          ▼                                        ▼
//!  AcpRuntime::stream ── build_policy ── resolve_launch ──── probe_agent
//!      │ busy lock · restart fingerprint · idle close · resume recovery
//!      │ SessionStore (browseros.acp-session.v1, acpx.session.v1 import)
//!      ▼
//!  acp_client::{AgentConnection, Session, Turn} ── stdio JSON-RPC ── agent process
//!      │ session/update
//!      ▼
//!  UiTranslator ──► UiChunk stream (AI SDK v7 UI message stream, same wire as TS)
//! ```
//!
//! The crate never touches SQLite or HTTP: agent rows arrive parsed as [`AgentDefinition`], and the
//! display history arrives pre-flattened as a [`TurnPrompt`].

mod definition;
mod launch;
mod policy;
mod probe;
mod runtime;
mod store;
mod translate;

/// The ACP schema and error types callers build prompts from and match on, re-exported so the
/// server needs no direct `acp-client` dependency.
pub use acp_client::{AcpError, schema};
pub use definition::{AgentDefinition, AgentKind, CustomAgentConfig};
pub use launch::{
    AdapterPackage, CLAUDE_ADAPTER, CODEX_ADAPTER, HostEnvironment, LaunchError, LaunchSource,
    ResolvedLaunch, resolve_launch, split_command_line,
};
pub use policy::{
    AgentPolicy, BROWSEROS_ACP_INSTRUCTIONS, BROWSEROS_TOOL_LEASE_HEADER, CustomMcpServer,
    SessionControls, SessionKey, TurnContext, build_policy, ensure_workspace,
};
pub use probe::{
    DEFAULT_PROBE_TIMEOUT, ProbeErrorBody, ProbeModel, ProbeReasoning, ProbeRequest, ProbeResult,
    probe_agent,
};
pub use runtime::{
    AcpRuntime, AcpRuntimeConfig, DEFAULT_CONTROL_TIMEOUT, DEFAULT_IDLE_TIMEOUT,
    DEFAULT_TURN_TIMEOUT, HOST_STOP_REASON, RuntimeError, TurnPrompt, TurnRequest,
};
pub use store::{
    FileSessionStore, IMPORTED_LAUNCH_FINGERPRINT, SESSION_RECORD_SCHEMA, SessionRecord,
    SessionStore,
};
pub use translate::{ACP_FAILED_TO_RESPOND, TurnEnd, UiTranslator};
/// The output vocabulary, shared with the built-in engine.
pub type UiChunkStream = ui_message_stream::ChunkStream;
pub use ui_message_stream::{FinishReason, UiChunk};
