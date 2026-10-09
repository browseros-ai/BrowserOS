use crate::{
    AppState,
    api::mcp::{effects, guards, helper_runtime, observers},
    identity::ClientIdentity,
    ids::{ConvoId, DispatchId, SessionId},
    services::sessions::Session,
};
use browseros_core::{BrowserSession, PageId, pages::PageInfo};
use browseros_mcp::{
    BrowserToolDefaults, BrowserToolOptions, OutputFileAccess, ToolCtx, ToolDef, ToolResult,
    execute_tool,
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

#[derive(Clone)]
pub struct ToolCall {
    catalog: Arc<Vec<ToolDef>>,
    tool_index: usize,
    pub raw_args: Value,
    pub session_id: SessionId,
    pub identity: Option<ToolIdentity>,
    pub browser_session: Option<Arc<BrowserSession>>,
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
            foreign_pages: Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new())),
            catalog,
            tool_index,
            raw_args,
            session_id,
            identity,
            browser_session,
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

/// The same pipeline, returning the tool's own result instead of the wire envelope.
///
/// The envelope drops structured content for every tool that advertises no output
/// schema, which is correct for a client and wrong for a caller inside the server
/// that needs a flag the tool computed. Reading the envelope instead of this cost
/// the goal-driven loop its only feedback signal: `act` has no output schema, so
/// its `changed` flag was stripped on the way out and every action read as a no-op.
pub async fn dispatch_tool_call_in_process(call: ToolCall) -> Result<ToolResult, McpError> {
    dispatch_tool_call_in_process_with(call, GUARDS, EFFECTS, OBSERVERS).await
}

async fn dispatch_tool_call_in_process_with(
    call: ToolCall,
    guards: &[ToolGuard],
    effects: &[NamedToolEffect],
    observers: &[NamedToolObserver],
) -> Result<ToolResult, McpError> {
    dispatch_pipeline(call, guards, effects, observers)
        .await
        .map(|(result, _)| result)
}

async fn dispatch_tool_call_with(
    call: ToolCall,
    guards: &[ToolGuard],
    effects: &[NamedToolEffect],
    observers: &[NamedToolObserver],
) -> Result<CallToolResult, McpError> {
    let has_output_schema = call.tool().output_schema.is_some();
    let (result, content_only) = dispatch_pipeline(call, guards, effects, observers).await?;
    Ok(wire_result(result, has_output_schema && !content_only))
}

/// Runs the pipeline. The flag is set when the result is a dispatch-layer
/// envelope rather than the tool's promised output, which must stay content-only
/// even for a tool that advertises a schema.
async fn dispatch_pipeline(
    mut call: ToolCall,
    guards: &[ToolGuard],
    effects: &[NamedToolEffect],
    observers: &[NamedToolObserver],
) -> Result<(ToolResult, bool), McpError> {
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
    if teardown_before_finish && operator_stop_requested {
        let cancellation = operator_cancellation_result();
        call.dispatch_cancel.cancel();
        call.cancel.cancel();
        // The operator-cancellation envelope is a dispatch-layer error, not the tool's
        // promised output, so it must stay content-only even for schema-bearing tools;
        // forwarding its structured content would violate the tool's output_schema.
        return Ok((cancellation, true));
    }
    call.dispatch_cancel.cancel();
    call.cancel.cancel();
    result.map(|result| (result, false))
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
    let result = match &call.browser_session {
        Some(browser_session) => {
            // Script tools drive primitives straight against the shared browser
            // session, bypassing the guards and audit effect the pipeline runs
            // per tool. Inject a hook so each primitive is ownership-checked and
            // recorded as a child of this script's dispatch.
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
                        browser_session,
                    )
                    .await
                }
                _ => Vec::new(),
            };
            let ctx = ToolCtx::new(BrowserToolOptions {
                session: browser_session.clone(),
                defaults: BrowserToolDefaults {
                    default_window_id: None,
                    default_tab_group_id: call.default_tab_group_id.clone(),
                },
                cancel: call.cancel.clone(),
                output_files: call.output_files.clone(),
                inner_call_hook,
                preloaded_helpers,
            });
            match execute_tool(call.tool(), call.raw_args.clone(), &ctx).await {
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

    /// Stands in for the diff an action produces, so the test does not need a browser.
    fn navigated_effect(
        _context: ToolEffectContext<'_>,
    ) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
        Box::pin(async move {
            Ok(Some(ToolResult::text(
                "navigated",
                Some(json!({ "changed": true, "urlChanged": true })),
            )))
        })
    }

    /// The regression this closes: `act` advertises no output schema, so the wire
    /// envelope stripped the `changed` flag the goal-driven loop runs on, and every
    /// action including a navigation read as a no-op. A caller inside the server has
    /// to receive the tool's own result, not the client-facing envelope.
    ///
    /// Asserting through the real pipeline matters here. A test that hand-builds a
    /// result with structured content present passes while the bug is live.
    #[tokio::test]
    async fn an_in_process_dispatch_keeps_structured_content_the_envelope_strips()
    -> anyhow::Result<()> {
        let effects = [NamedToolEffect {
            name: "test_navigated",
            run: navigated_effect,
        }];
        let args = json!({ "page": 1, "kind": "click", "ref": "e1" });

        let call = crate::api::mcp::test_support::tool_call("act", args.clone()).await?;
        assert!(
            call.tool().output_schema.is_none(),
            "this test only means something while act advertises no output schema"
        );
        let enveloped = dispatch_tool_call_with(call, &[], &effects, &[])
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(
            enveloped.structured_content, None,
            "a client gets content only, which is correct for a tool with no schema"
        );

        let in_process = dispatch_tool_call_in_process_with(
            crate::api::mcp::test_support::tool_call("act", args).await?,
            &[],
            &effects,
            &[],
        )
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(
            in_process.structured_content,
            Some(json!({ "changed": true, "urlChanged": true })),
            "the flag the loop reads must survive an in-process dispatch"
        );
        Ok(())
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
