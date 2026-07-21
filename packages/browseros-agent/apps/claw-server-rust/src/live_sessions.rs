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
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

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

struct ProjectedTab {
    ownership_id: i64,
    session_id: String,
    active: bool,
    tab: SessionBrowserTab,
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
    let Some(pages) = current_pages(state).await else {
        let connected = state
            .sessions
            .snapshot()
            .await
            .into_iter()
            .map(|session| session.id().as_str().to_string())
            .collect::<HashSet<_>>();
        let items = projected
            .into_iter()
            .filter(|projected| connected.contains(&projected.summary.session_id))
            .map(|mut projected| {
                projected.summary.live = Some(Box::new(LiveSessionState::new(
                    LiveSessionActivityState::Idle,
                    Vec::new(),
                )));
                projected.summary
            })
            .collect();
        return Ok(SessionList::new(items));
    };
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
    let mut tab_candidates = Vec::new();
    for ownership in ownership {
        let Some(page) = pages_by_tab.get(&ownership.tab_id) else {
            continue;
        };
        let record = activity_by_incarnation.get(&(
            ownership.session_id.as_str(),
            ownership.tab_id,
            page.page_id.0,
            page.target_id.as_str(),
        ));
        let active = record.is_some_and(|record| record.status == "active");
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
            .frame_for(
                &ownership.session_id,
                page.page_id.0,
                page.target_id.as_str(),
            )
            .await
            .filter(|frame| !frame.jpeg_base64.is_empty())
            .map(|frame| frame.captured_at);
        tab_candidates.push(ProjectedTab {
            ownership_id: ownership.id,
            session_id: ownership.session_id,
            active,
            tab,
        });
    }

    state.audit.drain_claim_writes().await;
    let current_ownership_ids = state
        .audit
        .list_open_session_tabs(&session_ids)
        .await?
        .into_iter()
        .map(|ownership| ownership.id)
        .collect::<HashSet<_>>();
    let connected = state
        .sessions
        .snapshot()
        .await
        .into_iter()
        .map(|session| session.id().as_str().to_string())
        .collect::<HashSet<_>>();
    let mut tabs_by_session = tab_candidates
        .into_iter()
        .filter(|candidate| current_ownership_ids.contains(&candidate.ownership_id))
        .fold(
            HashMap::<String, Vec<ProjectedTab>>::new(),
            |mut by_session, candidate| {
                by_session
                    .entry(candidate.session_id.clone())
                    .or_default()
                    .push(candidate);
                by_session
            },
        );
    let items = projected
        .into_iter()
        .filter(|projected| connected.contains(&projected.summary.session_id))
        .map(|mut projected| {
            let candidates = tabs_by_session
                .remove(&projected.summary.session_id)
                .unwrap_or_default();
            let active = candidates.iter().any(|candidate| candidate.active);
            let mut browser_tabs = candidates
                .into_iter()
                .map(|candidate| candidate.tab)
                .collect::<Vec<_>>();
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
            projected.summary
        })
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
    let Some(pages) = current_pages(state).await else {
        return Ok(None);
    };
    let Some(page) = pages.iter().find(|page| page.tab_id.0 == browser_tab_id) else {
        return Ok(None);
    };
    let page_id = page.page_id.0;
    let target_id = page.target_id.as_str().to_string();
    let candidate = state
        .screencast
        .frame_for(session_id, page_id, &target_id)
        .await;
    let Some(current_pages) = current_pages(state).await else {
        return Ok(None);
    };
    if !current_pages.iter().any(|page| {
        page.tab_id.0 == browser_tab_id
            && page.page_id.0 == page_id
            && page.target_id.as_str() == target_id.as_str()
    }) {
        return Ok(None);
    }
    // Browser and cache reads establish the incarnation first. Durable ownership and connected
    // liveness are checked afterward so session authority is the return boundary.
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

async fn current_pages(state: &AppState) -> Option<Vec<PageInfo>> {
    let browser = state.browser.session().await?;
    if !browser.is_connected() {
        return None;
    }
    match browser.pages.list().await {
        Ok(pages) => Some(pages),
        Err(error) => {
            tracing::warn!(error = %error, "failed to reconcile live browser pages");
            None
        }
    }
}
