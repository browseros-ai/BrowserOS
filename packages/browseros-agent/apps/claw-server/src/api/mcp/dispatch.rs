use crate::{
    AppState,
    api::mcp::{effects, guards, helper_runtime, observers},
    identity::ClientIdentity,
    ids::{ConvoId, DispatchId, SessionId},
    services::sessions::Session,
};
use browseros_core::{BrowserSession, PageId, pages::PageInfo, target::BrowserView};
use browseros_mcp::{
    BrowserToolDefaults, BrowserToolOptions, OutputFileAccess, TargetStore, ToolCtx, ToolDef,
    ToolResult, execute_tool,
};
use futures_util::future::BoxFuture;
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, ContentBlock},
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Instant};
use tokio_util::sync::CancellationToken;
use tracing::warn;

const CANCELLATION_REASON: &str = "Operation cancelled by the User";
const CLIENT_CANCELLATION_ERROR: &str = "Request cancelled by client";
pub(crate) const ARBITRARY_SCRIPT_TOOLS: &[&str] = &["run", "evaluate"];
const DISPATCH_ERROR_TEXT_MAX: usize = 200;

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct ToolFlags {
    pub new_page: bool,
    pub close_page: bool,
    pub list_tabs: bool,
}

#[derive(Clone)]
pub struct ToolIdentity {
    pub session: Arc<Session>,
    pub agent: ClientIdentity,
    /// Per-conversation key; distinct from transport-session and profile ids.
    pub ownership_key: ConvoId,
    pub agent_label: String,
}

/// Creation-time identity and dispatch capability for detached grouping. The view
/// retains the original transport epoch even when the creating call was unbound.
#[derive(Clone)]
pub(crate) struct CreatedPageCapture {
    pub page: PageInfo,
    pub view: BrowserView,
}

#[derive(Clone)]
pub struct ToolCall {
    catalog: Arc<Vec<ToolDef>>,
    tool_index: usize,
    pub raw_args: Value,
    pub session_id: SessionId,
    pub identity: Option<ToolIdentity>,
    pub browser_session: Option<Arc<BrowserSession>>,
    /// Captured once at admission, shared by execution, run primitives and delayed effects.
    /// The global session above is reserved for registry/audit infrastructure.
    pub view: Option<BrowserView>,
    pub target_store: Arc<dyn TargetStore>,
    /// Pre-execution page incarnation retained for activity, close cleanup, and
    /// audit fallback even if execution changes or removes the live page.
    pub page_snapshot: Option<PageInfo>,
    pub started_at_ms: i64,
    /// Linked token passed into browser execution; session cancellation becomes a
    /// protocol error that skips effects.
    pub cancel: CancellationToken,
    /// Original transport/request token; client cancellation follows the same protocol path.
    pub client_cancel: CancellationToken,
    /// Original cockpit/operator token; cancellation becomes an audited in-band tool error.
    pub dispatch_cancel: CancellationToken,
    pub default_tab_group_id: Option<String>,
    pub flags: ToolFlags,
    pub state: AppState,
    pub dispatch_id: DispatchId,
    pub output_files: OutputFileAccess,
    /// Pages a `run` script acted on that belong to the user or another agent.
    ///
    /// `run` has no top-level `page` argument, so the ownership notice cannot be derived
    /// from its arguments the way it is for a granular tool. The script hook records what
    /// the script actually touched, and `effects::page_ownership_notice` reports it.
    pub foreign_pages: Arc<std::sync::Mutex<std::collections::BTreeMap<u32, String>>>,
    /// Shared with script hooks, including calls that fail or are cancelled
    /// before they can return runtime audit hints.
    pub sensitive_credentials: Arc<std::sync::atomic::AtomicBool>,
    /// Original snapshots and deep dispatch guards, captured before detached grouping.
    /// A reusable PageId must never let a later callback adopt its replacement tab.
    pub(crate) created_page_snapshots:
        Arc<tokio::sync::Mutex<std::collections::BTreeMap<u32, CreatedPageCapture>>>,
}

impl ToolCall {
    /// Builds the immutable context shared by guards, execution, and effects.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        catalog: Arc<Vec<ToolDef>>,
        tool_index: usize,
        raw_args: Value,
        session_id: SessionId,
        identity: Option<ToolIdentity>,
        browser_session: Option<Arc<BrowserSession>>,
        view: Option<BrowserView>,
        target_store: Arc<dyn TargetStore>,
        cancel: CancellationToken,
        client_cancel: CancellationToken,
        dispatch_cancel: CancellationToken,
        default_tab_group_id: Option<String>,
        state: AppState,
        output_files: OutputFileAccess,
    ) -> Self {
        let tool_name = catalog[tool_index].name;
        let flags = if tool_name == "tabs" {
            match raw_args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("list")
            {
                "new" => ToolFlags {
                    new_page: true,
                    ..ToolFlags::default()
                },
                "close" => ToolFlags {
                    close_page: true,
                    ..ToolFlags::default()
                },
                "list" => ToolFlags {
                    list_tabs: true,
                    ..ToolFlags::default()
                },
                _ => ToolFlags::default(),
            }
        } else {
            ToolFlags::default()
        };
        Self {
            // Conservatively classify common credential-bearing source before
            // target admission or execution. The bridge also marks dynamic calls.
            sensitive_credentials: Arc::new(std::sync::atomic::AtomicBool::new(
                tool_name == "credentials"
                    || tool_name == "run"
                        && raw_args
                            .get("code")
                            .and_then(Value::as_str)
                            .is_some_and(|source| {
                                let source = source.to_ascii_lowercase();
                                source.contains("credential") || source.contains("password")
                            }),
            )),
            foreign_pages: Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new())),
            created_page_snapshots: Arc::default(),
            catalog,
            tool_index,
            raw_args,
            session_id,
            identity,
            browser_session,
            view,
            target_store,
            page_snapshot: None,
            started_at_ms: crate::clock::now_epoch_ms(),
            cancel,
            client_cancel,
            dispatch_cancel,
            default_tab_group_id,
            flags,
            state,
            dispatch_id: DispatchId::new(),
            output_files,
        }
    }

    #[must_use]
    pub fn tool(&self) -> &ToolDef {
        &self.catalog[self.tool_index]
    }

    #[must_use]
    pub fn tool_named(&self, name: &str) -> Option<&ToolDef> {
        self.catalog.iter().find(|tool| tool.name == name)
    }

    /// Orders explicit group mutations with detached creation and lifecycle work.
    /// An owned guard can cross the script host seam without borrowing its caller;
    /// the captured BrowserView still supplies all identity/epoch admission.
    pub(crate) async fn acquire_group_operation_guard(
        &self,
        tool_name: &str,
        args: &Value,
    ) -> Result<Option<tokio::sync::OwnedMutexGuard<()>>, ()> {
        let Some(identity) = &self.identity else {
            return Ok(None);
        };
        if tool_name != "tab_groups"
            || !matches!(
                args.get("action").and_then(Value::as_str),
                Some("create" | "update" | "ungroup" | "move" | "close")
            )
        {
            return Ok(None);
        }
        let acquire = async {
            self.state
                .sessions
                .ownership()
                .group_operation_lock(&identity.ownership_key)
                .await
                .lock_owned()
                .await
        };
        // Cancellation covers both lock lookup and queueing. Stop/client tokens
        // are linked into this call, so no queued mutation may escape either.
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => Err(()),
            guard = acquire => Ok(Some(guard)),
        }
    }
}

pub type ToolGuard = for<'a> fn(&'a ToolCall) -> BoxFuture<'a, Option<ToolResult>>;

pub struct ToolEffectContext<'a> {
    pub call: &'a ToolCall,
    pub result: &'a ToolResult,
    pub cancelled: bool,
    pub duration_ms: i64,
}

pub type ToolEffect =
    for<'a> fn(ToolEffectContext<'a>) -> BoxFuture<'a, anyhow::Result<Option<ToolResult>>>;

#[derive(Clone, Copy)]
pub struct NamedToolEffect {
    pub name: &'static str,
    pub run: ToolEffect,
}

pub struct ToolObserverContext<'a> {
    pub call: &'a ToolCall,
    pub result: &'a ToolResult,
    pub cancelled: bool,
    pub duration_ms: i64,
}

pub type ToolObserver = for<'a> fn(ToolObserverContext<'a>) -> BoxFuture<'a, anyhow::Result<()>>;

#[derive(Clone, Copy)]
pub struct NamedToolObserver {
    pub name: &'static str,
    pub run: ToolObserver,
}

