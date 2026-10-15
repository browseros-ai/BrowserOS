//! claw's audit observer inside the shared dispatch pipeline: what the audit timeline
//! records for final, cancelled and rejected dispatches. The pipeline mechanics are
//! tested in `browseros_mcp_host::mcp::dispatch`; these assert claw's rows.

use crate::{
    AppState,
    api::mcp::{
        dispatch::{
            CANCELLATION_REASON, CLIENT_CANCELLATION_ERROR, NamedToolEffect, NamedToolObserver,
            ToolEffectContext, ToolGuard, ToolObserverContext, dispatch_tool_call_with,
            run_effects, run_observers, wire_result,
        },
        effects,
        observers::audit::AuditObserver,
        test_support::{claw_state, claw_tool_call},
    },
    db::audit_log::ListDispatchesQuery,
};
use browseros_mcp::{
    ToolResult,
    token_estimate::{
        TOKEN_ESTIMATOR_VERSION, estimate_tool_input_tokens, estimate_tool_output_tokens,
    },
};
use browseros_mcp_host::seams::DispatchObserver;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::{
    Arc, LazyLock,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;

const NO_GUARDS: &[ToolGuard] = &[];
const NO_EFFECTS: &[NamedToolEffect] = &[];

static CLIENT_OBSERVER_CALLS: AtomicUsize = AtomicUsize::new(0);
static LATE_EFFECT_ENTERED: LazyLock<Notify> = LazyLock::new(Notify::new);
static LATE_EFFECT_RELEASE: LazyLock<Notify> = LazyLock::new(Notify::new);

fn audit(state: &AppState) -> AuditObserver {
    AuditObserver {
        audit_worker: state.audit_worker.clone(),
        audit_log: state.audit_log.clone(),
        visuals: state.visuals.clone(),
        screenshots: state.screenshots.clone(),
    }
}

fn audit_observer(state: &AppState) -> Arc<dyn DispatchObserver> {
    Arc::new(audit(state))
}

/// Audits, then parks until the test releases it, so Stop can win after the audit.
struct AuditThenWait(AuditObserver);

impl DispatchObserver for AuditThenWait {
    fn name(&self) -> &'static str {
        "audit"
    }

    fn observe<'a>(
        &'a self,
        context: ToolObserverContext<'a>,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            self.0.observe(context).await?;
            LATE_EFFECT_ENTERED.notify_one();
            LATE_EFFECT_RELEASE.notified().await;
            Ok(())
        })
    }
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

fn client_counting_observer(context: ToolObserverContext<'_>) -> BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let _ = context;
        CLIENT_OBSERVER_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
}

#[tokio::test]
async fn terminal_audit_counts_the_final_wire_content_once() -> anyhow::Result<()> {
    let (call, state) = claw_tool_call("tabs", json!({ "action": "list" })).await?;
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
        &[audit_observer(&state)],
    )
    .await;
    state
        .audit_worker
        .flush_session(call.session_id.as_str())
        .await?;
    let wire = wire_result(final_result.clone(), false);
    assert_eq!(wire.content.len(), 2);
    assert_eq!(wire.structured_content, None);

    let row = state
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
async fn stopped_session_rejects_browser_dispatch_before_effects() -> anyhow::Result<()> {
    let (call, state) = claw_tool_call("tabs", json!({ "action": "list" })).await?;
    let session = call
        .identity
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("tool call identity missing"))?
        .session
        .clone();
    session.stop_dispatches().await;

    let result = dispatch_tool_call_with(
        call.clone(),
        NO_GUARDS,
        NO_EFFECTS,
        &[audit_observer(&state)],
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
        state
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
    let (call, state) = claw_tool_call("tabs", json!({ "action": "list" })).await?;
    let session = call
        .identity
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("tool call identity missing"))?
        .session
        .clone();
    let dispatched_call = call.clone();
    let observer = Arc::new(AuditThenWait(audit(&state))) as Arc<dyn DispatchObserver>;
    let dispatch = tokio::spawn(async move {
        dispatch_tool_call_with(dispatched_call, NO_GUARDS, NO_EFFECTS, &[observer]).await
    });

    LATE_EFFECT_ENTERED.notified().await;
    let sessions = call.host.sessions().clone();
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
    let rows = state
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
async fn production_pipeline_keeps_audit_in_the_observer_lane() -> anyhow::Result<()> {
    let state = claw_state().await?;
    let effect_names = state.mcp.effect_names();
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
        state.mcp.guard_count(),
        2,
        "a guard was added. Ownership must not be one of them: it informs, it never blocks"
    );
    assert_eq!(state.mcp.observer_names(), ["audit", "run_failure"]);
    Ok(())
}

#[tokio::test]
async fn operator_cancellation_returns_and_audits_operator_result() -> anyhow::Result<()> {
    let (call, state) = claw_tool_call("tabs", json!({ "action": "list" })).await?;
    let session = call
        .identity
        .as_ref()
        .unwrap_or_else(|| unreachable!())
        .session
        .clone();
    call.dispatch_cancel.cancel();
    let result = dispatch_tool_call_with(
        call.clone(),
        NO_GUARDS,
        NO_EFFECTS,
        &[audit_observer(&state)],
    )
    .await
    .unwrap_or_else(|error| panic!("operator cancellation should stay in-band: {error:?}"));
    state
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
    let rows = state
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
    let summary = state
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
    let (call, state) = claw_tool_call("tabs", json!({ "action": "list" })).await?;
    let session = call
        .identity
        .as_ref()
        .unwrap_or_else(|| unreachable!())
        .session
        .clone();
    call.client_cancel.cancel();
    let result = dispatch_tool_call_with(
        call.clone(),
        NO_GUARDS,
        NO_EFFECTS,
        &[
            audit_observer(&state),
            Arc::new(NamedToolObserver {
                name: "count",
                run: client_counting_observer,
            }) as Arc<dyn DispatchObserver>,
        ],
    )
    .await;
    let Err(error) = result else {
        panic!("client cancellation should be a protocol error");
    };
    assert_eq!(error.message.as_ref(), CLIENT_CANCELLATION_ERROR);
    assert!(
        state
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
