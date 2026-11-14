//! Projects named SDK tasks onto the host session label after tab reconciliation.
//! SDK task touches are the rename signal, including tasks created before a failed run.
//! Desktop rename reminders live here so pursue's internal dispatches cannot consume them.

use super::super::service::StartedSession;
use crate::{
    AppState,
    api::mcp::{
        effects::tab_groups::apply_agent_tab_group_title,
        hosted::rename_tip,
        naming::{normalize_small_name, session_group_title},
    },
    services::neo::{RunOutcome, RunStatus, TaskTouchKind},
};

/// Live naming and the parent audit row share one declaration rule so History
/// retains the same label, including when task creation precedes a run failure.
pub(in crate::api::mcp) fn created_task_name(outcome: &RunOutcome) -> Option<String> {
    outcome
        .tasks
        .iter()
        .filter(|task| task.kind == TaskTouchKind::Created)
        .filter_map(|task| task.name.as_deref())
        .map(normalize_small_name)
        .find(|name| !name.is_empty())
}

pub(in crate::api::mcp) async fn apply(
    state: &AppState,
    started: &StartedSession,
    outcome: &RunOutcome,
) -> Vec<String> {
    if outcome
        .tasks
        .iter()
        .any(|task| task.kind == TaskTouchKind::Created)
    {
        started.session.reset_pursue_handoff();
    }
    if let Some(name) = created_task_name(outcome) {
        started.session.rename(name.clone()).await;
        // Keep search metadata bounded to the same local label as the cockpit.
        // The script and its return value remain redacted in the run's audit row.
        if let Err(error) = state
            .audit_log
            .set_task_summary(started.session.id().as_str(), &name)
            .await
        {
            tracing::warn!(%error, "failed to store SDK task summary");
        }
        if state.config.behavior.tab_grouping.is_automatic() {
            let browser = state.browser.session().await;
            apply_agent_tab_group_title(
                browser.as_ref(),
                &state.sessions.ownership(),
                started.session.convo_id(),
                started.session.as_ref(),
                started.session.child_token(),
            )
            .await;
        }
    }
    // A failed script can still have created a named task, but should not spend a
    // reminder from an unnamed session while the agent is handling that failure.
    if outcome.status != RunStatus::Ok {
        return Vec::new();
    }
    let Some(label) = started.session.take_rename_nudge().await else {
        return Vec::new();
    };
    vec![rename_tip(
        state.config.mode,
        &session_group_title(&started.session, &label),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        api::mcp::test_support,
        config::{Behavior, ServerMode, TabGrouping},
        services::neo::{Effects, TaskTouch},
    };
    use rmcp::model::CallToolResult;
    use serde_json::json;

    async fn fixture(grouping: TabGrouping) -> anyhow::Result<(AppState, StartedSession)> {
        let call = test_support::tool_call_with_behavior(
            "run",
            json!({}),
            ServerMode::Desktop,
            Behavior {
                tab_grouping: grouping,
                ..Behavior::default()
            },
        )
        .await?;
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;
        let started = StartedSession {
            session: identity.session.clone(),
            agent_label: identity.agent_label.clone(),
        };
        call.state
            .audit_log
            .record_session_start(
                started.session.id().as_str(),
                started.session.convo_id().as_str(),
                started.session.agent().slug(),
                &started.agent_label,
                started.session.client_name(),
                "test",
            )
            .await?;
        Ok((call.state, started))
    }

    fn outcome(status: RunStatus, tasks: Vec<TaskTouch>) -> RunOutcome {
        RunOutcome {
            reply: CallToolResult::success(Vec::new()),
            run_id: "test-run".to_string(),
            status,
            effects: Effects::None,
            tasks,
        }
    }

    fn task(kind: TaskTouchKind, name: Option<&str>) -> anyhow::Result<TaskTouch> {
        Ok(TaskTouch {
            id: serde_json::from_value(json!("task-id"))?,
            name: name.map(str::to_string),
            kind,
            tabs: Vec::new(),
        })
    }

    #[tokio::test]
    async fn a_created_task_with_the_same_name_resets_pursue() -> anyhow::Result<()> {
        let (state, started) = fixture(TabGrouping::Automatic).await?;
        started
            .session
            .rename("invoice-processing".to_string())
            .await;
        started.session.mark_pursue_handed_back();
        apply(
            &state,
            &started,
            &outcome(
                RunStatus::Ok,
                vec![task(TaskTouchKind::Created, Some("invoice processing"))?],
            ),
        )
        .await;
        assert!(!started.session.pursue_handed_back());
        Ok(())
    }

    #[tokio::test]
    async fn a_reused_task_keeps_its_pursue_handback() -> anyhow::Result<()> {
        let (state, started) = fixture(TabGrouping::Automatic).await?;
        started
            .session
            .rename("invoice-processing".to_string())
            .await;
        started.session.mark_pursue_handed_back();
        apply(
            &state,
            &started,
            &outcome(
                RunStatus::Ok,
                vec![task(TaskTouchKind::Fetched, Some("invoice processing"))?],
            ),
        )
        .await;
        assert!(started.session.pursue_handed_back());
        Ok(())
    }

    #[tokio::test]
    async fn first_named_created_task_renames_even_when_the_script_later_fails()
    -> anyhow::Result<()> {
        let (state, started) = fixture(TabGrouping::Automatic).await?;
        let result = outcome(
            RunStatus::Error,
            vec![
                task(TaskTouchKind::Fetched, Some("old task"))?,
                task(TaskTouchKind::Closed, Some("finished task"))?,
                task(TaskTouchKind::Created, Some("!!!"))?,
                task(TaskTouchKind::Created, Some("Invoice Processing!"))?,
                task(TaskTouchKind::Created, Some("later task"))?,
            ],
        );
        assert!(apply(&state, &started, &result).await.is_empty());
        assert_eq!(started.session.label().await, "invoice-processing");
        let summary = state
            .audit_log
            .get_task_summary(started.session.id().as_str())
            .await?;
        assert_eq!(
            summary.and_then(|summary| summary.task_summary).as_deref(),
            Some("invoice-processing")
        );
        let group = state
            .sessions
            .ownership()
            .tab_group_state(started.session.convo_id())
            .await
            .ok_or_else(|| anyhow::anyhow!("group title missing"))?;
        assert_eq!(
            group.desired_title.as_deref(),
            Some("codex/invoice-processing")
        );
        assert!(
            apply(&state, &started, &outcome(RunStatus::Ok, Vec::new()))
                .await
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn explicit_grouping_keeps_the_session_name_without_retitling_groups()
    -> anyhow::Result<()> {
        let (state, started) = fixture(TabGrouping::Explicit).await?;
        let result = outcome(
            RunStatus::Ok,
            vec![task(TaskTouchKind::Created, Some("flight search"))?],
        );
        assert!(apply(&state, &started, &result).await.is_empty());
        assert_eq!(started.session.label().await, "flight-search");
        assert!(
            state
                .sessions
                .ownership()
                .tab_group_state(started.session.convo_id())
                .await
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn unnamed_successful_runs_get_five_reminders_and_errors_spend_none() -> anyhow::Result<()>
    {
        let (state, started) = fixture(TabGrouping::Automatic).await?;
        for status in [RunStatus::Error, RunStatus::Cancelled, RunStatus::Timeout] {
            assert!(
                apply(&state, &started, &outcome(status, Vec::new()))
                    .await
                    .is_empty()
            );
        }
        let result = outcome(RunStatus::Ok, Vec::new());
        for _ in 0..5 {
            let notes = apply(&state, &started, &result).await;
            assert_eq!(notes.len(), 1);
            assert!(notes[0].contains("neo.createTask("), "{}", notes[0]);
            assert_eq!(
                crate::api::mcp::surface::removed_tool_reference(&notes[0]),
                None,
                "{}",
                notes[0]
            );
        }
        assert!(apply(&state, &started, &result).await.is_empty());
        Ok(())
    }
}