// Ownership is not a guard and must not become one. An agent is allowed to act on
// the user's tabs and on other agents' tabs; it is simply told whose they are, by
// `effects::page_ownership_notice`. See that module for why.
const GUARDS: &[ToolGuard] = &[
    guards::navigate_scheme::guard,
    guards::browser_connected::guard,
];

const EFFECTS: &[NamedToolEffect] = &[
    NamedToolEffect {
        name: "ownership-claims",
        run: effects::ownership_claims::apply,
    },
    NamedToolEffect {
        name: "tabs-list-view",
        run: effects::tabs_list_view::apply,
    },
    NamedToolEffect {
        name: "tab-activity",
        run: effects::tab_activity::apply,
    },
    NamedToolEffect {
        name: "tab-groups",
        run: effects::tab_groups::apply,
    },
    NamedToolEffect {
        name: "session-naming",
        run: effects::session_naming::apply,
    },
    NamedToolEffect {
        // Runs late so it annotates the result the agent will actually read, and
        // never changes it beyond appending a note. Informational only.
        name: "page-ownership-notice",
        run: effects::page_ownership_notice::apply,
    },
    NamedToolEffect {
        // Also an appended label: a saved profile target the native could not apply.
        name: "scope-notice",
        run: effects::scope_notice::apply,
    },
    NamedToolEffect {
        name: "helper-discovery",
        run: helper_runtime::discovery,
    },
];

// The distiller (auto-capture of a successful run into a candidate helper) is
// intentionally not wired. Self-healing keeps its agent-driven surface
// (saveHelper/listHelpers/readHelper, discovery, hot-load) but does not
// auto-distill. Re-add a NamedToolObserver for `distill::distill` to re-enable.
const OBSERVERS: &[NamedToolObserver] = &[
    NamedToolObserver {
        name: "audit",
        run: observers::audit::apply,
    },
    NamedToolObserver {
        name: "run_failure",
        run: observers::run_failure::apply,
    },
];

struct ExecutionOutcome {
    result: ToolResult,
    cancelled: bool,
    duration_ms: i64,
}

enum DispatchExecution {
    Completed(ExecutionOutcome),
    ProtocolCancelled,
}

/// Dispatches a tool through guards, execution, ordered effects, and read-only observers.
pub async fn dispatch_tool_call(call: ToolCall) -> Result<CallToolResult, McpError> {
    dispatch_tool_call_with(call, GUARDS, EFFECTS, OBSERVERS).await
}

async fn dispatch_tool_call_with(
    mut call: ToolCall,
    guards: &[ToolGuard],
    effects: &[NamedToolEffect],
    observers: &[NamedToolObserver],
) -> Result<CallToolResult, McpError> {
    if let Some(identity) = &call.identity
        && !identity
            .session
            .try_register_dispatch(call.dispatch_id.clone(), call.dispatch_cancel.clone())
            .await
    {
        call.dispatch_cancel.cancel();
        call.cancel.cancel();
        return Err(McpError::invalid_request(
            "BrowserOS neo session is no longer live",
            None,
        ));
    }
    if let (Some(browser), Some(page_id)) = (&call.browser_session, extract_page_id(&call)) {
        call.page_snapshot = browser.pages.get_info(PageId(page_id)).await;
    }

    let result = if let Some(rejection) = run_guards(&call, guards).await {
        Ok(rejection)
    } else {
        if ARBITRARY_SCRIPT_TOOLS.contains(&call.tool().name) {
            warn!(
                tool = call.tool().name,
                session_id = %call.session_id,
                "cockpit dispatched arbitrary-script tool"
            );
        }

        match execute_with_cancellation(&call).await {
            DispatchExecution::ProtocolCancelled => {
                tracing::info!(
                    tool = call.tool().name,
                    session_id = %call.session_id,
                    "cockpit tool dispatch cancelled by client"
                );
                Err(McpError::internal_error(CLIENT_CANCELLATION_ERROR, None))
            }
            DispatchExecution::Completed(outcome) => {
                if outcome.result.is_error && !outcome.cancelled {
                    warn!(
                        tool = call.tool().name,
                        session_id = %call.session_id,
                        duration_ms = outcome.duration_ms,
                        error = ?dispatch_error_text(&outcome.result),
                        "cockpit tool dispatch failed"
                    );
                }
                let result = run_effects(
                    ToolEffectContext {
                        call: &call,
                        result: &outcome.result,
                        cancelled: outcome.cancelled,
                        duration_ms: outcome.duration_ms,
                    },
                    effects,
                )
                .await;
                run_observers(
                    ToolObserverContext {
                        call: &call,
                        result: &result,
                        cancelled: outcome.cancelled,
                        duration_ms: outcome.duration_ms,
                    },
                    observers,
                )
                .await;
                Ok(result)
            }
        }
    };

    let (teardown_before_finish, operator_stop_requested) = if let Some(identity) = &call.identity {
        (
            !identity.session.finish_dispatch(&call.dispatch_id).await,
            identity.session.operator_stop_requested(),
        )
    } else {
        (false, false)
    };
    let has_output_schema = call.tool().output_schema.is_some();
    if teardown_before_finish && operator_stop_requested {
        let cancellation = operator_cancellation_result();
        call.dispatch_cancel.cancel();
        call.cancel.cancel();
        // The operator-cancellation envelope is a dispatch-layer error, not the tool's
        // promised output, so it must stay content-only even for schema-bearing tools;
        // forwarding its structured content would violate the tool's output_schema.
        return Ok(wire_result(cancellation, false));
    }
    call.dispatch_cancel.cancel();
    call.cancel.cancel();
    result.map(|result| wire_result(result, has_output_schema))
}

async fn run_guards(call: &ToolCall, guards: &[ToolGuard]) -> Option<ToolResult> {
    for guard in guards {
        if let Some(rejection) = guard(call).await {
            return Some(rejection);
        }
    }
    None
}

async fn run_effects(context: ToolEffectContext<'_>, effects: &[NamedToolEffect]) -> ToolResult {
    let mut result = context.result.clone();
    for effect in effects {
        match (effect.run)(ToolEffectContext {
            call: context.call,
            result: &result,
            cancelled: context.cancelled,
            duration_ms: context.duration_ms,
        })
        .await
        {
            Ok(Some(replacement)) => result = replacement,
            Ok(None) => {}
            Err(error) => warn!(
                tool = context.call.tool().name,
                session_id = %context.call.session_id,
                effect = effect.name,
                error = %error,
                "cockpit tool dispatch effect failed"
            ),
        }
    }
    result
}

async fn run_observers(context: ToolObserverContext<'_>, observers: &[NamedToolObserver]) {
    for observer in observers {
        if let Err(error) = (observer.run)(ToolObserverContext {
            call: context.call,
            result: context.result,
            cancelled: context.cancelled,
            duration_ms: context.duration_ms,
        })
        .await
        {
            warn!(
                tool = context.call.tool().name,
                session_id = %context.call.session_id,
                observer = observer.name,
                error = %error,
                "cockpit tool dispatch observer failed"
            );
        }
    }
}

async fn execute_with_cancellation(call: &ToolCall) -> DispatchExecution {
    let started = Instant::now();
    if call.dispatch_cancel.is_cancelled() {
        return DispatchExecution::Completed(ExecutionOutcome {
            result: operator_cancellation_result(),
            cancelled: true,
            duration_ms: 0,
        });
    }
    if call.client_cancel.is_cancelled() || call.cancel.is_cancelled() {
        return DispatchExecution::ProtocolCancelled;
    }
    let result = match &call.view {
        Some(view) => {
            // The same captured view reaches direct primitives and tool-backed calls.
            // The hook adds ownership labels and child audit rows, never scope authority.
            let is_script = ARBITRARY_SCRIPT_TOOLS.contains(&call.tool().name);
            let inner_call_hook: Option<Arc<dyn browseros_mcp::InnerCallHook>> =
                if is_script && call.identity.is_some() {
                    Some(
                        Arc::new(crate::api::mcp::script_hook::ScriptInnerCallHook::new(
                            call.clone(),
                        )) as Arc<dyn browseros_mcp::InnerCallHook>,
                    )
                } else {
                    None
                };
            // Hot-load the helpers the agent's owned-tab hosts make relevant, so
            // the script can call them by name. Cheap-gated when no helpers exist.
            let preloaded_helpers = match &call.identity {
                Some(identity) if is_script => {
                    crate::api::mcp::helper_runtime::preload_helpers(
                        &call.state,
                        &identity.ownership_key,
                        view,
                    )
                    .await
                }
                _ => Vec::new(),
            };
            let ctx = ToolCtx::new(BrowserToolOptions {
                browser: view.clone(),
                target: Some(call.target_store.clone()),
                defaults: BrowserToolDefaults {
                    default_window_id: None,
                    default_tab_group_id: call.default_tab_group_id.clone(),
                },
                cancel: call.cancel.clone(),
                output_files: call.output_files.clone(),
                inner_call_hook,
                preloaded_helpers,
            });
            // A tab group is destroyed when its last page is reparented. Explicit
            // placement must share the detached grouper's conversation lock, so a
            // callback cannot erase a group after we return its successful ID.
            let execution = async {
                // `run` acquires per inner tool through ScriptInnerCallHook,
                // never for the whole script or for unrelated primitives.
                let _guard = call
                    .acquire_group_operation_guard(call.tool().name, &call.raw_args)
                    .await
                    .map_err(|()| browseros_mcp::framework::ToolError::Cancelled)?;
                execute_tool(call.tool(), call.raw_args.clone(), &ctx).await
            };
            match execution.await {
                Ok(result) => result,
                Err(browseros_mcp::framework::ToolError::Cancelled) => {
                    ToolResult::error(format!("{} failed: cancelled", call.tool().name))
                }
                Err(error) => ToolResult::error(format!("{} failed: {error}", call.tool().name)),
            }
        }
        None => ToolResult::error(browser_unavailable_message(&call.state).await),
    };
    let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    if call.dispatch_cancel.is_cancelled() {
        return DispatchExecution::Completed(ExecutionOutcome {
            result: operator_cancellation_result(),
            cancelled: true,
            duration_ms,
        });
    }
    if call.client_cancel.is_cancelled() || call.cancel.is_cancelled() {
        return DispatchExecution::ProtocolCancelled;
    }
    DispatchExecution::Completed(ExecutionOutcome {
        result,
        cancelled: false,
        duration_ms,
    })
}

