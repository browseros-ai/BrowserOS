//! Read-side projection for the live-session cockpit.
//!
//! Connected sessions drive inclusion. Durable Chrome-tab ownership is then reconciled against
//! one current browser snapshot, with activity joined only as metadata. This
//! keeps historical session reads on the audit path and prevents stale page or target identities
//! from becoming public API.

use crate::{
    db::audit_log::TaskSummary,
    error::{AppError, AppResult},
    services::{
        browser::{BrowserService, hex_for_slug},
        cockpit::{TabActivityService, ToolEvent},
        profiles::{ProfileService, StoredAgentProfile},
        sessions::{Session, Sessions},
    },
};
use browseros_core::pages::PageInfo;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use crate::db::{AuditLog, SessionTabLedger};

#[derive(Debug, Clone, Default)]
pub struct LiveSessionFilters {
    pub profile_id: Option<String>,
    pub slug: Option<String>,
    pub site: Option<String>,
    pub search: Option<String>,
    pub since: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveActivityState {
    Active,
    Idle,
}

#[derive(Debug, Clone)]
pub struct LiveTabProjection {
    pub browser_tab_id: i64,
    pub url: String,
    pub title: String,
    pub first_activity_at: Option<i64>,
    pub last_activity_at: Option<i64>,
    pub last_tool_name: Option<String>,
    pub tool_count: i64,
    pub recent_tools: Vec<ToolEvent>,
}

#[derive(Debug, Clone)]
pub struct LiveStateProjection {
    pub state: LiveActivityState,
    pub browser_tabs: Vec<LiveTabProjection>,
}

#[derive(Debug, Clone)]
pub struct LiveSessionProjection {
    pub task: TaskSummary,
    pub profile_id: Option<String>,
    pub harness: Option<String>,
    pub color: String,
    pub label: String,
    pub name: String,
    pub live: LiveStateProjection,
}

struct ProjectedTab {
    ownership_id: i64,
    session_id: String,
    active: bool,
    tab: LiveTabProjection,
}

pub struct CockpitQuery {
    sessions: Arc<Sessions>,
    profiles: Arc<ProfileService>,
    audit_log: Arc<AuditLog>,
    session_tabs: Arc<SessionTabLedger>,
    browser: Arc<BrowserService>,
    tab_activity: Arc<TabActivityService>,
}

impl CockpitQuery {
    #[must_use]
    pub fn new(
        sessions: Arc<Sessions>,
        profiles: Arc<ProfileService>,
        audit_log: Arc<AuditLog>,
        session_tabs: Arc<SessionTabLedger>,
        browser: Arc<BrowserService>,
        tab_activity: Arc<TabActivityService>,
    ) -> Self {
        Self {
            sessions,
            profiles,
            audit_log,
            session_tabs,
            browser,
            tab_activity,
        }
    }

