//! Tool leases: the opaque capability a conversation's agent carries on its loopback `/mcp`
//! client (`X-BrowserOS-Internal-Lease`), and the admission that turns one into the host's
//! per-request [`AdmittedCaller`]. Port of the lease half of TS `BrowserMcpModule`.
//!
//! A lease stores who is calling (conversation, read-only, client, task label, browser
//! context), never run authority: every dispatch asks the run registry whether the
//! conversation is running right now, so a lease minted for one turn stays valid across
//! turns and a stopped conversation's agent is refused until its next run starts.

use super::runs::RunRegistry;
use crate::chat::request::BrowserContext;
use browseros_core::WindowId;
use browseros_mcp::{OutputFileAccess, output_file::create_browser_output_file_access};
use browseros_mcp_host::{Admission, AdmittedCaller, SessionSpec, identity::ClientInfo};
use http::{StatusCode, request::Parts};
use serde_json::json;
use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
    sync::{Arc, Mutex, PoisonError},
};
use uuid::Uuid;

/// The header the engines' MCP clients send (`S/lib/browser-tool-lease.ts:2`).
pub const LEASE_HEADER: &str = "x-browseros-internal-lease";

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

impl McpQuery {
    /// Like `URLSearchParams.get(key) === '1'`: the first occurrence of each key decides.
    pub fn parse(query: Option<&str>) -> Self {
        let first = |key: &str| {
            query.is_some_and(|query| {
                url::form_urlencoded::parse(query.as_bytes())
                    .find(|(name, _)| name == key)
                    .is_some_and(|(_, value)| value == "1")
            })
        };
        Self {
            read_only: first("read_only"),
            structured: first("structured"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid or expired BrowserOS tool lease")]
pub struct InvalidLease;

#[derive(Debug, thiserror::Error)]
#[error("failed to mint tool lease: {0}")]
pub struct LeaseError(pub String);

/// What a lease caller's dispatch can see of its lease: the host hands it to server-rust's
/// guard, side-panel effect and connector gate through `AdmittedCaller::host_data`.
#[derive(Debug, Clone)]
pub struct LeaseScope {
    conversation_id: Uuid,
    source: LeaseSource,
    enabled_mcp_servers: Vec<String>,
}

impl LeaseScope {
    pub fn conversation_id(&self) -> Uuid {
        self.conversation_id
    }

    pub fn source(&self) -> LeaseSource {
        self.source
    }

    /// Managed connectors the user enabled for this conversation (`enabledMcpServers`).
    pub fn enabled_mcp_servers(&self) -> &[String] {
        &self.enabled_mcp_servers
    }
}

/// One minted lease. Immutable except for the browser context, which the engines refresh
/// in place every turn (the active window and tab move; re-minting would restart ACP agents).
struct LeaseState {
    conversation_id: Uuid,
    read_only: bool,
    source: LeaseSource,
    auto_group: bool,
    /// Every lease of a conversation binds the same host session (`bound-<conversation>`),
    /// so its tabs, group and browser target survive lease re-mints and server-side turns.
    session: Arc<SessionSpec>,
    output_files: OutputFileAccess,
    browser_context: Mutex<Option<BrowserContext>>,
}

impl LeaseState {
    fn caller(&self, query: McpQuery) -> AdmittedCaller {
        let context = self
            .browser_context
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let context = context.as_ref();
        AdmittedCaller {
            session: Some(self.session.clone()),
            // The query may further restrict a caller but can never weaken a lease.
            read_only: self.read_only || query.read_only,
            structured_content: query.structured,
            default_window_id: context
                .and_then(|context| context.window_id)
                .and_then(integral)
                .map(WindowId),
            output_files: Some(self.output_files.clone()),
            lent_pages: context.map(lent_pages).unwrap_or_default(),
            auto_group: self.auto_group,
            host_data: Some(Arc::new(LeaseScope {
                conversation_id: self.conversation_id,
                source: self.source,
                enabled_mcp_servers: context
                    .and_then(|context| context.enabled_mcp_servers.clone())
                    .unwrap_or_default(),
            })),
        }
    }
}

/// The pages the user handed the conversation (its active tab and selected tabs). They are
/// the user's, yet acting on them is the point of the request, so the host skips its
/// "someone else's tab" notice for them. Only ids the engine already resolved count.
fn lent_pages(context: &BrowserContext) -> BTreeSet<u32> {
    context
        .active_tab
        .iter()
        .chain(context.selected_tabs.iter().flatten())
        .filter_map(|tab| tab.page_id.and_then(integral))
        .filter_map(|page| u32::try_from(page).ok())
        .filter(|page| *page >= 1)
        .collect()
}

/// Contract numbers arrive as JSON doubles; only exact integers are ids.
fn integral(value: f64) -> Option<i64> {
    (value.is_finite() && value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0)
        .then_some(value as i64)
}

/// A minted lease: the bearer token a loopback MCP client sends, bound to one conversation's
/// host session. Revoked when the engine drops the conversation's agent.
#[derive(Clone)]
pub struct ToolLease {
    token: String,
    state: Arc<LeaseState>,
}

impl ToolLease {
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Refreshes the context later requests on this lease see (window, lent tabs, enabled
    /// connectors). A revoked lease admits nothing, so updating one is harmless.
    pub fn update_browser_context(&self, ctx: Option<&BrowserContext>) {
        *self
            .state
            .browser_context
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = ctx.cloned();
    }

    /// Which tool-output files this lease's `filesystem_read` may open: those the
    /// conversation's own browser tools wrote (pdf, large reads, …).
    pub fn output_grants(&self) -> Arc<dyn browseros_agent::tools::OutputGrants> {
        Arc::new(OutputGrants(self.state.output_files.clone()))
    }
}

struct OutputGrants(OutputFileAccess);

impl browseros_agent::tools::OutputGrants for OutputGrants {
    fn allows(&self, path: &Path) -> bool {
        // The grant set sits behind the catalog's tokio mutex, held only for a set insert;
        // a synchronous reader retries briefly instead of blocking a runtime thread.
        for _ in 0..64 {
            if let Ok(granted) = self.0.try_lock() {
                return granted.contains(path);
            }
            std::thread::yield_now();
        }
        false
    }
}

/// token → lease; conversation → output grants. Shared by both engines (minting) and the
/// `/mcp` admission hook (resolving), and the owner of the run registry `/mcp` policy reads.
pub struct ToolLeases {
    runs: Arc<dyn RunRegistry>,
    inner: Mutex<Registry>,
}

#[derive(Default)]
struct Registry {
    leases: HashMap<String, Arc<LeaseState>>,
    /// Grants outlive any one lease so a re-minted lease (an ACP fingerprint change) still
    /// reads files earlier turns wrote, as TS reused `outputFileAccess`. One small set per
    /// conversation leased in this process.
    outputs: HashMap<Uuid, OutputFileAccess>,
}

impl ToolLeases {
    pub fn new(runs: Arc<crate::chat::ConversationRuns>) -> Arc<Self> {
        Self::with_registry(runs)
    }

    /// Leases over any run registry (tests drive runs without the chat layer).
    pub fn with_registry(runs: Arc<dyn RunRegistry>) -> Arc<Self> {
        Arc::new(Self {
            runs,
            inner: Mutex::default(),
        })
    }

    pub fn runs(&self) -> &Arc<dyn RunRegistry> {
        &self.runs
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Mints a lease for the conversation's session. The host session itself is bound
    /// lazily by the first admitted request, so minting never touches the browser.
    pub async fn mint(&self, spec: LeaseSpec) -> Result<ToolLease, LeaseError> {
        let token = Uuid::new_v4().to_string();
        let mut registry = self.registry();
        let output_files = registry
            .outputs
            .entry(spec.conversation_id)
            .or_insert_with(create_browser_output_file_access)
            .clone();
        let state = Arc::new(LeaseState {
            conversation_id: spec.conversation_id,
            read_only: spec.read_only,
            source: spec.source,
            auto_group: spec.auto_group,
            session: Arc::new(session_spec(
                spec.conversation_id,
                spec.client_slug,
                spec.task_label,
            )),
            output_files,
            browser_context: Mutex::new(spec.browser_context),
        });
        registry.leases.insert(token.clone(), state.clone());
        Ok(ToolLease { token, state })
    }

    /// Stops new requests from presenting this lease. Requests already admitted keep
    /// their captured caller; the run guard remains their execution authority.
    pub fn revoke(&self, lease: &ToolLease) {
        let mut registry = self.registry();
        if registry
            .leases
            .get(&lease.token)
            .is_some_and(|state| Arc::ptr_eq(state, &lease.state))
        {
            registry.leases.remove(&lease.token);
        }
    }

    /// The caller one `/mcp` request is admitted as. A blank header is no lease (TS trims);
    /// an unknown token is refused. Without a lease, only the query flags apply, to an
    /// anonymous client with the host's own session handling.
    pub fn resolve(
        &self,
        token: Option<&str>,
        query: McpQuery,
    ) -> Result<Option<AdmittedCaller>, InvalidLease> {
        match token.map(str::trim).filter(|token| !token.is_empty()) {
            Some(token) => {
                let state = self.registry().leases.get(token).cloned();
                state.map(|state| Some(state.caller(query))).ok_or(InvalidLease)
            }
            None if query.read_only || query.structured => Ok(Some(AdmittedCaller {
                read_only: query.read_only,
                structured_content: query.structured,
                ..AdmittedCaller::default()
            })),
            None => Ok(None),
        }
    }
}

/// The host session spec every lease of `conversation_id` binds. Its key is the
/// conversation id, so the session id is `bound-<conversation_id>`.
fn session_spec(conversation_id: Uuid, client_slug: &str, task_label: String) -> SessionSpec {
    SessionSpec {
        key: conversation_id.to_string(),
        client: ClientInfo {
            name: client_slug.to_owned(),
            version: crate::VERSION.to_owned(),
            title: None,
        },
        label: Some(task_label),
    }
}

/// The conversation a bound host session belongs to (inverse of [`session_spec`]).
pub fn conversation_of_session(session_id: &str) -> Option<Uuid> {
    let key = session_id.strip_prefix("bound-")?;
    let conversation_id = Uuid::parse_str(key).ok()?;
    let spec = SessionSpec {
        key: conversation_id.to_string(),
        client: ClientInfo {
            name: String::new(),
            version: String::new(),
            title: None,
        },
        label: None,
    };
    (spec.session_id().as_str() == session_id).then_some(conversation_id)
}

impl Admission for ToolLeases {
    fn admit(
        &self,
        request: &Parts,
    ) -> Result<Option<AdmittedCaller>, Box<axum::response::Response>> {
        // Fetch `Headers.get` joins repeated headers with ", ", which no token matches.
        let token = request
            .headers
            .get_all(LEASE_HEADER)
            .iter()
            .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
            .reduce(|joined, next| format!("{joined}, {next}"));
        let query = McpQuery::parse(request.uri.query());
        self.resolve(token.as_deref(), query).map_err(|error| {
            tracing::warn!(
                read_only = query.read_only,
                structured = query.structured,
                "MCP request rejected for invalid lease"
            );
            Box::new(invalid_lease_response(&error))
        })
    }
}

/// 401 with JSON-RPC `-32001` and a null id, before MCP parses anything (TS `mcp.ts:179-192`).
fn invalid_lease_response(error: &InvalidLease) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(json!({
            "jsonrpc": "2.0",
            "error": { "code": -32001, "message": error.to_string() },
            "id": null,
        })),
    )
        .into_response()
}
