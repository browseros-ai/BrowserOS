use crate::{
    AppState,
    domain::{AgentKey, AgentRef, DispatchId, Session, SessionId},
    mcp::{effects, guards},
};
use browseros_core::{BrowserSession, PageId};
use browseros_mcp::{
    BrowserToolDefaults, BrowserToolOptions, OutputFileAccess, ToolCtx, ToolDef, ToolResult,
    execute_tool,
};
use futures_util::future::BoxFuture;
use rmcp::{
    ErrorData as McpError, RoleServer,
    model::{CallToolResult, ContentBlock, RequestId},
    service::Peer,
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use tracing::warn;

const CANCELLATION_REASON: &str = "Operation cancelled by the User";
const CLIENT_CANCELLATION_ERROR: &str = "Request cancelled by client";
const ARBITRARY_SCRIPT_TOOLS: &[&str] = &["run", "evaluate"];
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
    pub agent: AgentRef,
    pub ownership_key: AgentKey,
    pub agent_label: String,
}

#[derive(Clone)]
pub struct ToolCall {
    catalog: Arc<Vec<ToolDef>>,
    tool_index: usize,
    pub raw_args: Value,
    pub session_id: SessionId,
    pub request_id: RequestId,
    pub identity: Option<ToolIdentity>,
    pub browser_session: Option<Arc<BrowserSession>>,
    pub cancel: CancellationToken,
    pub client_cancel: CancellationToken,
    pub dispatch_cancel: CancellationToken,
    pub default_tab_group_id: Option<String>,
    pub flags: ToolFlags,
    pub state: AppState,
    pub dispatch_id: DispatchId,
    pub output_files: OutputFileAccess,
    pub peer: Option<Peer<RoleServer>>,
    pub naming_started: Arc<AtomicBool>,
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
        request_id: RequestId,
        identity: Option<ToolIdentity>,
        browser_session: Option<Arc<BrowserSession>>,
        cancel: CancellationToken,
        client_cancel: CancellationToken,
        dispatch_cancel: CancellationToken,
        default_tab_group_id: Option<String>,
        state: AppState,
        output_files: OutputFileAccess,
        peer: Option<Peer<RoleServer>>,
        naming_started: Arc<AtomicBool>,
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
            catalog,
            tool_index,
            raw_args,
            session_id,
            request_id,
            identity,
            browser_session,
            cancel,
            client_cancel,
            dispatch_cancel,
            default_tab_group_id,
            flags,
            state,
            dispatch_id: DispatchId::new(),
            output_files,
            peer,
            naming_started,
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

const GUARDS: &[ToolGuard] = &[
    guards::navigate_scheme::guard,
    guards::browser_connected::guard,
    guards::page_ownership::guard,
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
        name: "audit",
        run: effects::audit::apply,
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

/// Dispatches a tool through ordered guards, cancellation, and ordered effects.
pub async fn dispatch_tool_call(call: ToolCall) -> Result<CallToolResult, McpError> {
    dispatch_tool_call_with(call, GUARDS, EFFECTS).await
}

async fn dispatch_tool_call_with(
    call: ToolCall,
    guards: &[ToolGuard],
    effects: &[NamedToolEffect],
) -> Result<CallToolResult, McpError> {
    if let Some(identity) = &call.identity {
        identity
            .session
            .register_dispatch(call.dispatch_id.clone(), call.dispatch_cancel.clone())
            .await;
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
                Ok(run_effects(
                    ToolEffectContext {
                        call: &call,
                        result: &outcome.result,
                        cancelled: outcome.cancelled,
                        duration_ms: outcome.duration_ms,
                    },
                    effects,
                )
                .await)
            }
        }
    };

    if let Some(identity) = &call.identity {
        identity
            .session
            .unregister_dispatch(&call.dispatch_id)
            .await;
    }
    call.dispatch_cancel.cancel();
    call.cancel.cancel();
    result.map(wire_result)
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
            let ctx = ToolCtx::new(BrowserToolOptions {
                session: browser_session.clone(),
                defaults: BrowserToolDefaults {
                    default_window_id: None,
                    default_tab_group_id: call.default_tab_group_id.clone(),
                },
                cancel: call.cancel.clone(),
                output_files: call.output_files.clone(),
            });
            match execute_tool(call.tool(), call.raw_args.clone(), &ctx).await {
                Ok(result) => result,
                Err(browseros_mcp::framework::ToolError::Cancelled) => {
                    ToolResult::error(format!("{} failed: cancelled", call.tool().name))
                }
                Err(error) => ToolResult::error(format!("{} failed: {error}", call.tool().name)),
            }
        }
        None => ToolResult::error(
            "browser session not connected; the agent browser is not running or paired. Tell the user to start BrowserClaw and check the cockpit connection status; do not fall back to another browser tool.",
        ),
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

