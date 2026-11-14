//! Projects the SDK's task holders into host ownership, cockpit activity and tab groups.
//! The SDK is authoritative. Its implicit host task is absent from run metadata, so a
//! read-only, unbound census supplies holders even when the agent returned no task touches.

use std::{collections::HashSet, sync::Arc, time::Duration};

use browseros_core::{BrowserSession, pages::PageInfo, target::BrowserView};
use browseros_mcp::output_file::create_browser_output_file_access;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::super::{
    dispatch::{CreatedPageCapture, ToolCall, ToolIdentity},
    effects::{ownership_claims, tab_groups},
    service::StartedSession,
    tab_ref,
};
use crate::{
    AppState,
    services::{
        cockpit::RecordToolInput,
        neo::{RunCall, RunOperation, RunStatus, TaskKey},
        sessions::ClawTargetStore,
    },
};

const CENSUS_CODE: &str = "return context.pages().map(p => ({ tabId: p.tabId, taskId: p.taskId }))";
const CENSUS_WAIT: Duration = Duration::from_secs(5);

/// Current holders, rather than historical task touches from the SDK's tabs API.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CensusTab {
    tab_id: i64,
    task_id: Option<TaskKey>,
}

/// Only first claims trigger grouping. Keeping this decision independent of browser
/// writes prevents a later run from undoing the agent's own group placement.
fn plan_claims(census: &[CensusTab], tasks: &[TaskKey], claimed: &HashSet<i64>) -> Vec<i64> {
    let tasks: HashSet<_> = tasks.iter().collect();
    let mut seen = claimed.clone();
    census
        .iter()
        .filter(|tab| {
            tab.task_id
                .as_ref()
                .is_some_and(|task| tasks.contains(task))
        })
        .filter_map(|tab| seen.insert(tab.tab_id).then_some(tab.tab_id))
        .collect()
}

/// Only a successful opener proves the tab did not exist before this run.
/// Keep replay's exact ownership filter by moving that tab's first claim to
/// the opener's start, before its initial recorder checkpoint can be emitted.
fn claim_start(tab_id: i64, operations: &[RunOperation], run_started_at: i64, now: i64) -> i64 {
    operations
        .iter()
        .filter(|operation| operation.opened_tab() == Some(tab_id))
        .map(|operation| operation.started_at)
        .min()
        .map_or(now, |started_at| started_at.max(run_started_at).min(now))
}

