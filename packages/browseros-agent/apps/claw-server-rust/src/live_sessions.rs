//! Read-side projection for the live-session cockpit.
//!
//! Connected sessions drive inclusion. Durable Chrome-tab ownership is then reconciled against
//! one current browser snapshot, with activity and screencast data joined only as metadata. This
//! keeps historical session reads on the audit path and prevents stale page or target identities
//! from becoming public API.

use crate::{
    AppState,
    agents::StoredAgentProfile,
    capture::audit::{TaskStatus, TaskSummary},
    error::{AppError, AppResult},
    sessions::Session,
    tabs::{activity::ScreencastFrame, hex_for_slug},
};
use browseros_core::pages::PageInfo;
use claw_api::models::{
    LiveSessionActivityState, LiveSessionState, SessionBrowserTab, SessionList, SessionStatus,
    SessionSummary, ToolEvent,
};
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Clone, Default)]
pub struct LiveSessionFilters {
    pub profile_id: Option<String>,
    pub slug: Option<String>,
    pub site: Option<String>,
    pub search: Option<String>,
    pub since: Option<i64>,
}

struct ProjectedSession {
    summary: SessionSummary,
}

pub async fn list(state: &AppState, filters: &LiveSessionFilters) -> AppResult<SessionList> {
    let sessions = state.sessions.snapshot().await;
    let profiles = state.agents.list_profiles().await?;
    let mut projected = Vec::with_capacity(sessions.len());

    for session in sessions {
        let task = state
            .audit
            .get_task_summary(session.id().as_str())
            .await?
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "live session {} has no audit summary",
                    session.id().as_str()
                ))
            })?;
        let task_title = task.title.clone();
        let profile = matched_profile(&session, &profiles);
        let mut summary = contract_summary(task, Some(&session)).await;
        summary.status = SessionStatus::Live;
        summary.profile_id = profile.map(|profile| profile.id.clone());
        summary.harness = profile.map(|profile| profile.harness.to_string());
        summary.color = Some(hex_for_slug(session.agent().slug()).to_string());
        if let Some(profile) = profile {
            summary.label.clone_from(&profile.name);
        }
        if matches_filters(&summary, &task_title, filters) {
            projected.push(ProjectedSession { summary });
        }
    }

    state.audit.drain_claim_writes().await;
    let session_ids = projected
        .iter()
        .map(|projected| projected.summary.session_id.clone())
        .collect::<Vec<_>>();
    let ownership = state.audit.list_open_session_tabs(&session_ids).await?;
    let pages = current_pages(state).await;
    let pages_by_tab = pages
        .iter()
        .map(|page| (page.tab_id.0, page))
        .collect::<HashMap<_, _>>();
    let activity = state.tab_activity.reconcile_pages(&pages).await;
    let activity_by_incarnation = activity
        .iter()
        .map(|record| {
            (
                (
                    record.session_id.as_str(),
                    record.tab_id,
                    record.page_id,
                    record.target_id.as_str(),
                ),
                record,
            )
        })
        .collect::<HashMap<_, _>>();
    let ownership_by_session =
        ownership
            .into_iter()
            .fold(HashMap::<String, Vec<_>>::new(), |mut by_session, row| {
                by_session
                    .entry(row.session_id.clone())
                    .or_default()
                    .push(row);
                by_session
            });

    for projected in &mut projected {
        let mut browser_tabs = Vec::new();
        let mut active = false;
        if let Some(rows) = ownership_by_session.get(&projected.summary.session_id) {
            for ownership in rows {
                let Some(page) = pages_by_tab.get(&ownership.tab_id) else {
                    continue;
                };
                let record = activity_by_incarnation.get(&(
                    projected.summary.session_id.as_str(),
                    ownership.tab_id,
                    page.page_id.0,
                    page.target_id.as_str(),
                ));
                active |= record.is_some_and(|record| record.status == "active");
                let recent_tools = record
                    .map(|record| {
                        record
                            .recent_tools
                            .iter()
                            .map(|event| ToolEvent::new(event.name.clone(), event.at))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut tab = SessionBrowserTab::new(
                    ownership.tab_id,
                    page.url.clone(),
                    page.title.clone(),
                    record
                        .map(|record| i64::try_from(record.tool_count).unwrap_or(i64::MAX))
                        .unwrap_or(0),
                    recent_tools,
                );
                if let Some(record) = record {
                    tab.first_activity_at = Some(record.first_tool_at);
                    tab.last_activity_at = Some(record.last_tool_at);
                    tab.last_tool_name = Some(record.last_tool_name.clone());
                }
                tab.preview_captured_at = state
                    .screencast
                    .frame_for(page.page_id.0, page.target_id.as_str())
                    .await
                    .filter(|frame| !frame.jpeg_base64.is_empty())
                    .map(|frame| frame.captured_at);
                browser_tabs.push(tab);
            }
        }
        browser_tabs.sort_by(|left, right| {
            right
                .last_activity_at
                .cmp(&left.last_activity_at)
                .then_with(|| left.browser_tab_id.cmp(&right.browser_tab_id))
        });
        projected.summary.live = Some(Box::new(LiveSessionState::new(
            if active {
                LiveSessionActivityState::Active
            } else {
                LiveSessionActivityState::Idle
            },
            browser_tabs,
        )));
    }

    let items = projected
        .into_iter()
        .map(|projected| projected.summary)
        .collect();
    Ok(SessionList::new(items))
}

pub async fn preview(
    state: &AppState,
    session_id: &str,
    browser_tab_id: i64,
) -> AppResult<Option<ScreencastFrame>> {
    let live_session_id = crate::ids::SessionId::new(session_id);
    if !state.sessions.contains(&live_session_id).await {
        return Ok(None);
    }
    state.audit.drain_claim_writes().await;
    if state
        .audit
        .open_session_tab(session_id, browser_tab_id)
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let pages = current_pages(state).await;
    let Some(page) = pages.iter().find(|page| page.tab_id.0 == browser_tab_id) else {
        return Ok(None);
    };
    let candidate = state
        .screencast
        .frame_for(page.page_id.0, page.target_id.as_str())
        .await;
    // Browser reconciliation and cache access cross async boundaries. Re-check connected
    // liveness and durable ownership afterward so teardown or reassignment cannot expose
    // target-incarnation bytes after the requested session's authority ends.
    state.audit.drain_claim_writes().await;
    let owns_tab = state
        .audit
        .open_session_tab(session_id, browser_tab_id)
        .await?
        .is_some();
    let connected = state.sessions.contains(&live_session_id).await;
    if !connected || !owns_tab {
        return Ok(None);
    }
    Ok(candidate)
}

pub async fn contract_summary(task: TaskSummary, live: Option<&Arc<Session>>) -> SessionSummary {
    let name = match live {
        Some(session) => session.label().await,
        None => task.title.clone(),
    };
    let mut summary = SessionSummary::new(
        task.session_id,
        task.slug,
        task.agent_label,
        name,
        task.started_at,
        task.duration_ms.max(0),
        task.dispatch_count,
        task.tool_sequence,
        match task.status {
            TaskStatus::Live => SessionStatus::Live,
            TaskStatus::Done => SessionStatus::Done,
            TaskStatus::Failed => SessionStatus::Failed,
        },
        task.error_count,
    );
    summary.profile_id = live
        .and_then(|session| session.agent().profile_id())
        .map(|profile_id| profile_id.as_str().to_string());
    summary.site = task.site;
    summary.ended_at = task.ended_at;
    summary.last_screenshot_dispatch_id = task.last_screenshot_dispatch_id;
    summary
}

fn matched_profile<'a>(
    session: &Session,
    profiles: &'a [StoredAgentProfile],
) -> Option<&'a StoredAgentProfile> {
    let profile_id = session.agent().profile_id()?;
    profiles
        .iter()
        .find(|profile| profile.id == profile_id.as_str())
}

