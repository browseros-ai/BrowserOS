//! server-rust's `/mcp`: the shared `browseros-mcp-host` (claw's sessions, ownership, tab
//! groups and dispatch over the `browseros-mcp` catalog) with this server's policy plugged
//! into the host's seams. Both engines reach browser tools here over loopback HTTP, carrying
//! a lease; external MCP clients (Claude Code, Cursor, …) come without one.
//!
//! ```text
//!  BrowserOsEngine / AcpEngine ── ToolLeases::mint ──► token
//!        │  POST /mcp[?read_only=1][&structured=1]   X-BrowserOS-Internal-Lease: token
//!        ▼
//!  http::security::ingress     Origin 403, Sec-Fetch-Site 403, in-flight count for /status
//!        ▼
//!  endpoint::post              lease → AdmittedCaller (401 -32001); SDK 413/406/415/400
//!        ▼
//!  rmcp, stateless JSON ─► HostMcpService, session `bound-<conversation>` for a lease
//!        ├─ catalog tool: read-only refusal → navigate scheme → browser connected
//!        │                → ConversationRunning → execute → effects (claims → PanelTabs
//!        │                → runs.associate_tabs) → wire (structured opt-in)
//!        └─ connector tool: Klavis provider, asking `connector_scope` for the lease's
//!                           run refusal and selected connectors (then refusing read-only)
//! ```
//!
//! Telemetry follows TS's `/mcp`: `mcp.request` per admitted POST, `tool_executed` per catalog
//! dispatch (the [`telemetry`] observer) and per connector call, and a `/mcp`-scoped Sentry
//! report when the transport fails internally.
//!
//! The host also owns the browser link: `McpServer::start` begins CDP reattach (non-fatal,
//! retried forever, unlike TS's exit), `/system/health` reports its live state, and the
//! chat layer reaches the browser through it: [`HostPages`] opens scheduled runs'
//! background pages and [`HostTabs`] resolves the app's tab ids to page ids.

mod branding;
mod connectors;
mod endpoint;
mod leases;
mod pages;
mod panel;
mod runs;
mod telemetry;

pub use branding::{branding, instructions};
pub use connectors::connector_scope;
pub use endpoint::{MAX_BODY_BYTES, is_json_content_type};
pub use leases::{
    InvalidLease, LeaseError, LeaseScope, LeaseSource, LeaseSpec, McpQuery, ToolLease, ToolLeases,
    conversation_of_session,
};
pub use pages::{HostPages, HostTabs};
pub use runs::{NO_ACTIVE_RUN, RunRegistry};

use crate::telemetry::metrics::Metrics;
use axum::{extract::Request, response::Response};
use browseros_core::NativeCapabilities;
use browseros_mcp_host::{
    HostCore, HostCoreConfig, HostCtx, HostError, LocalToolProvider, McpRouteConfig,
    streamable_http_service,
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// A conversation's host session ends after this long without a tool call; its next call
/// re-mints it under the same id with its tabs and group (claw's default).
const SESSION_IDLE: Duration = Duration::from_secs(30 * 60);
/// How long an ended session's collapsed tab group is kept before it is closed.
const SESSION_RETENTION: Duration = Duration::from_secs(60 * 60);
const SESSION_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

pub struct McpConfig {
    pub cdp_port: u16,
    /// Owns the host's persisted browser-target bindings and code-mode helpers.
    pub data_root: PathBuf,
    /// From the sidecar's `instance.browseros_version`; gates profile-targeting tools.
    pub native: NativeCapabilities,
}

/// The `/mcp` surface of one process: the shared host, the transport in front of it, and
/// the host's background loops (CDP reattach, idle-session sweep).
pub struct McpServer {
    host: Arc<HostCtx>,
    leases: Arc<ToolLeases>,
    transport: endpoint::Transport,
    metrics: Metrics,
    background: CancellationToken,
}

impl McpServer {
    /// Opens the host core over `config` and plugs in server-rust's policy. Nothing runs
    /// until [`McpServer::start`]. `connectors` is the Klavis tool provider, built over
    /// [`connector_scope`] so it applies this server's lease policy. `metrics` is the
    /// process's PostHog sink.
    pub async fn open(
        config: McpConfig,
        leases: Arc<ToolLeases>,
        connectors: Option<Arc<dyn LocalToolProvider>>,
        metrics: Metrics,
    ) -> Result<Self, HostError> {
        let core = HostCore::open(HostCoreConfig {
            cdp_port: config.cdp_port,
            data_root: config.data_root,
            native: config.native,
            session_idle: SESSION_IDLE,
            session_retention: SESSION_RETENTION,
            session_sweep_interval: SESSION_SWEEP_INTERVAL,
        })
        .await?;
        Ok(Self::on_core(core, leases, connectors, metrics))
    }

    /// [`McpServer::open`] over a core the caller built (tests attach a synthetic browser).
    pub fn on_core(
        core: HostCore,
        leases: Arc<ToolLeases>,
        connectors: Option<Arc<dyn LocalToolProvider>>,
        metrics: Metrics,
    ) -> Self {
        let runs = leases.runs().clone();
        let mut builder = HostCtx::builder(core)
            .branding(branding())
            .tab_journal(Arc::new(panel::PanelTabs(runs.clone())))
            .guard(Arc::new(runs::ConversationRunning(runs.clone())))
            .observer(Arc::new(telemetry::ToolTelemetry(metrics.clone())));
        if let Some(connectors) = connectors {
            builder = builder.tool_provider(connectors);
        }
        let host = builder.build();
        // TS's transport: every request stateless with single-JSON replies (its internal ACP
        // client depends on them), any Host (the server is LAN-reachable), no Origin check
        // here (ingress owns the Origin allowlist).
        let transport =
            streamable_http_service(host.clone(), McpRouteConfig::stateless_json().http);
        Self {
            host,
            leases,
            transport,
            metrics,
            background: CancellationToken::new(),
        }
    }

    pub fn host(&self) -> &Arc<HostCtx> {
        &self.host
    }

    /// Starts CDP reattach and the idle-session sweep, then waits for the first connection
    /// attempt so `/system/health` is truthful from the first request. A missing browser
    /// is not fatal: the loop keeps retrying and tools explain the outage to agents.
    pub async fn start(&self) {
        // The handles are detached: `stop` ends both loops (cancel + browser stop).
        drop(self.host.spawn_background(self.background.clone()));
        self.host.browser().wait_for_initial_attempt().await;
    }

    pub async fn cdp_connected(&self) -> bool {
        self.host.browser().link_status().await.connected
    }

    /// `POST /mcp`.
    pub async fn post(&self, request: Request) -> Response {
        endpoint::post(
            self.leases.as_ref(),
            &self.transport,
            &self.metrics,
            request,
        )
        .await
    }

    /// Stops reattaching and ends every live session (their groups are kept collapsed).
    pub async fn stop(&self) {
        self.background.cancel();
        if let Err(error) = self.host.shutdown().await {
            tracing::warn!(%error, "MCP host shutdown failed");
        }
    }
}