    pub async fn list(
        &self,
        filters: &LiveSessionFilters,
    ) -> AppResult<Vec<LiveSessionProjection>> {
        let sessions = self.sessions.snapshot().await;
        let profiles = self.profiles.list_profiles().await?;
        let mut projected = Vec::with_capacity(sessions.len());

        for session in sessions {
            let task = self
                .audit_log
                .get_task_summary(session.id().as_str())
                .await?
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "live session {} has no audit summary",
                        session.id().as_str()
                    ))
                })?;
            let profile = matched_profile(&session, &profiles);
            let projection = LiveSessionProjection {
                label: profile
                    .map(|profile| profile.name.clone())
                    .unwrap_or_else(|| task.agent_label.clone()),
                name: session.label().await,
                profile_id: profile.map(|profile| profile.id.clone()),
                harness: profile.map(|profile| profile.harness.to_string()),
                color: hex_for_slug(session.agent().slug()).to_string(),
                task,
                live: LiveStateProjection {
                    state: LiveActivityState::Idle,
                    browser_tabs: Vec::new(),
                },
            };
            if matches_filters(&projection, filters) {
                projected.push(projection);
            }
        }

        self.session_tabs.drain_writes().await;
        let session_ids = projected
            .iter()
            .map(|projected| projected.task.session_id.clone())
            .collect::<Vec<_>>();
        let ownership = self
            .session_tabs
            .list_open_session_tabs(&session_ids)
            .await?;
        let Some(pages) = self.current_pages().await else {
            let connected = self.connected_session_ids().await;
            return Ok(projected
                .into_iter()
                .filter(|projected| connected.contains(&projected.task.session_id))
                .collect());
        };
        let pages_by_tab = pages
            .iter()
            .map(|page| (page.tab_id.0, page))
            .collect::<HashMap<_, _>>();
        let activity = self.tab_activity.reconcile_pages(&pages).await;
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
            let tab = LiveTabProjection {
                browser_tab_id: ownership.tab_id,
                url: page.url.clone(),
                title: page.title.clone(),
                first_activity_at: record.map(|record| record.first_tool_at),
                last_activity_at: record.map(|record| record.last_tool_at),
                last_tool_name: record.map(|record| record.last_tool_name.clone()),
                tool_count: record
                    .map(|record| i64::try_from(record.tool_count).unwrap_or(i64::MAX))
                    .unwrap_or(0),
                recent_tools: record
                    .map(|record| record.recent_tools.clone())
                    .unwrap_or_default(),
            };
            tab_candidates.push(ProjectedTab {
                ownership_id: ownership.id,
                session_id: ownership.session_id,
                active,
                tab,
            });
        }

        self.session_tabs.drain_writes().await;
        let current_ownership_ids = self
            .session_tabs
            .list_open_session_tabs(&session_ids)
            .await?
            .into_iter()
            .map(|ownership| ownership.id)
            .collect::<HashSet<_>>();
        let connected = self.connected_session_ids().await;
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
        Ok(projected
            .into_iter()
            .filter(|projected| connected.contains(&projected.task.session_id))
            .map(|mut projected| {
                let candidates = tabs_by_session
                    .remove(&projected.task.session_id)
                    .unwrap_or_default();
                projected.live.state = if candidates.iter().any(|candidate| candidate.active) {
                    LiveActivityState::Active
                } else {
                    LiveActivityState::Idle
                };
                projected.live.browser_tabs = candidates
                    .into_iter()
                    .map(|candidate| candidate.tab)
                    .collect();
                projected.live.browser_tabs.sort_by(|left, right| {
                    right
                        .last_activity_at
                        .cmp(&left.last_activity_at)
                        .then_with(|| left.browser_tab_id.cmp(&right.browser_tab_id))
                });
                projected
            })
            .collect())
    }

    async fn connected_session_ids(&self) -> HashSet<String> {
        self.sessions
            .snapshot()
            .await
            .into_iter()
            .map(|session| session.id().as_str().to_string())
            .collect()
    }

    async fn current_pages(&self) -> Option<Vec<PageInfo>> {
        let browser = self.browser.session().await?;
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

fn matches_filters(projection: &LiveSessionProjection, filters: &LiveSessionFilters) -> bool {
    if filters
        .profile_id
        .as_ref()
        .is_some_and(|profile_id| projection.profile_id.as_ref() != Some(profile_id))
        || filters
            .slug
            .as_ref()
            .is_some_and(|slug| &projection.task.slug != slug)
        || filters
            .site
            .as_ref()
            .is_some_and(|site| projection.task.site.as_ref() != Some(site))
        || filters
            .since
            .is_some_and(|since| projection.task.started_at < since)
    {
        return false;
    }
    filters.search.as_ref().is_none_or(|search| {
        let search = search.to_ascii_lowercase();
        projection.task.title.to_ascii_lowercase().contains(&search)
            || projection.name.to_ascii_lowercase().contains(&search)
            || projection.label.to_ascii_lowercase().contains(&search)
            || projection.task.slug.to_ascii_lowercase().contains(&search)
            || projection
                .task
                .site
                .as_ref()
                .is_some_and(|site| site.to_ascii_lowercase().contains(&search))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assemble_query(
        sessions: Arc<Sessions>,
        profiles: Arc<ProfileService>,
        audit_log: Arc<AuditLog>,
        session_tabs: Arc<SessionTabLedger>,
        browser: Arc<BrowserService>,
        tab_activity: Arc<TabActivityService>,
    ) -> CockpitQuery {
        CockpitQuery::new(
            sessions,
            profiles,
            audit_log,
            session_tabs,
            browser,
            tab_activity,
        )
    }

    #[test]
    fn constructor_accepts_explicit_dependencies() {
        let _constructor = assemble_query;
    }
}
