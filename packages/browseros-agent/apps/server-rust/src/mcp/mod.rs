//! server-rust's adapters at `browseros-mcp-host`'s seams (P15, after H1/H2 land): tool leases
//! (`X-BrowserOS-Internal-Lease`, minted per conversation for both engines' loopback `/mcp`
//! clients), admission (`?read_only=1`, `?structured=1`), the run guard, the side-panel effect
//! and branding. Until the host crate exists S0 keeps only the host-independent lease registry
//! shape; P15 adds the host-typed constructors (`HostCtx`, `AdmittedCaller`, guards/effects).

use crate::chat::{ConversationRuns, request::BrowserContext};
use std::{path::Path, sync::Arc};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseSource {
    Chat,
    Acp,
}

pub struct LeaseSpec {
    pub conversation_id: Uuid,
    pub read_only: bool,
    pub source: LeaseSource,
    /// `browseros` | `claude` | `codex` | `custom`; the session's tab-group prefix.
    pub client_slug: &'static str,
    pub task_label: String,
    /// False for scheduled runs (their pages are not grouped).
    pub auto_group: bool,
    pub browser_context: Option<BrowserContext>,
}

/// `?read_only=1`, `?structured=1` on `/mcp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct McpQuery {
    pub read_only: bool,
    pub structured: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid or expired BrowserOS tool lease")]
pub struct InvalidLease;

#[derive(Debug, thiserror::Error)]
#[error("failed to mint tool lease: {0}")]
pub struct LeaseError(pub String);

/// A minted lease: the bearer token a loopback MCP client sends, bound to one conversation's
/// host session. Revoked when the conversation is forgotten.
#[derive(Clone)]
pub struct ToolLease {
    token: String,
}

impl ToolLease {
    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn update_browser_context(&self, ctx: Option<&BrowserContext>) {
        let _ = ctx;
    }

    /// Which tool-output files this lease's `filesystem_read` may open.
    pub fn output_grants(&self) -> Arc<dyn browseros_agent::tools::OutputGrants> {
        Arc::new(NoGrants)
    }
}

struct NoGrants;

impl browseros_agent::tools::OutputGrants for NoGrants {
    fn allows(&self, path: &Path) -> bool {
        let _ = path;
        false
    }
}

/// token → lease state; conversation → host session.
pub struct ToolLeases {
    runs: Arc<ConversationRuns>,
}

impl ToolLeases {
    pub fn new(runs: Arc<ConversationRuns>) -> Arc<Self> {
        Arc::new(Self { runs })
    }

    /// Binds the conversation's host session and returns its lease.
    pub async fn mint(&self, spec: LeaseSpec) -> Result<ToolLease, LeaseError> {
        let _ = (spec, &self.runs);
        Err(LeaseError(
            "tool leases are not implemented yet in server-rust".to_owned(),
        ))
    }

    pub fn revoke(&self, lease: &ToolLease) {
        let _ = lease;
    }
}
