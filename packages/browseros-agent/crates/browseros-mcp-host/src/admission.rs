//! Caller admission: per-request policy a host attaches to MCP traffic before rmcp sees it.
//!
//! claw admits nobody (every caller is an anonymous MCP client with claw's session model).
//! The BrowserOS server admits its own agents: each loopback request carries a lease the
//! host resolves to an [`AdmittedCaller`] — a session pre-bound to the conversation, a
//! read-only catalog for chat mode, structured content, the window to open tabs in, and
//! which of the user's tabs are lent to the run.
//!
//! ```text
//!  HTTP /mcp ─► Admission::admit(parts) ─► request extensions ─┐
//!  stdio/duplex ─► HostMcpService::with_caller(caller) ────────┤
//!                                                             ▼
//!          HostMcpService: session (bound spec | transport | handle), tools/list filter
//!                                                             ▼
//!          dispatch: read-only refusal → guards → execute(defaults, grants) → effects
//!                    (auto_group, lent pages) → observers → wire (structured opt-in)
//! ```
//!
//! The same caller value means the same thing on every transport, so a lease admitted over
//! a stateful SSE session and over stateless JSON lands in one bound session.

use crate::{identity::ClientInfo, ids::SessionId, mcp::dispatch::ToolCall, seams::LocalToolCall};
use browseros_core::WindowId;
use browseros_mcp::{OutputFileAccess, ToolDef, ToolResult};
use futures_util::future::BoxFuture;
use rmcp::model::{CallToolResult, Tool, ToolAnnotations};
use serde_json::{Value, json};
use std::{any::Any, collections::BTreeSet, fmt, sync::Arc};

/// Policy for one admitted caller. Absent means claw's behaviour, unchanged.
#[derive(Clone)]
pub struct AdmittedCaller {
    /// Use this pre-bound session instead of the transport session or `session` handle.
    /// Bound callers see no `session`/`agentName` arguments and no handle in results.
    pub session: Option<Arc<SessionSpec>>,
    /// Read-only catalog in `tools/list` (`tabs` limited to list/active, plus tools
    /// annotated read-only) and a refusal for anything else, checked before the guards.
    pub read_only: bool,
    /// Keep `structuredContent` on every result, not only tools with an output schema.
    pub structured_content: bool,
    /// Window a new tab opens in when the call names none.
    pub default_window_id: Option<WindowId>,
    /// Output-file grants scoped to this caller; `None` uses the transport session's own.
    pub output_files: Option<OutputFileAccess>,
    /// Pages lent to the session (the user's active side-panel tab): acting on one is not
    /// "someone else's tab", so no ownership notice is appended for it.
    pub lent_pages: BTreeSet<u32>,
    /// Whether the session groups the tabs it opens. Off for scheduled runs, whose host
    /// owns placement; the session then never creates, joins or retitles a group.
    pub auto_group: bool,
    /// Opaque host state for the host's own guards, effects and local tools (the
    /// BrowserOS server's lease). The host layer never reads it.
    pub host_data: Option<Arc<dyn Any + Send + Sync>>,
}

impl Default for AdmittedCaller {
    fn default() -> Self {
        Self {
            session: None,
            read_only: false,
            structured_content: false,
            default_window_id: None,
            output_files: None,
            lent_pages: BTreeSet::new(),
            auto_group: true,
            host_data: None,
        }
    }
}

impl fmt::Debug for AdmittedCaller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdmittedCaller")
            .field("session", &self.session)
            .field("read_only", &self.read_only)
            .field("structured_content", &self.structured_content)
            .field("default_window_id", &self.default_window_id)
            .field("lent_pages", &self.lent_pages)
            .field("auto_group", &self.auto_group)
            .finish_non_exhaustive()
    }
}

impl AdmittedCaller {
    /// The caller an admission layer attached to this request, if any. rmcp forwards the
    /// HTTP request parts (extensions included) into every handler's context.
    #[must_use]
    pub fn from_extensions(extensions: &rmcp::model::Extensions) -> Option<Self> {
        extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<Self>())
            .cloned()
    }

    /// `host_data` downcast to the host's own type.
    #[must_use]
    pub fn host_data<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.host_data.as_deref()?.downcast_ref::<T>()
    }
}