pub(in crate::api::mcp) async fn apply(
    state: &AppState,
    started: &StartedSession,
    operations: &[RunOperation],
    run_started_at: i64,
) -> Vec<String> {
    let cancel = started.session.child_token();
    let census = tokio::select! {
        biased;
        () = cancel.cancelled() => return Vec::new(),
        census = tokio::time::timeout(CENSUS_WAIT, census(state)) => census,
    };
    let census = match census {
        Ok(Ok(census)) => census,
        _ => {
            tracing::warn!("SDK tab ownership census failed");
            return vec!["note: tab ownership could not be refreshed; it will be retried after the next run.".to_owned()];
        }
    };
    let Some(browser) = state.browser.session().await else {
        return Vec::new();
    };
    let convo = started.session.convo_id();
    let tasks = state.neo.session_tasks(convo);
    let held = plan_claims(&census, &tasks, &HashSet::new());
    let mut pages = Vec::new();
    let mut missing = Vec::new();
    for tab in held {
        match resolve_page(state, &browser, tab).await {
            Some(page) => pages.push(page),
            None => missing.push(tab),
        }
    }
    // The SDK and host learn tabs through separate CDP connections. Reconcile once
    // if the SDK's census beat the host's registry, then leave later arrivals for
    // the next run rather than inventing a PageId or retrying without a bound.
    if !missing.is_empty() && browser.pages.list().await.is_ok() {
        for tab in missing {
            if let Some(page) = resolve_page(state, &browser, tab).await {
                pages.push(page);
            }
        }
    }
    let identity = ToolIdentity {
        session: started.session.clone(),
        agent: started.session.agent().clone(),
        ownership_key: convo.clone(),
        agent_label: started.agent_label.clone(),
    };
    let ownership = state.sessions.ownership();
    let operation_lock = ownership.group_operation_lock(convo).await;
    let mut claimed = HashSet::new();
    let mut newly_claimed = Vec::new();
    {
        // Claim admission shares the conversation's group lock so overlapping
        // runs cannot both schedule first grouping for the same native tab.
        // The grouper takes this lock itself, after claim admission releases it.
        let _guard = operation_lock.lock().await;
        if cancel.is_cancelled() {
            return Vec::new();
        }
        let owned = ownership.owned_pages(convo).await;
        for page in &pages {
            if owned.contains(&page.page_id) {
                claimed.insert(page.tab_id.0);
            }
        }
        let additions = plan_claims(&census, &tasks, &claimed);
        for page in &pages {
            if additions.contains(&page.tab_id.0) {
                ownership_claims::record_new_page(
                    state,
                    &identity,
                    Some(&browser),
                    started.session.id().as_str(),
                    page.page_id.0,
                    claim_start(
                        page.tab_id.0,
                        operations,
                        run_started_at,
                        crate::clock::now_epoch_ms(),
                    ),
                )
                .await;
                newly_claimed.push(page);
            }
        }
    }
    if !newly_claimed.is_empty() {
        let catalog = Arc::new(browseros_mcp::catalog());
        if let Some(index) = catalog.iter().position(|tool| tool.name == "tab_groups") {
            // SDK run deliberately ignores legacy saved browser targets. This view
            // follows the SDK's actual tabs, through the existing grouping effect.
            let call = ToolCall::new(
                catalog,
                index,
                json!({}),
                started.session.id().clone(),
                Some(identity),
                Some(browser.clone()),
                Some(BrowserView::unbound(browser.clone(), None)),
                Arc::new(ClawTargetStore {
                    bindings: state.targets.clone(),
                    session: started.session.id().clone(),
                }),
                cancel.clone(),
                CancellationToken::new(),
                CancellationToken::new(),
                None,
                state.clone(),
                create_browser_output_file_access(),
            );
            // Cosmetic browser work must not hold an SDK result behind slow group
            // operations. Each page retains its census incarnation, and the group
            // effect rechecks placement and session cancellation after admission.
            let newly_claimed: Vec<_> = newly_claimed.into_iter().cloned().collect();
            drop(tokio::spawn(async move {
                for page in newly_claimed {
                    group_new_claim(&call, &browser, &page).await;
                }
            }));
        }
    }
    // The shared first-claim helper already records the tab's creation activity.
    // Only previously held tabs need a separate run event, or one run counts twice.
    for page in pages
        .into_iter()
        .filter(|page| claimed.contains(&page.tab_id.0))
    {
        state
            .tab_activity
            .record_tool(RecordToolInput {
                target_id: page.target_id,
                tab_id: page.tab_id.0,
                page_id: page.page_id.0,
                session_id: started.session.id().as_str().to_owned(),
                agent_id: convo.as_str().to_owned(),
                slug: started.session.agent().slug().to_owned(),
                tool_name: "run".to_owned(),
            })
            .await;
    }
    Vec::new()
}