/// Why the browser is unreachable, in terms the caller can act on.
///
/// The server is always retrying, so every case advises retrying rather than
/// relaunching. Only the case where no link was ever established mentions starting
/// the browser, because that is the only one where it might not be running.
/// Telling an agent to start a browser that is already running is what turns a
/// transient loss into the user quitting the app.
pub(super) async fn browser_unavailable_message(state: &AppState) -> String {
    let link = state.browser.link_status().await;
    if let Some(down_for) = link.down_for {
        // Whole seconds truncate a sub-second outage to "0s ago", which reads as a
        // bug in the very message meant to explain one.
        let ago = match down_for.as_secs() {
            0 => "under a second".to_string(),
            seconds => format!("{seconds}s"),
        };
        return format!(
            "browser link lost {ago} ago and the server is reconnecting; BrowserOS neo is running, so wait a moment and retry this tool. Do not relaunch it and do not fall back to another browser tool."
        );
    }
    if link.ever_connected {
        return "browser link is down and the server is reconnecting; BrowserOS neo was reachable a moment ago, so wait and retry this tool. Do not relaunch it and do not fall back to another browser tool."
            .to_string();
    }
    "no link to the browser yet and the server is still retrying; wait a few seconds and retry this tool. If BrowserOS neo is not running, tell the user to start it. Do not fall back to another browser tool."
        .to_string()
}

pub(super) fn operator_cancellation_result() -> ToolResult {
    ToolResult {
        content: vec![ContentBlock::text(CANCELLATION_REASON)],
        is_error: true,
        structured_content: Some(json!({
            "cancellationReason": CANCELLATION_REASON,
            "cancellationKind": "cockpit.operator-cancelled"
        })),
        audit_hints: None,
    }
}

fn wire_result(result: ToolResult, has_output_schema: bool) -> CallToolResult {
    // Ordered effects retain structured content internally; the default BrowserClaw
    // wire envelope exposes content blocks only. The exception is a tool that
    // advertises an output_schema (e.g. `run`): a spec-compliant client rejects the
    // call unless the promised structured content is actually delivered, so keep it.
    let structured = if has_output_schema {
        result.structured_content
    } else {
        None
    };
    let mut call_result = if result.is_error {
        CallToolResult::error(result.content)
    } else {
        CallToolResult::success(result.content)
    };
    call_result.structured_content = structured;
    call_result
}

#[must_use]
pub fn extract_page_id(call: &ToolCall) -> Option<u32> {
    if !call.tool().metadata.accepts_page_arg {
        return None;
    }
    call.raw_args
        .get("page")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value >= 1)
}

#[must_use]
pub fn result_page_id(result: &ToolResult) -> Option<u32> {
    result
        .structured_content
        .as_ref()
        .and_then(|value| value.get("page"))
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value >= 1)
}

#[must_use]
pub fn page_id(call: &ToolCall, result: &ToolResult) -> Option<PageId> {
    result_page_id(result)
        .or_else(|| extract_page_id(call))
        .map(PageId)
}

pub(crate) fn dispatch_error_text(result: &ToolResult) -> Option<String> {
    result.content.iter().find_map(|block| match block {
        ContentBlock::Text(text) => Some(text.text.chars().take(DISPATCH_ERROR_TEXT_MAX).collect()),
        _ => None,
    })
}