/// A session the host binds to one of its own keys (the BrowserOS server: one per
/// conversation). Binding is idempotent: every request carrying the same spec lands in the
/// same session, and a session that ended (idle sweep, `end_session`) is re-minted under
/// the same id and conversation, keeping its tab ownership, groups and browser target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSpec {
    /// Stable host key; the session id and conversation key derive from it.
    pub key: String,
    /// Who the session is attributed to; its slug prefixes the conversation key and
    /// the tab-group title, and its name is the analytics client.
    pub client: ClientInfo,
    /// Task label, shown as the `<client>/<label>` group title like `name_session` sets.
    /// `None` keeps the session's current label.
    pub label: Option<String>,
}

impl SessionSpec {
    /// The deterministic session id every binding of this spec uses. Stable across
    /// re-mints and restarts, so persisted browser-target bindings follow the session.
    #[must_use]
    pub fn session_id(&self) -> SessionId {
        SessionId::new(format!("bound-{}", self.key))
    }
}

/// Resolves the caller for one HTTP request to `/mcp`, before rmcp parses it.
pub trait Admission: Send + Sync {
    /// `Ok(None)` admits the request with claw's default behaviour; `Ok(Some)` attaches the
    /// caller; `Err(response)` answers without reaching MCP (the BrowserOS server: 401 with
    /// JSON-RPC `-32001` for an unknown lease).
    fn admit(
        &self,
        request: &axum::http::request::Parts,
    ) -> Result<Option<AdmittedCaller>, Box<axum::response::Response>>;
}

/// Tools that appear and disappear at runtime (the BrowserOS server's Klavis/Strata
/// connectors). Listed after the catalog and static local tools; called like local tools,
/// after session admission and outside the catalog pipeline, so each provider enforces its
/// own policy (read-only, run guard) from the caller it is given.
pub trait LocalToolProvider: Send + Sync {
    /// The tools this provider serves to `caller` right now. Called on every `tools/list`
    /// and to route each `tools/call`, so implementations should answer from a cache.
    fn tools<'a>(&'a self, caller: Option<&'a AdmittedCaller>) -> BoxFuture<'a, Vec<Tool>>;

    /// Runs `name`, one of the tools `tools` returned for this caller.
    fn call<'a>(&'a self, name: &'a str, call: LocalToolCall<'a>) -> BoxFuture<'a, CallToolResult>;
}

/// The read-only catalog's `tabs`: inspection only, as the TS server published it.
#[must_use]
pub(crate) fn read_only_tabs_tool() -> Tool {
    let Value::Object(input_schema) = json!({
        "type": "object",
        "properties": {
            "action": { "type": "string", "enum": ["list", "active"], "default": "list" }
        },
        "additionalProperties": false
    }) else {
        unreachable!();
    };
    Tool::new(
        "tabs",
        "Inspect browser tabs. `list` returns every open page and `active` returns the front page. Read-only mode cannot open or close tabs.",
        Arc::new(input_schema),
    )
    .with_annotations(ToolAnnotations::new().read_only(true).destructive(false))
}

pub(crate) fn is_read_only_tool(annotations: Option<&ToolAnnotations>) -> bool {
    annotations.is_some_and(|annotations| annotations.read_only_hint == Some(true))
}

/// Whether a read-only caller may list this catalog tool (`tabs` is listed rewritten).
pub(crate) fn listed_read_only(tool: &ToolDef) -> bool {
    tool.name == "tabs" || is_read_only_tool(tool.annotations.as_ref())
}

/// Refuses what a read-only caller may not run. Checked before the guards, so it holds
/// even for a client that calls a tool its read-only list never offered.
pub(crate) fn read_only_refusal(call: &ToolCall) -> Option<ToolResult> {
    if !call.caller.as_ref().is_some_and(|caller| caller.read_only) {
        return None;
    }
    let tool = call.tool();
    if tool.name != "tabs" {
        return (!is_read_only_tool(tool.annotations.as_ref()))
            .then(|| read_only_unavailable(tool.name));
    }
    let action = call
        .raw_args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("list");
    (action != "list" && action != "active").then(|| {
        ToolResult::error("tabs: read-only mode only supports action=\"list\" or \"active\".")
    })
}

pub(crate) fn read_only_unavailable(tool_name: &str) -> ToolResult {
    ToolResult::error(format!("{tool_name}: unavailable in read-only mode."))
}