fn matches_filters(
    summary: &SessionSummary,
    task_title: &str,
    filters: &LiveSessionFilters,
) -> bool {
    if filters
        .profile_id
        .as_ref()
        .is_some_and(|profile_id| summary.profile_id.as_ref() != Some(profile_id))
        || filters
            .slug
            .as_ref()
            .is_some_and(|slug| &summary.slug != slug)
        || filters
            .site
            .as_ref()
            .is_some_and(|site| summary.site.as_ref() != Some(site))
        || filters
            .since
            .is_some_and(|since| summary.started_at < since)
    {
        return false;
    }
    filters.search.as_ref().is_none_or(|search| {
        let search = search.to_ascii_lowercase();
        task_title.to_ascii_lowercase().contains(&search)
            || summary.name.to_ascii_lowercase().contains(&search)
            || summary.label.to_ascii_lowercase().contains(&search)
            || summary.slug.to_ascii_lowercase().contains(&search)
            || summary
                .site
                .as_ref()
                .is_some_and(|site| site.to_ascii_lowercase().contains(&search))
    })
}

async fn current_pages(state: &AppState) -> Vec<PageInfo> {
    let Some(browser) = state.browser.session().await else {
        return Vec::new();
    };
    if !browser.is_connected() {
        return Vec::new();
    }
    match browser.pages.list().await {
        Ok(pages) => pages,
        Err(error) => {
            tracing::warn!(error = %error, "failed to reconcile live browser pages");
            Vec::new()
        }
    }
}