/// Retains first-claim identity and placement through the existing group's async handoff.
async fn group_new_claim(call: &ToolCall, browser: &Arc<BrowserSession>, page: &PageInfo) {
    // A script can place its tab before its first claim reaches the host.
    // Refresh native placement and retain this incarnation through browser awaits.
    let Ok(Some(current)) = browser.pages.refresh(page.page_id.clone()).await else {
        return;
    };
    if current.tab_id != page.tab_id
        || current.target_id != page.target_id
        || current.group_id.is_some()
    {
        return;
    }
    let Ok(view) = BrowserView::unbound(browser.clone(), None).with_expected_created_page(&current)
    else {
        return;
    };
    // The captured path rechecks placement after taking its operation lock.
    // The legacy path can overwrite an explicit group placed while it waits.
    let window_id = current.window_id;
    let mut page_call = call.clone();
    page_call.created_page_snapshots.lock().await.insert(
        page.page_id.0,
        CreatedPageCapture {
            page: current,
            view: view.clone(),
        },
    );
    page_call.view = Some(view);
    tab_groups::run_tab_group_work(
        page_call,
        Some(tab_groups::CreatedPage {
            page_id: page.page_id.0,
            window_id,
        }),
    )
    .await;
}

async fn census(state: &AppState) -> Result<Vec<CensusTab>, ()> {
    let outcome = state
        .neo
        .run(RunCall {
            args: json!({"code": CENSUS_CODE, "timeout": 4_000})
                .as_object()
                .cloned()
                .unwrap_or_default(),
            task: None,
            host_data: Value::Null,
        })
        .await
        .map_err(|_| ())?;
    if outcome.status != RunStatus::Ok || outcome.reply.is_error.unwrap_or(false) {
        return Err(());
    }
    let value = outcome
        .reply
        .structured_content
        .and_then(|mut content| content.get_mut("value").map(Value::take))
        .ok_or(())?;
    serde_json::from_value(value).map_err(|_| ())
}