fn operator_cancellation_result() -> ToolResult {
    ToolResult {
        content: vec![ContentBlock::text(CANCELLATION_REASON)],
        is_error: true,
        structured_content: Some(json!({
            "cancellationReason": CANCELLATION_REASON,
            "cancellationKind": "cockpit.operator-cancelled"
        })),
    }
}

fn wire_result(result: ToolResult) -> CallToolResult {
    if result.is_error {
        CallToolResult::error(result.content)
    } else {
        CallToolResult::success(result.content)
    }
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

fn dispatch_error_text(result: &ToolResult) -> Option<String> {
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
    use crate::services::audit::ListDispatchesQuery;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    static EFFECT_CALLS: AtomicUsize = AtomicUsize::new(0);
    static EFFECT_ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

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
            Ok(Some(ToolResult::text("replacement", None)))
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
            crate::mcp::test_support::tool_call("tabs", Value::Object(serde_json::Map::new()))
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
        let call = crate::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
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
        let call = crate::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
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
    async fn guard_rejection_skips_effects() -> anyhow::Result<()> {
        EFFECT_CALLS.store(0, Ordering::SeqCst);
        let call = crate::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let result = dispatch_tool_call_with(
            call,
            &[rejecting_guard],
            &[NamedToolEffect {
                name: "count",
                run: counting_effect,
            }],
        )
        .await
        .unwrap_or_else(|error| panic!("guard rejection should stay in-band: {error:?}"));
        assert_eq!(result.is_error, Some(true));
        assert_eq!(EFFECT_CALLS.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test]
    async fn passthrough_guard_returns_no_rejection() -> anyhow::Result<()> {
        let call = crate::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        assert!(run_guards(&call, &[passthrough_guard]).await.is_none());
        Ok(())
    }

    #[test]
    fn production_effects_run_in_ts_pipeline_order() {
        let names = EFFECTS.iter().map(|effect| effect.name).collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "ownership-claims",
                "tabs-list-view",
                "audit",
                "tab-activity",
                "tab-groups",
                "session-naming",
            ]
        );
    }

    #[test]
    fn wire_result_strips_structured_content_and_metadata() {
        let result = wire_result(ToolResult::text(
            "ok",
            Some(json!({ "page": 7, "secret": true })),
        ));
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.structured_content, None);
        assert_eq!(result.meta, None);
    }

    #[tokio::test]
    async fn operator_cancellation_returns_and_audits_operator_result() -> anyhow::Result<()> {
        let call = crate::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
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
            &[NamedToolEffect {
                name: "audit",
                run: effects::audit::apply,
            }],
        )
        .await
        .unwrap_or_else(|error| panic!("operator cancellation should stay in-band: {error:?}"));
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
            .audit
            .list_dispatches(ListDispatchesQuery::default())
            .await?
            .rows;
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0]
                .result_meta
                .as_deref()
                .is_some_and(|meta| { meta.contains("cancellationKind") })
        );
        assert_eq!(session.cancel_active_dispatches().await, 0);
        Ok(())
    }

    #[tokio::test]
    async fn client_cancellation_skips_effects_and_operator_result() -> anyhow::Result<()> {
        let call = crate::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
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
            &[NamedToolEffect {
                name: "audit",
                run: effects::audit::apply,
            }],
        )
        .await;
        let Err(error) = result else {
            panic!("client cancellation should be a protocol error");
        };
        assert_eq!(error.message.as_ref(), CLIENT_CANCELLATION_ERROR);
        assert!(
            call.state
                .audit
                .list_dispatches(ListDispatchesQuery::default())
                .await?
                .rows
                .is_empty()
        );
        assert_eq!(session.cancel_active_dispatches().await, 0);
        assert!(call.dispatch_cancel.is_cancelled());
        Ok(())
    }
}