/// Links request and operator cancellation into a session-owned child token.
#[must_use]
pub fn linked_cancel_token(
    session_cancel: CancellationToken,
    request_cancel: CancellationToken,
    dispatch_cancel: CancellationToken,
) -> CancellationToken {
    for source in [request_cancel, dispatch_cancel] {
        let cancel = session_cancel.clone();
        let completion = session_cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = source.cancelled_owned() => cancel.cancel(),
                () = completion.cancelled_owned() => {}
            }
        });
    }
    session_cancel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deferred_grouping_preserves_explicit_group_update() -> anyhow::Result<()> {
        assert_deferred_grouping_preserves_explicit_group("update", true).await
    }

    #[tokio::test]
    async fn deferred_grouping_preserves_explicit_group_close() -> anyhow::Result<()> {
        assert_deferred_grouping_preserves_explicit_group("close", true).await
    }

    #[tokio::test]
    async fn deferred_grouping_preserves_completed_explicit_group_update() -> anyhow::Result<()> {
        assert_deferred_grouping_preserves_explicit_group("update", false).await
    }

    #[tokio::test]
    async fn deferred_grouping_preserves_completed_explicit_group_close() -> anyhow::Result<()> {
        assert_deferred_grouping_preserves_explicit_group("close", false).await
    }

    async fn assert_deferred_grouping_preserves_explicit_group(
        action: &str,
        grouping_first: bool,
    ) -> anyhow::Result<()> {
        use crate::{
            api::mcp::test_support::{ProfileConnection, tool_call},
            services::sessions::{ScopedGroupKey, TabGroup},
        };
        use browseros_core::{WindowId, pages::NewPageOptions, target::BrowserView};
        use std::{sync::atomic::Ordering, task::Poll};
        let native = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(native.clone());
        browser.pages.list().await?;
        let mut call = tool_call("tabs", json!({"action":"new"})).await?;
        call.browser_session = Some(browser.clone());
        call.view = Some(BrowserView::unbound(browser, None));
        let page = call
            .view
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("view missing"))?
            .new_page(
                "https://synthetic.test/explicit-group",
                NewPageOptions::default(),
            )
            .await?;
        let created = effects::tab_groups::created_page(&call, page.page_id.0).await;
        assert!(created.is_some());
        let key = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .ownership_key
            .clone();
        let ownership = call.state.sessions.ownership();
        // A preceding case closed the last tab of the remembered session group.
        // Its native lookup awaits while this new page still has no group.
        ownership
            .remember_scoped_group(
                &key,
                ScopedGroupKey {
                    browser_profile_id: None,
                    window_id: WindowId(2),
                },
                TabGroup {
                    group_ref: Some("previous-closed-group".into()),
                    window_id: Some(WindowId(2)),
                    ..TabGroup::default()
                },
            )
            .await;
        let mut explicit = call.clone();
        explicit.tool_index = explicit
            .catalog
            .iter()
            .position(|tool| tool.name == "tab_groups")
            .ok_or_else(|| anyhow::anyhow!("tab_groups missing"))?;
        explicit.flags = ToolFlags::default();
        explicit.raw_args =
            json!({"action":"create", "pages":[page.page_id.0], "title":"before-update"});
        let created = if grouping_first {
            native.pause_group_lookup.store(true, Ordering::SeqCst);
            let entered = native.group_lookup_entered.notified();
            let grouping = tokio::spawn(effects::tab_groups::run_tab_group_work(
                call.clone(),
                created,
            ));
            tokio::time::timeout(std::time::Duration::from_secs(3), entered).await?;
            let mut create = Box::pin(execute_with_cancellation(&explicit));
            // Poll the actual Claw execution seam. Without mutual exclusion, the
            // explicit native group is published before the older callback resumes.
            let first = futures_util::poll!(create.as_mut());
            native.group_lookup_release.notify_one();
            tokio::time::timeout(std::time::Duration::from_secs(3), grouping).await??;
            match first {
                Poll::Ready(result) => result,
                Poll::Pending => {
                    tokio::time::timeout(std::time::Duration::from_secs(3), create).await?
                }
            }
        } else {
            // The creation receipt predates the explicit placement. A queued
            // callback must use the current group once it acquires the lock.
            let result = execute_with_cancellation(&explicit).await;
            effects::tab_groups::run_tab_group_work(call, created).await;
            result
        };
        let DispatchExecution::Completed(created) = created else {
            anyhow::bail!("creation cancelled")
        };
        assert!(
            !created.result.is_error,
            "{:?}",
            dispatch_error_text(&created.result)
        );
        let id = created
            .result
            .structured_content
            .as_ref()
            .and_then(|value| value["group"]["groupId"].as_str())
            .ok_or_else(|| anyhow::anyhow!("explicit group id missing"))?
            .to_string();
        explicit.raw_args = if action == "update" {
            json!({"action":"update", "groupId":id, "title":"after-update", "color":"green", "collapsed":true})
        } else {
            json!({"action":"close", "groupId":id})
        };
        let DispatchExecution::Completed(result) = execute_with_cancellation(&explicit).await
        else {
            anyhow::bail!("mutation cancelled")
        };
        assert!(
            !result.result.is_error,
            "explicit group {action} failed: {:?}",
            dispatch_error_text(&result.result)
        );
        if action == "update" {
            let groups = native.groups.lock().unwrap_or_else(|p| p.into_inner());
            let group = groups
                .iter()
                .find(|group| group["groupId"] == id)
                .ok_or_else(|| anyhow::anyhow!("explicit group vanished"))?;
            assert_eq!(group["title"], "after-update");
            assert_eq!(group["color"], "green");
            assert_eq!(group["collapsed"], true);
            assert_eq!(group["tabIds"], json!([page.tab_id.0]));
        } else {
            assert!(
                !native
                    .groups
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .iter()
                    .any(|group| group["groupId"] == id)
            );
            assert!(
                !native
                    .tabs
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .iter()
                    .any(|tab| tab["tabId"] == page.tab_id.0)
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn deferred_grouping_observes_new_native_group_during_lookup() -> anyhow::Result<()> {
        use crate::{
            api::mcp::test_support::{ProfileConnection, tool_call},
            services::sessions::{ScopedGroupKey, TabGroup},
        };
        use browseros_core::{WindowId, pages::NewPageOptions, target::BrowserView};
        use std::sync::atomic::Ordering;
        let native = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(native.clone());
        browser.pages.list().await?;
        let mut call = tool_call("tabs", json!({"action":"new"})).await?;
        call.browser_session = Some(browser.clone());
        call.view = Some(BrowserView::unbound(browser, None));
        let view = call
            .view
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("view missing"))?;
        let original = view
            .new_page(
                "https://synthetic.test/native-placement",
                NewPageOptions::default(),
            )
            .await?;
        let created = effects::tab_groups::created_page(&call, original.page_id.0).await;
        let key = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .ownership_key
            .clone();
        call.state
            .sessions
            .ownership()
            .remember_scoped_group(
                &key,
                ScopedGroupKey {
                    browser_profile_id: None,
                    window_id: WindowId(2),
                },
                TabGroup {
                    group_ref: Some("previous-closed-group".into()),
                    window_id: Some(WindowId(2)),
                    ..TabGroup::default()
                },
            )
            .await;
        native.pause_group_lookup.store(true, Ordering::SeqCst);
        let entered = native.group_lookup_entered.notified();
        let grouping = tokio::spawn(effects::tab_groups::run_tab_group_work(
            call.clone(),
            created,
        ));
        tokio::time::timeout(std::time::Duration::from_secs(3), entered).await?;
        // Another native actor does not participate in the host mutex. Group
        // membership may change during lookup without changing creation identity.
        let explicit = view
            .cdp(
                "Browser.createTabGroup",
                json!({"tabIds":[original.tab_id.0], "title":"explicit"}),
                None,
            )
            .await?;
        let group_id = explicit["group"]["groupId"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("explicit group missing"))?;
        native.group_lookup_release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(3), grouping).await??;
        let current = view.page(original.page_id.clone()).await?;
        assert_eq!(current.target_id, original.target_id);
        assert_eq!(current.tab_id, original.tab_id);
        assert_eq!(current.window_id, original.window_id);
        assert_eq!(
            current.group_id.as_deref(),
            Some(group_id),
            "newer explicit native placement was overwritten"
        );
        let calls = native.calls.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(
            calls
                .iter()
                .filter(|(method, _)| method == "Browser.createTabGroup")
                .count(),
            1
        );
        assert!(!calls.iter().any(|(method, _)| matches!(
            method.as_str(),
            "Browser.addTabsToGroup" | "Browser.updateTabGroup"
        )));
        Ok(())
    }

    #[tokio::test]
    async fn explicit_group_wait_honors_client_cancellation() -> anyhow::Result<()> {
        assert_explicit_group_wait_honors_client_cancellation(false).await
    }

    #[tokio::test]
    async fn run_explicit_group_wait_honors_client_cancellation() -> anyhow::Result<()> {
        assert_explicit_group_wait_honors_client_cancellation(true).await
    }

    async fn queued_group_call(
        script: bool,
    ) -> anyhow::Result<(
        ToolCall,
        Arc<crate::api::mcp::test_support::ProfileConnection>,
    )> {
        use crate::api::mcp::test_support::{ProfileConnection, tool_call};
        use browseros_core::target::BrowserView;
        let native = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(native.clone());
        browser.pages.list().await?;
        let mut call = if script {
            tool_call(
                "run",
                json!({"code":"return await browser.tabGroups({action:'create',pages:[1]});"}),
            )
            .await?
        } else {
            tool_call("tab_groups", json!({"action":"create", "pages":[1]})).await?
        };
        call.browser_session = Some(browser.clone());
        call.view = Some(BrowserView::unbound(browser, None));
        Ok((call, native))
    }

    async fn assert_explicit_group_wait_honors_client_cancellation(
        script: bool,
    ) -> anyhow::Result<()> {
        let (call, native) = queued_group_call(script).await?;
        let key = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .ownership_key
            .clone();
        let ownership = call.state.sessions.ownership();
        let lock = ownership.group_operation_lock(&key).await;
        let _busy = lock.lock().await;
        let mut work = Box::pin(execute_with_cancellation(&call));
        assert!(futures_util::poll!(work.as_mut()).is_pending());
        call.client_cancel.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), work).await?;
        assert!(matches!(result, DispatchExecution::ProtocolCancelled));
        assert!(
            !native
                .calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .any(|(method, _)| method.contains("TabGroup"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn explicit_group_wait_honors_operator_stop() -> anyhow::Result<()> {
        assert_explicit_group_wait_honors_operator_stop(false).await
    }

    #[tokio::test]
    async fn run_explicit_group_wait_honors_operator_stop() -> anyhow::Result<()> {
        assert_explicit_group_wait_honors_operator_stop(true).await
    }

    async fn assert_explicit_group_wait_honors_operator_stop(script: bool) -> anyhow::Result<()> {
        let (call, native) = queued_group_call(script).await?;
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;
        let session = identity.session.clone();
        let lock = call
            .state
            .sessions
            .ownership()
            .group_operation_lock(&identity.ownership_key)
            .await;
        let busy = lock.lock().await;
        // Register the real dispatch token so the manager's Stop transition owns
        // cancellation and waits for this queued call's completion/audit handoff.
        assert!(
            session
                .try_register_dispatch(call.dispatch_id.clone(), call.dispatch_cancel.clone())
                .await
        );
        let mut work = Box::pin(execute_with_cancellation(&call));
        assert!(futures_util::poll!(work.as_mut()).is_pending());
        let sessions = call.state.sessions.clone();
        let session_id = call.session_id.clone();
        let stop = tokio::spawn(async move { sessions.cancel_by_session(&session_id).await });
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            call.dispatch_cancel.cancelled(),
        )
        .await?;
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), work).await?;
        let DispatchExecution::Completed(outcome) = result else {
            anyhow::bail!("operator Stop became protocol cancellation")
        };
        assert!(outcome.cancelled);
        assert_eq!(
            dispatch_error_text(&outcome.result).as_deref(),
            Some(CANCELLATION_REASON)
        );
        assert!(
            !native
                .calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .any(|(method, _)| method.contains("TabGroup"))
        );
        assert!(!session.finish_dispatch(&call.dispatch_id).await);
        drop(busy);
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(3), stop).await???,
            Some(1)
        );
        assert!(session.operator_stop_requested());
        Ok(())
    }

    #[tokio::test]
    async fn run_group_listing_and_unrelated_tools_do_not_wait_for_group_mutation()
    -> anyhow::Result<()> {
        let (mut call, native) = queued_group_call(true).await?;
        let key = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .ownership_key
            .clone();
        let lock = call
            .state
            .sessions
            .ownership()
            .group_operation_lock(&key)
            .await;
        let _busy = lock.lock().await;
        // The optional host seam must be per mutation, not a whole-run lock.
        // Read-only groups and unrelated windows remain usable while it is busy.
        call.raw_args = json!({"code":"return {groups:await browser.tabGroups({action:'list'}),windows:await browser.windows({action:'list'})};"});
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            execute_with_cancellation(&call),
        )
        .await?;
        let DispatchExecution::Completed(outcome) = result else {
            anyhow::bail!("script cancelled")
        };
        assert!(
            !outcome.result.is_error,
            "{:?}",
            dispatch_error_text(&outcome.result)
        );
        let value = outcome
            .result
            .structured_content
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("script readback missing"))?;
        assert_eq!(value["value"]["groups"]["groups"], json!([]));
        assert_eq!(
            value["value"]["windows"]["windows"]
                .as_array()
                .map(Vec::len),
            Some(3)
        );
        assert!(
            !native
                .calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .any(|(method, _)| {
                    matches!(
                        method.as_str(),
                        "Browser.createTabGroup"
                            | "Browser.addTabsToGroup"
                            | "Browser.updateTabGroup"
                            | "Browser.closeTabGroup"
                    )
                })
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_deferred_grouping_preserves_explicit_group_update() -> anyhow::Result<()> {
        assert_run_deferred_grouping_preserves_explicit_group("update").await
    }

    #[tokio::test]
    async fn run_deferred_grouping_preserves_explicit_group_close() -> anyhow::Result<()> {
        assert_run_deferred_grouping_preserves_explicit_group("close").await
    }

    async fn assert_run_deferred_grouping_preserves_explicit_group(
        action: &str,
    ) -> anyhow::Result<()> {
        use crate::api::mcp::test_support::{ProfileConnection, tool_call};
        use browseros_core::{pages::NewPageOptions, target::BrowserView};
        use std::{sync::atomic::Ordering, task::Poll};
        let native = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(native.clone());
        browser.pages.list().await?;
        let mut call = tool_call("tabs", json!({"action":"new"})).await?;
        call.browser_session = Some(browser.clone());
        call.view = Some(BrowserView::unbound(browser, None));
        let page = call
            .view
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("view missing"))?
            .new_page(
                "https://synthetic.test/script-group",
                NewPageOptions::default(),
            )
            .await?;
        let created = effects::tab_groups::created_page(&call, page.page_id.0).await;
        assert!(created.is_some());
        // Unlike the lookup test, this callback has finished ALL fresh reads.
        // Pause its native write: a script must share ordering with this callback,
        // not merely rely on another advisory membership read before dispatch.
        native.pause_group_create.store(true, Ordering::SeqCst);
        let entered = native.group_create_entered.notified();
        let grouping = tokio::spawn(effects::tab_groups::run_tab_group_work(
            call.clone(),
            created,
        ));
        tokio::time::timeout(std::time::Duration::from_secs(3), entered).await?;
        let mut script = call.clone();
        script.tool_index = script
            .catalog
            .iter()
            .position(|tool| tool.name == "run")
            .ok_or_else(|| anyhow::anyhow!("run missing"))?;
        script.flags = ToolFlags::default();
        script.raw_args = json!({"code":format!(
            "return await browser.tabGroups({{action:'create',pages:[{}],title:'script-explicit'}});",
            page.page_id.0
        )});
        let result = {
            let mut work = Box::pin(execute_with_cancellation(&script));
            let first = futures_util::poll!(work.as_mut());
            native.group_create_release.notify_one();
            tokio::time::timeout(std::time::Duration::from_secs(3), grouping).await??;
            match first {
                Poll::Ready(result) => result,
                Poll::Pending => {
                    tokio::time::timeout(std::time::Duration::from_secs(3), work).await?
                }
            }
        };
        let DispatchExecution::Completed(outcome) = result else {
            anyhow::bail!("script cancelled")
        };
        assert!(
            !outcome.result.is_error,
            "{:?}",
            dispatch_error_text(&outcome.result)
        );
        let group_id = outcome
            .result
            .structured_content
            .as_ref()
            .and_then(|value| value["value"]["group"]["groupId"].as_str())
            .ok_or_else(|| anyhow::anyhow!("script group id missing"))?;
        let mut mutation = script.clone();
        mutation.tool_index = mutation
            .catalog
            .iter()
            .position(|tool| tool.name == "tab_groups")
            .ok_or_else(|| anyhow::anyhow!("tab_groups missing"))?;
        mutation.raw_args = if action == "update" {
            json!({"action":"update", "groupId":group_id, "title":"script-after", "color":"green"})
        } else {
            json!({"action":"close", "groupId":group_id})
        };
        let DispatchExecution::Completed(outcome) = execute_with_cancellation(&mutation).await
        else {
            anyhow::bail!("mutation cancelled")
        };
        assert!(
            !outcome.result.is_error,
            "script's explicit group {action} failed: {:?}",
            dispatch_error_text(&outcome.result)
        );
        if action == "update" {
            let groups = native.groups.lock().unwrap_or_else(|p| p.into_inner());
            let group = groups
                .iter()
                .find(|group| group["groupId"] == group_id)
                .ok_or_else(|| anyhow::anyhow!("script group vanished"))?;
            assert_eq!(group["title"], "script-after");
            assert_eq!(group["color"], "green");
            assert_eq!(group["tabIds"], json!([page.tab_id.0]));
        } else {
            assert!(
                !native
                    .tabs
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .iter()
                    .any(|tab| tab["tabId"] == page.tab_id.0)
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn stale_binding_blocks_execution_while_management_recovers() -> anyhow::Result<()> {
        use crate::api::mcp::test_support::{ProfileConnection, tool_call};
        let transport = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(transport.clone());
        browser.pages.list().await?;
        let mut call = tool_call("tabs", json!({"action":"list"})).await?;
        let binding = call
            .state
            .targets
            .set(
                &call.session_id,
                ProfileConnection::binding("eng", 1, 1).intent,
            )
            .await?;
        transport
            .profiles
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|profile| profile["browserProfileId"] != "eng");
        call.browser_session = Some(browser.clone());
        call.view = Some(BrowserView::capture(browser.clone(), Some(binding), None).await);
        let DispatchExecution::Completed(outcome) = execute_with_cancellation(&call).await else {
            anyhow::bail!("unexpected cancellation")
        };
        assert!(outcome.result.is_error);
        assert!(
            dispatch_error_text(&outcome.result).is_some_and(|text| text.contains("profile_gone")),
            "{:?}",
            dispatch_error_text(&outcome.result)
        );
        call.tool_index = call
            .catalog
            .iter()
            .position(|tool| tool.name == "browser_target")
            .ok_or_else(|| anyhow::anyhow!("browser_target missing"))?;
        call.raw_args = json!({"action":"get"});
        let DispatchExecution::Completed(outcome) = execute_with_cancellation(&call).await else {
            anyhow::bail!("unexpected cancellation")
        };
        assert!(!outcome.result.is_error);
        assert_eq!(
            outcome
                .result
                .structured_content
                .as_ref()
                .map(|value| &value["bound"]),
            Some(&json!(true))
        );
        call.raw_args = json!({"action":"clear"});
        let DispatchExecution::Completed(outcome) = execute_with_cancellation(&call).await else {
            anyhow::bail!("unexpected cancellation")
        };
        assert!(!outcome.result.is_error);
        assert_eq!(call.state.targets.get(&call.session_id).await, None);
        call.raw_args = json!({"action":"set", "name":"ops"});
        let DispatchExecution::Completed(outcome) = execute_with_cancellation(&call).await else {
            anyhow::bail!("unexpected cancellation")
        };
        assert!(!outcome.result.is_error);
        let selected = call
            .state
            .targets
            .get(&call.session_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("binding missing"))?;
        assert_eq!(selected.intent.browser_profile_id.as_str(), "ops");
        assert_eq!(selected.revision, 3);
        let view = BrowserView::capture(browser, Some(selected), None).await;
        let listing = view.list_pages().await?;
        assert_eq!(listing.pages.len(), 1);
        assert_eq!(
            listing.pages[0].window_id,
            Some(browseros_core::WindowId(2))
        );
        Ok(())
    }

    /// A binding saved on a newer browser meets an older native. The call runs with unbound
    /// routing, the outer result says so exactly once, and the saved intent stays put. A client
    /// still holding a list with the hidden profile tools is refused without profile CDP.
    #[tokio::test]
    async fn older_native_runs_saved_binding_unbound_and_refuses_hidden_tools() -> anyhow::Result<()>
    {
        use crate::api::mcp::test_support::{ProfileConnection, tool_call};
        let transport = ProfileConnection::new();
        let older = BrowserSession::new_with_native(
            transport.clone(),
            browseros_core::BrowserSessionHooks::default(),
            browseros_core::NativeCapabilities::from_browseros_version(Some("0.51.1.2")),
        );
        older.pages.list().await?;
        let mut call = tool_call("tabs", json!({"action":"list"})).await?;
        let binding = call
            .state
            .targets
            .set(
                &call.session_id,
                ProfileConnection::binding("eng", 1, 1).intent,
            )
            .await?;
        call.browser_session = Some(older.clone());
        call.view = Some(BrowserView::capture(older.clone(), Some(binding.clone()), None).await);
        let profile_reads = || {
            transport
                .calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .filter(|(method, _)| method.starts_with("Browser.") && method.contains("Profile"))
                .count()
        };

        let state = call.state.clone();
        let session_id = call.session_id.clone();
        let listed = dispatch_tool_call(call).await?;
        assert_ne!(listed.is_error, Some(true), "{listed:?}");
        let notes = listed
            .content
            .iter()
            .filter_map(|content| content.as_text())
            .filter(|content| content.text.contains("was not applied"))
            .count();
        assert_eq!(notes, 1, "{listed:?}");
        // Unbound routing: the "ops" profile's tab is listed next to eng's.
        let text = listed
            .content
            .iter()
            .filter_map(|content| content.as_text())
            .map(|content| content.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for tab in ["Tab 101", "Tab 102", "Tab 103"] {
            assert!(text.contains(tab), "{tab} missing from {text}");
        }
        assert_eq!(state.targets.get(&session_id).await, Some(binding));

        let mut cached = tool_call("browser_profiles", json!({})).await?;
        cached.browser_session = Some(older.clone());
        cached.view = Some(BrowserView::capture(older, None, None).await);
        let refused = dispatch_tool_call(cached).await?;
        assert_eq!(refused.is_error, Some(true));
        assert!(
            refused
                .content
                .iter()
                .filter_map(|content| content.as_text())
                .any(|content| content
                    .text
                    .contains("native_feature_unavailable: profile targeting")),
            "{refused:?}"
        );
        assert_eq!(profile_reads(), 0);
        Ok(())
    }

    /// With no saved selection a capable native's call runs in the original Default profile,
    /// not the focused window. A selection made after admission does not move the admitted
    /// run or its detached grouping, and the default itself is never saved for the task.
    #[tokio::test]
    async fn unselected_run_uses_the_default_profile_and_keeps_it_after_rebind()
    -> anyhow::Result<()> {
        use crate::api::mcp::test_support::{ProfileConnection, tool_call};
        let transport = ProfileConnection::new().with_default_profile();
        let browser = crate::api::mcp::test_support::current_session(transport.clone());
        browser.pages.list().await?;
        let mut call = tool_call(
            "run",
            json!({"code":"return await browser.pages.newPage('https://synthetic.test/default');"}),
        )
        .await?;
        call.browser_session = Some(browser.clone());
        call.view = Some(BrowserView::capture(browser.clone(), None, None).await);
        assert_eq!(call.state.targets.get(&call.session_id).await, None);
        call.state
            .targets
            .set(
                &call.session_id,
                ProfileConnection::binding("ops", 1, 2).intent,
            )
            .await?;
        let group_created = transport.group_created.notified();
        let DispatchExecution::Completed(outcome) = execute_with_cancellation(&call).await else {
            anyhow::bail!("unexpected cancellation")
        };
        assert!(
            !outcome.result.is_error,
            "{:?}",
            dispatch_error_text(&outcome.result)
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), group_created).await?;
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;
        let ownership = call.state.sessions.ownership();
        let lock = ownership
            .group_operation_lock(&identity.ownership_key)
            .await;
        let _guard = lock.lock().await;
        let tab = transport
            .tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|tab| tab["url"] == "https://synthetic.test/default")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("created tab missing"))?;
        assert_eq!(tab["windowId"], 4);
        assert_eq!(tab["browserProfileId"], "default");
        assert!(tab.get("groupId").is_some());
        assert!(
            ownership
                .groups(&identity.ownership_key)
                .await
                .iter()
                .all(|group| group.window_id == Some(browseros_core::WindowId(4)))
        );
        let creation = transport
            .calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|(method, _)| method == "Browser.createTab")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("createTab missing"))?;
        assert_eq!(creation.1["browserContextId"], "ctx-default");
        // Only the explicit selection made after admission is stored; the default never was.
        assert_eq!(
            call.state
                .targets
                .get(&call.session_id)
                .await
                .map(|binding| binding.intent.browser_profile_id.0),
            Some("ops".to_string())
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_creation_and_hook_keep_admitted_view_after_rebind() -> anyhow::Result<()> {
        use crate::api::mcp::test_support::{ProfileConnection, tool_call};
        let transport = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(transport.clone());
        browser.pages.list().await?;
        let mut call = tool_call(
            "run",
            json!({"code":"return await browser.pages.newPage('https://synthetic.test/new');"}),
        )
        .await?;
        call.browser_session = Some(browser.clone());
        let eng = call
            .state
            .targets
            .set(
                &call.session_id,
                ProfileConnection::binding("eng", 1, 1).intent,
            )
            .await?;
        call.view = Some(BrowserView::capture(browser.clone(), Some(eng), None).await);
        // The store has already moved to ops by the time execution starts. Both the
        // primitive and its detached creation hook must still use the admitted eng view.
        call.state
            .targets
            .set(
                &call.session_id,
                ProfileConnection::binding("ops", 2, 2).intent,
            )
            .await?;
        let group_created = transport.group_created.notified();
        let DispatchExecution::Completed(outcome) = execute_with_cancellation(&call).await else {
            anyhow::bail!("unexpected cancellation")
        };
        assert!(
            !outcome.result.is_error,
            "{:?}",
            dispatch_error_text(&outcome.result)
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), group_created).await?;
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;
        let ownership = call.state.sessions.ownership();
        let lock = ownership
            .group_operation_lock(&identity.ownership_key)
            .await;
        let _guard = lock.lock().await;
        let tab = transport
            .tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .last()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("created tab missing"))?;
        assert_eq!(tab["windowId"], 1);
        assert_eq!(tab["browserProfileId"], "eng");
        assert!(tab.get("groupId").is_some());
        assert_eq!(
            ownership.owner_of_page(&PageId(4)).await,
            Some(identity.ownership_key.clone())
        );
        assert!(
            ownership
                .groups(&identity.ownership_key)
                .await
                .iter()
                .all(|group| group.window_id == Some(browseros_core::WindowId(1)))
        );
        let calls = transport.calls.lock().unwrap_or_else(|p| p.into_inner());
        let creation = calls
            .iter()
            .find(|(method, _)| method == "Browser.createTab")
            .ok_or_else(|| anyhow::anyhow!("createTab missing"))?;
        assert_eq!(creation.1["browserContextId"], "ctx-eng");
        Ok(())
    }
    use crate::db::audit_log::ListDispatchesQuery;
    use browseros_mcp::token_estimate::{
        TOKEN_ESTIMATOR_VERSION, estimate_tool_input_tokens, estimate_tool_output_tokens,
    };
    use std::sync::{
        LazyLock, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Notify;

    static EFFECT_CALLS: AtomicUsize = AtomicUsize::new(0);
    static OBSERVER_CALLS: AtomicUsize = AtomicUsize::new(0);
    static GUARD_OBSERVER_CALLS: AtomicUsize = AtomicUsize::new(0);
    static CLIENT_OBSERVER_CALLS: AtomicUsize = AtomicUsize::new(0);
    static EFFECT_ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    static OBSERVED_RESULT: Mutex<Option<String>> = Mutex::new(None);
    static LATE_EFFECT_ENTERED: LazyLock<Notify> = LazyLock::new(Notify::new);
    static LATE_EFFECT_RELEASE: LazyLock<Notify> = LazyLock::new(Notify::new);
    static FINISH_OBSERVER_ENTERED: LazyLock<Notify> = LazyLock::new(Notify::new);
    static FINISH_OBSERVER_RELEASE: LazyLock<Notify> = LazyLock::new(Notify::new);

    fn record_effect(name: &'static str) {
        EFFECT_ORDER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(name);
    }

    fn passthrough_guard(call: &ToolCall) -> BoxFuture<'_, Option<ToolResult>> {
        Box::pin(async move {
            let _ = call;
            None
        })
    }

    fn replacement_effect(
        context: ToolEffectContext<'_>,
    ) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
        Box::pin(async move {
            let _ = context;
            Ok(Some(ToolResult::text(
                "replacement",
                Some(json!({ "internal": "x".repeat(300) })),
            )))
        })
    }

    fn failing_effect(
        context: ToolEffectContext<'_>,
    ) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
        Box::pin(async move {
            let _ = context;
            anyhow::bail!("effect failed")
        })
    }

    fn rejecting_guard(call: &ToolCall) -> BoxFuture<'_, Option<ToolResult>> {
        Box::pin(async move {
            let _ = call;
            Some(ToolResult::error("rejected"))
        })
    }

    fn counting_effect(
        context: ToolEffectContext<'_>,
    ) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
        Box::pin(async move {
            let _ = context;
            EFFECT_CALLS.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        })
    }

    fn failing_observer(context: ToolObserverContext<'_>) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            OBSERVER_CALLS.fetch_add(1, Ordering::SeqCst);
            *OBSERVED_RESULT
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                dispatch_error_text(context.result);
            anyhow::bail!("observer failed")
        })
    }

    fn counting_observer(context: ToolObserverContext<'_>) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            OBSERVER_CALLS.fetch_add(1, Ordering::SeqCst);
            *OBSERVED_RESULT
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                dispatch_error_text(context.result);
            Ok(())
        })
    }

    fn blocking_observer(context: ToolObserverContext<'_>) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            let _ = context;
            FINISH_OBSERVER_ENTERED.notify_one();
            FINISH_OBSERVER_RELEASE.notified().await;
            Ok(())
        })
    }

    fn guard_counting_observer(
        context: ToolObserverContext<'_>,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            let _ = context;
            GUARD_OBSERVER_CALLS.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }

    fn client_counting_observer(
        context: ToolObserverContext<'_>,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            let _ = context;
            CLIENT_OBSERVER_CALLS.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }

    fn audit_then_wait_observer(
        context: ToolObserverContext<'_>,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            observers::audit::apply(context).await?;
            LATE_EFFECT_ENTERED.notify_one();
            LATE_EFFECT_RELEASE.notified().await;
            Ok(())
        })
    }

    fn first_ordered_effect(
        context: ToolEffectContext<'_>,
    ) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
        Box::pin(async move {
            let _ = context;
            record_effect("first");
            Ok(Some(ToolResult::text("first replacement", None)))
        })
    }

    fn failing_ordered_effect(
        context: ToolEffectContext<'_>,
    ) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
        Box::pin(async move {
            let _ = context;
            record_effect("failing");
            anyhow::bail!("ordered effect failed")
        })
    }

    fn last_ordered_effect(
        context: ToolEffectContext<'_>,
    ) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
        Box::pin(async move {
            let _ = context;
            record_effect("last");
            Ok(Some(ToolResult::text("last replacement", None)))
        })
    }

    #[tokio::test]
    async fn tabs_flags_default_to_list() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", Value::Object(serde_json::Map::new()))
                .await?;
        assert_eq!(
            call.flags,
            ToolFlags {
                list_tabs: true,
                ..ToolFlags::default()
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn effect_failure_keeps_latest_good_result() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let initial = ToolResult::text("initial", None);
        let result = run_effects(
            ToolEffectContext {
                call: &call,
                result: &initial,
                cancelled: false,
                duration_ms: 1,
            },
            &[
                NamedToolEffect {
                    name: "replace",
                    run: replacement_effect,
                },
                NamedToolEffect {
                    name: "fail",
                    run: failing_effect,
                },
            ],
        )
        .await;
        assert_eq!(dispatch_error_text(&result).as_deref(), Some("replacement"));
        Ok(())
    }

    #[tokio::test]
    async fn effects_run_in_order_and_continue_after_a_failure() -> anyhow::Result<()> {
        EFFECT_ORDER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let initial = ToolResult::text("initial", None);
        let result = run_effects(
            ToolEffectContext {
                call: &call,
                result: &initial,
                cancelled: false,
                duration_ms: 1,
            },
            &[
                NamedToolEffect {
                    name: "first",
                    run: first_ordered_effect,
                },
                NamedToolEffect {
                    name: "failing",
                    run: failing_ordered_effect,
                },
                NamedToolEffect {
                    name: "last",
                    run: last_ordered_effect,
                },
            ],
        )
        .await;
        assert_eq!(
            *EFFECT_ORDER
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ["first", "failing", "last"]
        );
        assert_eq!(
            dispatch_error_text(&result).as_deref(),
            Some("last replacement")
        );
        Ok(())
    }

    #[tokio::test]
    async fn observers_receive_the_final_effect_result_and_continue_after_failure()
    -> anyhow::Result<()> {
        OBSERVER_CALLS.store(0, Ordering::SeqCst);
        *OBSERVED_RESULT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let initial = ToolResult::text("initial", None);
        let result = run_effects(
            ToolEffectContext {
                call: &call,
                result: &initial,
                cancelled: false,
                duration_ms: 1,
            },
            &[NamedToolEffect {
                name: "replace",
                run: replacement_effect,
            }],
        )
        .await;
        run_observers(
            ToolObserverContext {
                call: &call,
                result: &result,
                cancelled: false,
                duration_ms: 1,
            },
            &[
                NamedToolObserver {
                    name: "fail",
                    run: failing_observer,
                },
                NamedToolObserver {
                    name: "count",
                    run: counting_observer,
                },
            ],
        )
        .await;

        assert_eq!(OBSERVER_CALLS.load(Ordering::SeqCst), 2);
        assert_eq!(
            OBSERVED_RESULT
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_deref(),
            Some("replacement")
        );
        assert_eq!(dispatch_error_text(&result).as_deref(), Some("replacement"));
        Ok(())
    }

    #[tokio::test]
    async fn terminal_audit_counts_the_final_wire_content_once() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let initial = ToolResult::text("initial", Some(json!({ "internal": true })));
        let final_result = run_effects(
            ToolEffectContext {
                call: &call,
                result: &initial,
                cancelled: false,
                duration_ms: 1,
            },
            &[
                NamedToolEffect {
                    name: "replace",
                    run: replacement_effect,
                },
                NamedToolEffect {
                    name: "session-naming",
                    run: effects::session_naming::apply,
                },
            ],
        )
        .await;
        run_observers(
            ToolObserverContext {
                call: &call,
                result: &final_result,
                cancelled: false,
                duration_ms: 1,
            },
            &[NamedToolObserver {
                name: "audit",
                run: observers::audit::apply,
            }],
        )
        .await;
        call.state
            .audit_worker
            .flush_session(call.session_id.as_str())
            .await?;
        let wire = wire_result(final_result.clone(), false);
        assert_eq!(wire.content.len(), 2);
        assert_eq!(wire.structured_content, None);

        let row = call
            .state
            .audit_log
            .list_dispatches(ListDispatchesQuery::default())
            .await?
            .rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("dispatch missing"))?;
        assert_eq!(row.token_estimator_version, TOKEN_ESTIMATOR_VERSION);
        assert_eq!(
            row.tool_input_token_estimate,
            estimate_tool_input_tokens(call.tool().name, &call.raw_args)
        );
        assert_eq!(
            row.tool_output_token_estimate,
            estimate_tool_output_tokens(&wire.content)
        );
        Ok(())
    }

    #[tokio::test]
    async fn guard_rejection_skips_effects() -> anyhow::Result<()> {
        EFFECT_CALLS.store(0, Ordering::SeqCst);
        GUARD_OBSERVER_CALLS.store(0, Ordering::SeqCst);
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let result = dispatch_tool_call_with(
            call,
            &[rejecting_guard],
            &[NamedToolEffect {
                name: "count",
                run: counting_effect,
            }],
            &[NamedToolObserver {
                name: "count",
                run: guard_counting_observer,
            }],
        )
        .await
        .unwrap_or_else(|error| panic!("guard rejection should stay in-band: {error:?}"));
        assert_eq!(result.is_error, Some(true));
        assert_eq!(EFFECT_CALLS.load(Ordering::SeqCst), 0);
        assert_eq!(GUARD_OBSERVER_CALLS.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test]
    async fn finish_dispatch_waits_for_observer_submission() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let session = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("tool call identity missing"))?
            .session
            .clone();
        let dispatch = tokio::spawn(dispatch_tool_call_with(
            call,
            &[],
            &[],
            &[NamedToolObserver {
                name: "blocking",
                run: blocking_observer,
            }],
        ));

        FINISH_OBSERVER_ENTERED.notified().await;
        assert_eq!(session.active_dispatch_count().await, 1);
        FINISH_OBSERVER_RELEASE.notify_one();
        dispatch.await??;
        assert_eq!(session.active_dispatch_count().await, 0);
        Ok(())
    }

    #[tokio::test]
    async fn stopped_session_rejects_browser_dispatch_before_effects() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let session = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("tool call identity missing"))?
            .session
            .clone();
        session.stop_dispatches().await;

        let result = dispatch_tool_call_with(
            call.clone(),
            &[],
            &[],
            &[NamedToolObserver {
                name: "audit",
                run: observers::audit::apply,
            }],
        )
        .await;

        let Err(error) = result else {
            panic!("stopped session must reject dispatch");
        };
        assert_eq!(
            error.message.as_ref(),
            "BrowserOS neo session is no longer live"
        );
        assert!(
            call.state
                .audit_log
                .list_dispatches(ListDispatchesQuery::default())
                .await?
                .rows
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn stop_winning_after_audit_rewrites_the_dispatch_as_cancelled() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let session = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("tool call identity missing"))?
            .session
            .clone();
        let dispatched_call = call.clone();
        let dispatch = tokio::spawn(async move {
            dispatch_tool_call_with(
                dispatched_call,
                &[],
                &[],
                &[NamedToolObserver {
                    name: "audit",
                    run: audit_then_wait_observer,
                }],
            )
            .await
        });

        LATE_EFFECT_ENTERED.notified().await;
        let sessions = call.state.sessions.clone();
        let session_id = session.id().clone();
        let cancel = tokio::spawn(async move { sessions.cancel_by_session(&session_id).await });
        call.dispatch_cancel.cancelled().await;
        LATE_EFFECT_RELEASE.notify_one();
        let result = dispatch.await??;
        assert_eq!(cancel.await??, Some(1));

        assert_eq!(
            result
                .content
                .first()
                .and_then(|block| block.as_text())
                .map(|text| text.text.as_str()),
            Some(CANCELLATION_REASON)
        );
        let rows = call
            .state
            .audit_log
            .list_dispatches(ListDispatchesQuery::default())
            .await?
            .rows;
        assert_eq!(rows.len(), 1);
        let meta: Value = serde_json::from_str(
            rows[0]
                .result_meta
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("result metadata missing"))?,
        )?;
        assert_eq!(meta["cancelled"], true);
        assert_eq!(meta["cancellationKind"], "cockpit.operator-cancelled");
        assert_eq!(rows[0].token_estimator_version, TOKEN_ESTIMATOR_VERSION);
        assert_eq!(
            rows[0].tool_input_token_estimate,
            estimate_tool_input_tokens(call.tool().name, &call.raw_args)
        );
        assert_eq!(
            rows[0].tool_output_token_estimate,
            estimate_tool_output_tokens(&result.content)
        );
        Ok(())
    }

    #[tokio::test]
    async fn passthrough_guard_returns_no_rejection() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        assert!(run_guards(&call, &[passthrough_guard]).await.is_none());
        Ok(())
    }

    #[test]
    fn production_pipeline_keeps_audit_in_the_observer_lane() {
        let effect_names = EFFECTS.iter().map(|effect| effect.name).collect::<Vec<_>>();
        assert_eq!(
            effect_names,
            [
                "ownership-claims",
                "tabs-list-view",
                "tab-activity",
                "tab-groups",
                "session-naming",
                "page-ownership-notice",
                "scope-notice",
                "helper-discovery",
            ]
        );
        // Ownership must never gate a dispatch. It is a label telling an agent whose
        // tab it is looking at, and agents are allowed to use the user's tabs and other
        // agents' tabs. This assertion exists so the guard that used to refuse them
        // cannot be reintroduced without someone deleting this line on purpose.
        assert_eq!(
            GUARDS.len(),
            2,
            "a guard was added. Ownership must not be one of them: it informs, it never blocks"
        );
        assert_eq!(
            OBSERVERS
                .iter()
                .map(|observer| observer.name)
                .collect::<Vec<_>>(),
            ["audit", "run_failure"]
        );
    }

    #[test]
    fn wire_result_strips_structured_content_unless_the_tool_has_an_output_schema() {
        // No output_schema: content-only envelope, structured content dropped.
        let stripped = wire_result(
            ToolResult::text("ok", Some(json!({ "page": 7, "secret": true }))),
            false,
        );
        assert_eq!(stripped.is_error, Some(false));
        assert_eq!(stripped.structured_content, None);
        assert_eq!(stripped.meta, None);

        // With output_schema (e.g. `run`): the promised structured content is delivered
        // so spec-compliant clients accept the result.
        let kept = wire_result(
            ToolResult::text("ok", Some(json!({ "ok": true, "logs": [] }))),
            true,
        );
        assert_eq!(
            kept.structured_content,
            Some(json!({ "ok": true, "logs": [] }))
        );
    }

    #[test]
    fn wire_result_drops_operator_cancellation_structured_content_for_schema_bearing_tools() {
        // A schema-bearing tool (e.g. `run`) cancelled mid-teardown must not forward the
        // cancellation envelope's structured content: it does not match the tool's
        // output_schema and a spec-compliant client would reject the whole result.
        let wire = wire_result(operator_cancellation_result(), false);
        assert_eq!(wire.is_error, Some(true));
        assert_eq!(wire.structured_content, None);
    }

    #[tokio::test]
    async fn operator_cancellation_returns_and_audits_operator_result() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let session = call
            .identity
            .as_ref()
            .unwrap_or_else(|| unreachable!())
            .session
            .clone();
        call.dispatch_cancel.cancel();
        let result = dispatch_tool_call_with(
            call.clone(),
            &[],
            &[],
            &[NamedToolObserver {
                name: "audit",
                run: observers::audit::apply,
            }],
        )
        .await
        .unwrap_or_else(|error| panic!("operator cancellation should stay in-band: {error:?}"));
        call.state
            .audit_worker
            .flush_session(call.session_id.as_str())
            .await?;
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result
                .content
                .first()
                .and_then(|block| block.as_text())
                .map(|text| text.text.as_str()),
            Some(CANCELLATION_REASON)
        );
        let rows = call
            .state
            .audit_log
            .list_dispatches(ListDispatchesQuery::default())
            .await?
            .rows;
        assert_eq!(rows.len(), 1);
        let meta: Value = serde_json::from_str(
            rows[0]
                .result_meta
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("result metadata missing"))?,
        )?;
        assert_eq!(meta["isError"], true);
        assert_eq!(meta["cancelled"], true);
        assert_eq!(meta["cancellationKind"], "cockpit.operator-cancelled");
        assert_eq!(rows[0].token_estimator_version, TOKEN_ESTIMATOR_VERSION);
        assert_eq!(
            rows[0].tool_input_token_estimate,
            estimate_tool_input_tokens(call.tool().name, &call.raw_args)
        );
        assert_eq!(
            rows[0].tool_output_token_estimate,
            estimate_tool_output_tokens(&result.content)
        );
        let summary = call
            .state
            .audit_log
            .get_task_summary(call.session_id.as_str())
            .await?
            .ok_or_else(|| anyhow::anyhow!("task summary missing"))?;
        assert_eq!(summary.error_count, 0);
        assert_eq!(session.stop_dispatches().await, 0);
        Ok(())
    }

    #[tokio::test]
    async fn client_cancellation_skips_effects_and_operator_result() -> anyhow::Result<()> {
        CLIENT_OBSERVER_CALLS.store(0, Ordering::SeqCst);
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let session = call
            .identity
            .as_ref()
            .unwrap_or_else(|| unreachable!())
            .session
            .clone();
        call.client_cancel.cancel();
        let result = dispatch_tool_call_with(
            call.clone(),
            &[],
            &[],
            &[
                NamedToolObserver {
                    name: "audit",
                    run: observers::audit::apply,
                },
                NamedToolObserver {
                    name: "count",
                    run: client_counting_observer,
                },
            ],
        )
        .await;
        let Err(error) = result else {
            panic!("client cancellation should be a protocol error");
        };
        assert_eq!(error.message.as_ref(), CLIENT_CANCELLATION_ERROR);
        assert!(
            call.state
                .audit_log
                .list_dispatches(ListDispatchesQuery::default())
                .await?
                .rows
                .is_empty()
        );
        assert_eq!(session.stop_dispatches().await, 0);
        assert!(call.dispatch_cancel.is_cancelled());
        assert_eq!(CLIENT_OBSERVER_CALLS.load(Ordering::SeqCst), 0);
        Ok(())
    }
}