async fn resolve_page(state: &AppState, browser: &BrowserSession, tab_id: i64) -> Option<PageInfo> {
    let page = tab_ref::page_for_tab(state, tab_id).await?;
    browser
        .pages
        .get_info(page)
        .await
        .filter(|page| page.tab_id.0 == tab_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::mcp::test_support::{ProfileConnection, current_session, tool_call};
    use crate::ids::ConvoId;

    #[test]
    fn first_claim_uses_only_proven_creation_within_the_host_run() -> anyhow::Result<()> {
        let operation = |name: &str, outcome: &str, tab_id: i64, started_at: i64| {
            json!({
                "id": format!("{name}:{tab_id}:{started_at}"),
                "name": name,
                "outcome": outcome,
                "page": {"tabId": tab_id},
                "startedAt": started_at,
                "effect": "write",
                "delivery": "done"
            })
        };
        let first = operation("context.newPage", "ok", 1, 120);
        let second = operation("task.newPage", "ok", 2, 180);
        let cases = [
            ("created in this run", 1, vec![first.clone()], 120),
            (
                "each tab keeps its own creation",
                2,
                vec![first.clone(), second],
                180,
            ),
            (
                "failed open",
                1,
                vec![operation("context.newPage", "error", 1, 120)],
                300,
            ),
            (
                "cancelled open",
                1,
                vec![operation("context.newPage", "cancelled", 1, 120)],
                300,
            ),
            (
                "future outcome",
                1,
                vec![operation("context.newPage", "future_outcome", 1, 120)],
                300,
            ),
            (
                "existing tab result",
                1,
                vec![operation("page.goto", "ok", 1, 120)],
                300,
            ),
            ("no creation proof", 1, vec![], 300),
            ("another tab was created", 2, vec![first.clone()], 300),
            (
                "start predates this run",
                1,
                vec![operation("context.newPage", "ok", 1, 90)],
                100,
            ),
            (
                "start follows now",
                1,
                vec![operation("context.newPage", "ok", 1, 400)],
                300,
            ),
            (
                "nested task and context opens keep the earliest start",
                1,
                vec![first, operation("task.newPage", "ok", 1, 110)],
                110,
            ),
            (
                "an input tab is not creation proof",
                1,
                vec![json!({
                    "id": "run:1",
                    "name": "context.newPage",
                    "outcome": "ok",
                    "tabId": 1,
                    "startedAt": 120,
                    "effect": "write",
                    "delivery": "done"
                })],
                300,
            ),
        ];
        for (label, tab_id, records, expected) in cases {
            let operations = records
                .into_iter()
                .map(serde_json::from_value)
                .collect::<Result<Vec<RunOperation>, _>>()?;
            assert_eq!(
                claim_start(tab_id, &operations, 100, 300),
                expected,
                "{label}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn explicit_placement_during_first_group_handoff_survives() -> anyhow::Result<()> {
        let transport = ProfileConnection::new();
        let browser = current_session(transport.clone());
        let page = browser.pages.list().await?.remove(0);
        let mut call = tool_call("tab_groups", json!({})).await?;
        call.browser_session = Some(browser.clone());
        call.view = Some(BrowserView::unbound(browser.clone(), None));
        let convo = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("session identity missing"))?
            .ownership_key
            .clone();
        let ownership = call.state.sessions.ownership();
        let operation_lock = ownership.group_operation_lock(&convo).await;
        let guard = operation_lock.lock().await;
        let waiting_call = call.clone();
        let waiting_browser = browser.clone();
        let waiting_page = page.clone();
        let work = tokio::spawn(async move {
            group_new_claim(&waiting_call, &waiting_browser, &waiting_page).await;
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !call
                .created_page_snapshots
                .lock()
                .await
                .contains_key(&page.page_id.0)
            {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        let explicit = BrowserView::unbound(browser, None)
            .cdp(
                "Browser.createTabGroup",
                json!({"tabIds": [page.tab_id.0], "title": "agent placement"}),
                None,
            )
            .await?;
        drop(guard);
        tokio::time::timeout(Duration::from_secs(2), work).await??;
        let groups = transport.groups.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(
            groups.len(),
            1,
            "the host created another group after explicit placement"
        );
        assert_eq!(groups[0]["groupId"], explicit["group"]["groupId"]);
        assert_eq!(groups[0]["tabIds"], json!([page.tab_id.0]));
        assert_eq!(groups[0]["title"], "agent placement");
        Ok(())
    }

    #[test]
    fn claims_only_the_sessions_holders_including_its_implicit_host_task() {
        let host = TaskKey::for_session(&ConvoId::new("host"));
        let explicit = TaskKey::for_session(&ConvoId::new("explicit"));
        let other = TaskKey::for_session(&ConvoId::new("other"));
        let census = vec![
            CensusTab {
                tab_id: 1,
                task_id: Some(host.clone()),
            },
            CensusTab {
                tab_id: 2,
                task_id: Some(explicit.clone()),
            },
            CensusTab {
                tab_id: 3,
                task_id: None,
            },
            CensusTab {
                tab_id: 4,
                task_id: Some(other),
            },
        ];
        assert_eq!(
            plan_claims(&census, &[host, explicit], &HashSet::new()),
            vec![1, 2]
        );
    }

    #[test]
    fn repeated_census_rows_and_existing_claims_never_schedule_regrouping() {
        let host = TaskKey::for_session(&ConvoId::new("host"));
        let census = vec![
            CensusTab {
                tab_id: 1,
                task_id: Some(host.clone()),
            },
            CensusTab {
                tab_id: 2,
                task_id: Some(host.clone()),
            },
            CensusTab {
                tab_id: 2,
                task_id: Some(host.clone()),
            },
        ];
        assert_eq!(
            plan_claims(&census, std::slice::from_ref(&host), &HashSet::from([1])),
            vec![2]
        );
        assert!(plan_claims(&census, &[host], &HashSet::from([1, 2])).is_empty());
    }

    #[test]
    fn closed_or_unobserved_tasks_do_not_claim_tabs() {
        let task = TaskKey::for_session(&ConvoId::new("closed"));
        let census = vec![CensusTab {
            tab_id: 1,
            task_id: Some(task),
        }];
        assert!(plan_claims(&census, &[], &HashSet::new()).is_empty());
    }
}
