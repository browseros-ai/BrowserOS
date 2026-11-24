//! Two cockpit read models: explicit `status=live` returns a complete browser-reconciled
//! snapshot; other lists return cursor-paginated audit history, enriched from a live
//! transport session only while one exists.

use super::{error, internal};
use crate::{
    AppState,
    clock::now_epoch_ms,
    db::{
        audit_log::{ListTasksQuery, TaskDetail, TaskStatus, TaskSummary, ToolDispatchRow},
        pursue_runs::{PursueOutcome, PursueRun, PursueStepKind, pursue_run},
        ultrafast_ledger::UltrafastRecord,
        ultrafast_runs::{UltrafastRunStats, ultrafast_run_stats},
    },
    error::{AppResult, CanonicalError, RequestId},
    ids::SessionId,
    services::{
        cockpit::{
            LiveActivityState, LiveSessionFilters, LiveSessionProjection, LiveStateProjection,
            LiveTabProjection,
        },
        sessions::Session,
        ultrafast_meter::{self, NeoWindow, NeoWindows},
    },
    storage::JsonStore,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use claw_api::models::{
    CancelSessionResponse, Dispatch, PlanSavings, PlanSavingsMeter, PlanSavingsPeriod,
    PlanSavingsPeriods, PlanSavingsSite, ResolveHelpRequest, ResolveHelpResponse,
    SessionBrowserTab, SessionDetail, SessionList, SessionPursueRun, SessionPursueStep,
    SessionStatus, SessionSummary, SessionTokenUsage, SessionUltrafastRun, SkillDraftStatus,
};
use std::{collections::HashMap, sync::Arc};

#[derive(Default)]
struct SessionQuery {
    profile_id: Option<String>,
    slug: Option<String>,
    status: Option<TaskStatus>,
    needs_attention: bool,
    site: Option<String>,
    search: Option<String>,
    since: Option<i64>,
    cursor: Option<i64>,
    limit: Option<i64>,
}

pub(super) async fn list(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<SessionList>, CanonicalError> {
    let query = parse_query(&request_id, &raw)?;
    if query.status == Some(TaskStatus::Live) {
        let mut items = state
            .cockpit
            .list(&LiveSessionFilters {
                profile_id: query.profile_id,
                slug: query.slug,
                site: query.site,
                search: query.search,
                since: query.since,
            })
            .await
            .map_err(|source| internal(&request_id, source))?
            .into_iter()
            .map(contract_live_projection)
            .collect::<Vec<_>>();
        for item in &mut items {
            let session_id = SessionId::new(item.session_id.clone());
            let target = item
                .live
                .as_deref()
                .and_then(|live| {
                    live.browser_tabs
                        .iter()
                        .max_by_key(|tab| tab.last_activity_at.unwrap_or(0))
                })
                .map(|tab| (tab.browser_tab_id, tab.url.clone(), tab.title.clone()));
            let (tab_id, url, title) = match target {
                Some((tab_id, url, title)) => (Some(tab_id), Some(url), Some(title)),
                None => (None, None, None),
            };
            if let Some(live) = item.live.as_deref_mut() {
                live.help_request = state
                    .help
                    .snapshot(&session_id, tab_id, url, title)
                    .await
                    .map(Box::new);
            }
        }
        return Ok(Json(SessionList::new(items)));
    }
    let response_status_filter = query.status;
    let stored_status_filter = query.status.filter(|status| *status != TaskStatus::Done);
    let result = state
        .audit_log
        .list_tasks(ListTasksQuery {
            slug: query.slug,
            status: stored_status_filter,
            needs_attention: query.needs_attention,
            site: query.site,
            search: query.search,
            since: query.since,
            cursor: query.cursor,
            limit: query.limit,
            ..ListTasksQuery::default()
        })
        .await
        .map_err(|source| internal(&request_id, source))?;
    let live = live_sessions(&state).await;
    let session_ids = result
        .tasks
        .iter()
        .map(|task| task.session_id.clone())
        .collect::<Vec<_>>();
    let ultrafast = state
        .audit_log
        .ultrafast_run_stats(&session_ids)
        .await
        .map_err(|source| internal(&request_id, source))?;
    let mut pursue = state
        .audit_log
        .pursue_runs(&session_ids)
        .await
        .map_err(|source| internal(&request_id, source))?;
    let mut skill_names = state
        .skills
        .names_by_session(&session_ids)
        .await
        .map_err(|source| internal(&request_id, source))?;
    // profile_id lives on the live session's agent, and a done filter
    // must reconcile persisted-live rows. Both apply after pagination,
    // so a filtered page may come back short rather than backfilled.
    let mut items = Vec::with_capacity(result.tasks.len());
    for task in result.tasks {
        let session = live.get(task.session_id.as_str());
        let stats = ultrafast.get(task.session_id.as_str()).copied();
        let pursued = pursue.remove(task.session_id.as_str());
        let mut summary = contract_summary(task, session).await;
        summary.ultrafast = stats.map(contract_ultrafast);
        summary.pursue = pursued.map(contract_pursue);
        summary.skill_name = skill_names.remove(summary.session_id.as_str());
        if response_status_filter
            .is_none_or(|status| summary.status == contract_status(status, true))
            && query
                .profile_id
                .as_ref()
                .is_none_or(|profile_id| summary.profile_id.as_ref() == Some(profile_id))
        {
            items.push(summary);
        }
    }
    let mut response = SessionList::new(items);
    response.next_cursor = result.next_cursor;
    Ok(Json(response))
}

pub(super) async fn get(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionDetail>, CanonicalError> {
    let task = state
        .audit_log
        .get_task(&session_id)
        .await
        .map_err(|source| internal(&request_id, source))?
        .ok_or_else(|| {
            error(
                &request_id,
                StatusCode::NOT_FOUND,
                "session_not_found",
                "session not found",
            )
        })?;
    let skill_name = saved_skill(&state, &session_id)
        .await
        .map_err(|source| internal(&request_id, source))?;
    let live = state.sessions.lookup(&SessionId::new(session_id)).await;
    let mut detail = contract_detail(task, live.as_ref()).await;
    detail.session.skill_name = skill_name;
    Ok(Json(detail))
}

/// The skill saved from this task, by the app or the agent, if there is one.
async fn saved_skill(state: &AppState, session_id: &str) -> AppResult<Option<String>> {
    Ok(state
        .skills
        .names_by_session(&[session_id.to_string()])
        .await?
        .remove(session_id))
}

/// GET /api/v1/sessions/{id}/skill-draft: where making a skill from this task stands.
pub(super) async fn skill_draft(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SkillDraftStatus>, CanonicalError> {
    state
        .audit_log
        .get_task(&session_id)
        .await
        .map_err(|source| internal(&request_id, source))?
        .ok_or_else(|| session_not_found(&request_id))?;
    let skill_name = saved_skill(&state, &session_id)
        .await
        .map_err(|source| internal(&request_id, source))?;
    let status = state
        .skill_drafts
        .status(&session_id, skill_name, now_epoch_ms())
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(status))
}

/// POST /api/v1/sessions/{id}/skill-draft: writes a skill draft from this finished task through
/// the decisions worker, or returns the draft already written. Ultrafast plans with skill drafts
/// only. The draft runs on its own task, so leaving the page does not waste a paid draft.
pub(super) async fn draft_skill(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SkillDraftStatus>, CanonicalError> {
    use crate::services::{billing::ULTRAFAST_FEATURE, skill_draft::digest};
    if !state.ultrafast_enabled() || !state.billing.has_feature(ULTRAFAST_FEATURE).await {
        return Err(error(
            &request_id,
            StatusCode::FORBIDDEN,
            "not_entitled",
            "this plan does not include making skills from tasks",
        ));
    }
    let task = state
        .audit_log
        .get_task(&session_id)
        .await
        .map_err(|source| internal(&request_id, source))?
        .ok_or_else(|| session_not_found(&request_id))?;
    let live = state
        .sessions
        .lookup(&SessionId::new(session_id.clone()))
        .await;
    if !digest::ended_cleanly(&task.summary) || live.is_some() {
        return Err(error(
            &request_id,
            StatusCode::UNPROCESSABLE_ENTITY,
            "not_finished",
            "only a finished task, or one stopped without errors, can become a skill",
        ));
    }
    let skill_name = saved_skill(&state, &session_id)
        .await
        .map_err(|source| internal(&request_id, source))?;
    if skill_name.is_some() {
        let status = state
            .skill_drafts
            .status(&session_id, skill_name, now_epoch_ms())
            .await
            .map_err(|source| internal(&request_id, source))?;
        return Ok(Json(status));
    }
    let request = digest::build(&task);
    let drafter = state.decisions.skill_drafter(state.billing.clone());
    let drafts = state.skill_drafts.clone();
    let id = session_id.clone();
    let written = tokio::spawn(async move {
        drafts
            .draft(&id, now_epoch_ms(), request, |request| async move {
                match drafter {
                    Some(drafter) => drafter.draft(&request).await,
                    None => Err(claw_api::models::SkillDraftFailure::new(
                        claw_api::models::SkillDraftFailureCode::UpstreamError,
                    )),
                }
            })
            .await
    })
    .await
    .map_err(|join| {
        internal(
            &request_id,
            crate::error::AppError::unavailable(join.to_string()),
        )
    })?
    .map_err(|source| internal(&request_id, source))?;
    Ok(Json(written))
}

fn session_not_found(request_id: &RequestId) -> CanonicalError {
    error(
        request_id,
        StatusCode::NOT_FOUND,
        "session_not_found",
        "session not found",
    )
}

/// How many of the newest ultrafast tasks the savings view lists.
const SAVINGS_LATEST: usize = 3;

/// GET /api/v1/plan/savings: what ultrafast has done on this computer, from the task history.
pub(super) async fn savings(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<PlanSavings>, CanonicalError> {
    let internal = |source| internal(&request_id, source);
    // Save what the history still shows first, so nothing it holds is lost once it ages out.
    let sessions = state
        .audit_log
        .all_ultrafast_sessions()
        .await
        .map_err(internal)?;
    let records = sessions
        .iter()
        .map(UltrafastRecord::from)
        .collect::<Vec<_>>();
    state
        .ultrafast_ledger
        .record(&records)
        .await
        .map_err(internal)?;
    let ledger = state.ultrafast_ledger.all().await.map_err(internal)?;
    let totals = ledger
        .iter()
        .fold(UltrafastRunStats::default(), |sum, record| {
            UltrafastRunStats {
                duration_ms: sum.duration_ms.saturating_add(record.totals.duration_ms),
                pages_read: sum.pages_read.saturating_add(record.totals.pages_read),
                tokens_saved: sum.tokens_saved.saturating_add(record.totals.tokens_saved),
            }
        });
    let in_history = sessions
        .iter()
        .map(|session| (session.session_id.as_str(), session))
        .collect::<HashMap<_, _>>();
    let live = live_sessions(&state).await;
    let mut latest = Vec::with_capacity(SAVINGS_LATEST);
    for record in &ledger {
        if latest.len() == SAVINGS_LATEST {
            break;
        }
        // A task whose history has aged out still counts, but has nothing left to list.
        let Some(session) = in_history.get(record.session_id.as_str()) else {
            continue;
        };
        let Some(task) = state
            .audit_log
            .get_task_summary(&session.session_id)
            .await
            .map_err(internal)?
        else {
            continue;
        };
        let mut summary = contract_summary(task, live.get(session.session_id.as_str())).await;
        summary.ultrafast = session.measured.map(contract_ultrafast);
        summary.pursue = session.pursue.clone().map(contract_pursue);
        latest.push(summary);
    }
    let tasks = i64::try_from(ledger.len()).unwrap_or(i64::MAX);
    let mut savings = PlanSavings::new(*contract_ultrafast(totals), tasks, latest);
    savings.meter = meter(&state, &ledger)
        .await
        .map_err(internal)?
        .map(Box::new);
    Ok(Json(savings))
}

/// The Home meter, counted from the day this computer got ultrafast; `None` before that day.
async fn meter(
    state: &AppState,
    records: &[UltrafastRecord],
) -> AppResult<Option<PlanSavingsMeter>> {
    let now = now_epoch_ms();
    let first_run = records.iter().map(|record| record.first_run_at).min();
    let store = JsonStore::new(state.config.browserclaw_dir.clone());
    let Some(since) =
        ultrafast_meter::since(&store, state.ultrafast_enabled(), first_run, now).await?
    else {
        return Ok(None);
    };
    let [since_start, month, week] = ultrafast_meter::period_starts(since, now);
    let neo = NeoWindows {
        since: neo_window(state, since_start).await?,
        last_30_days: neo_window(state, month).await?,
        last_7_days: neo_window(state, week).await?,
    };
    let meter = ultrafast_meter::meter(since, &neo, records, now);
    let periods = PlanSavingsPeriods::new(
        contract_period(&meter.periods.since),
        contract_period(&meter.periods.last_30_days),
        contract_period(&meter.periods.last_7_days),
    );
    let mut contract =
        PlanSavingsMeter::new(meter.since, meter.tokens_saved, meter.run_times, periods);
    contract.neo_step_share = meter.neo_step_share;
    contract.fastest_run_ms = meter.fastest_run_ms;
    Ok(Some(contract))
}

/// neo's own saving, time saved and tasks finished from sessions that ended at or after `start`.
async fn neo_window(state: &AppState, start: i64) -> AppResult<NeoWindow> {
    let window = state.session_efficiency.window_since(start).await?;
    Ok(NeoWindow {
        tokens_saved: window.raw_token_savings_estimate,
        time_saved_ms: window.human_time_saved_ms,
        tasks_finished: window.session_count,
    })
}

fn contract_period(period: &ultrafast_meter::Period) -> PlanSavingsPeriod {
    let mut contract = PlanSavingsPeriod::new(
        period.tokens_saved,
        period.runs,
        period.pages_read,
        period.duration_ms,
        period.site_tasks,
        period.site_tasks_finished,
        period
            .top_sites
            .iter()
            .map(|(site, runs)| PlanSavingsSite::new(site.clone(), *runs))
            .collect(),
        period.time_saved_ms,
        period.tasks_finished,
        period.run_times.clone(),
    );
    contract.fastest_run_ms = period.fastest_run_ms;
    contract.neo_step_share = period.neo_step_share;
    contract
}

pub(super) async fn cancel(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<CancelSessionResponse>, CanonicalError> {
    let session_id = SessionId::new(session_id);
    let cancelled_dispatches = match state
        .sessions
        .cancel_by_session(&session_id)
        .await
        .map_err(|source| internal(&request_id, source))?
    {
        Some(cancelled_dispatches) => cancelled_dispatches,
        None => match state
            .audit_log
            .get_task_summary(session_id.as_str())
            .await
            .map_err(|source| internal(&request_id, source))?
            .map(|summary| summary.status)
        {
            Some(TaskStatus::Cancelled) => 0,
            Some(_) => {
                return Err(error(
                    &request_id,
                    StatusCode::CONFLICT,
                    "session_not_live",
                    "session is not live",
                ));
            }
            None => {
                return Err(error(
                    &request_id,
                    StatusCode::NOT_FOUND,
                    "session_not_found",
                    "session not found",
                ));
            }
        },
    };
    Ok(Json(CancelSessionResponse::new(
        SessionStatus::Cancelled,
        i64::try_from(cancelled_dispatches).unwrap_or(i64::MAX),
    )))
}

pub(super) async fn resolve_help(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    body: Option<Json<ResolveHelpRequest>>,
) -> Json<ResolveHelpResponse> {
    let session_id = SessionId::new(session_id);
    let note = body
        .and_then(|Json(req)| req.note)
        .map(|note| note.trim().to_string())
        .filter(|note| !note.is_empty());
    let resolved = state.help.resolve(&session_id, note).await;
    Json(ResolveHelpResponse::new(resolved))
}

async fn live_sessions(state: &AppState) -> HashMap<String, Arc<Session>> {
    state
        .sessions
        .snapshot()
        .await
        .into_iter()
        .map(|session| (session.id().as_str().to_string(), session))
        .collect()
}

async fn contract_detail(task: TaskDetail, live: Option<&Arc<Session>>) -> SessionDetail {
    let screenshots = task
        .screenshot_dispatch_ids
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let profile_id = live
        .and_then(|session| session.agent().profile_id())
        .map(|profile_id| profile_id.as_str().to_string());
    let ultrafast = ultrafast_run_stats(&task.dispatches).map(contract_ultrafast);
    let pursue = pursue_run(&task.dispatches).map(contract_pursue);
    let dispatches = task
        .dispatches
        .into_iter()
        .map(|row| contract_dispatch(row, &screenshots, profile_id.as_ref()))
        .collect();
    let mut summary = contract_summary(task.summary, live).await;
    summary.ultrafast = ultrafast;
    summary.pursue = pursue;
    SessionDetail::new(summary, dispatches)
}

async fn contract_summary(task: TaskSummary, live: Option<&Arc<Session>>) -> SessionSummary {
    let name = match live {
        Some(session) => session.display_name(&task.title).await,
        None => task.title.clone(),
    };
    let token_usage = contract_token_usage(&task);
    let mut summary = SessionSummary::new(
        task.session_id,
        task.slug,
        task.agent_label,
        name,
        task.started_at,
        task.duration_ms.max(0),
        task.dispatch_count,
        task.tool_sequence,
        contract_status(task.status, live.is_some()),
        task.error_count,
    );
    summary.profile_id = live
        .and_then(|session| session.agent().profile_id())
        .map(|profile_id| profile_id.as_str().to_string());
    summary.site = task.site;
    summary.task_summary = task.task_summary;
    summary.ended_at = task.ended_at;
    summary.latest_screenshot_id = task.last_screenshot_dispatch_id;
    summary.token_usage = token_usage;
    summary
}

fn contract_live_projection(projection: LiveSessionProjection) -> SessionSummary {
    let LiveSessionProjection {
        task,
        profile_id,
        harness,
        color,
        label,
        name,
        live,
    } = projection;
    let token_usage = contract_token_usage(&task);
    let mut summary = SessionSummary::new(
        task.session_id,
        task.slug,
        label,
        name,
        task.started_at,
        task.duration_ms.max(0),
        task.dispatch_count,
        task.tool_sequence,
        SessionStatus::Live,
        task.error_count,
    );
    summary.profile_id = profile_id;
    summary.harness = harness;
    summary.color = Some(color);
    summary.site = task.site;
    summary.task_summary = task.task_summary;
    summary.ended_at = task.ended_at;
    summary.latest_screenshot_id = task.last_screenshot_dispatch_id;
    summary.token_usage = token_usage;
    summary.live = Some(Box::new(contract_live_state(live)));
    summary
}

/// Builds the session's token-consumption DTO from the materialized totals, clamped into the
/// JavaScript-safe range the contract promises. Returns `None` for legacy/unmeasured sessions so
/// the wire omits the field entirely instead of reporting a misleading zero.
fn contract_token_usage(task: &TaskSummary) -> Option<Box<SessionTokenUsage>> {
    if !task.tokens_measured {
        return None;
    }
    let input = js_safe_token_estimate(task.tool_input_token_estimate);
    let output = js_safe_token_estimate(task.tool_output_token_estimate);
    let total = js_safe_token_estimate(input.saturating_add(output));
    Some(Box::new(SessionTokenUsage::new(input, output, total)))
}

fn contract_ultrafast(stats: UltrafastRunStats) -> Box<SessionUltrafastRun> {
    Box::new(SessionUltrafastRun::new(
        stats.duration_ms.max(0),
        stats.pages_read.max(0),
        js_safe_token_estimate(stats.tokens_saved),
    ))
}

fn contract_pursue(run: PursueRun) -> Box<SessionPursueRun> {
    use claw_api::models::{SessionPursueRunOutcome as Outcome, SessionPursueStepAction as Action};
    let outcome = match run.outcome {
        PursueOutcome::Completed => Outcome::Completed,
        PursueOutcome::HandedBack => Outcome::HandedBack,
    };
    let steps = run
        .steps
        .into_iter()
        .map(|step| SessionPursueStep {
            action: match step.kind {
                PursueStepKind::Click => Action::Click,
                PursueStepKind::Type => Action::Type,
                PursueStepKind::Press => Action::Press,
                PursueStepKind::Check => Action::Check,
                PursueStepKind::Uncheck => Action::Uncheck,
                PursueStepKind::Select => Action::Select,
                PursueStepKind::Scroll => Action::Scroll,
                PursueStepKind::Drag => Action::Drag,
                PursueStepKind::Dialog => Action::Dialog,
                PursueStepKind::Open => Action::Open,
                PursueStepKind::Back => Action::Back,
                PursueStepKind::Reload => Action::Reload,
            },
            text: step.text,
        })
        .collect();
    Box::new(SessionPursueRun::new(
        outcome,
        run.duration_ms.max(0),
        steps,
    ))
}

/// Clamps a token total into `[0, 2^53-1]` so it survives a round trip through a JavaScript client.
fn js_safe_token_estimate(value: i64) -> i64 {
    const JS_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
    value.clamp(0, JS_SAFE_INTEGER)
}

fn contract_live_state(projection: LiveStateProjection) -> claw_api::models::LiveSessionState {
    claw_api::models::LiveSessionState::new(
        match projection.state {
            LiveActivityState::Active => claw_api::models::LiveSessionActivityState::Active,
            LiveActivityState::Idle => claw_api::models::LiveSessionActivityState::Idle,
        },
        projection
            .browser_tabs
            .into_iter()
            .map(contract_live_tab)
            .collect(),
    )
}

fn contract_live_tab(projection: LiveTabProjection) -> SessionBrowserTab {
    let mut tab = SessionBrowserTab::new(
        projection.browser_tab_id,
        projection.url,
        projection.title,
        projection.tool_count,
        projection
            .recent_tools
            .into_iter()
            .map(|event| claw_api::models::ToolEvent::new(event.name, event.at))
            .collect(),
    );
    tab.first_activity_at = projection.first_activity_at;
    tab.last_activity_at = projection.last_activity_at;
    tab.last_tool_name = projection.last_tool_name;
    tab
}

fn contract_status(status: TaskStatus, is_live: bool) -> SessionStatus {
    match status {
        TaskStatus::Live if !is_live => SessionStatus::Done,
        TaskStatus::Live => SessionStatus::Live,
        TaskStatus::Done => SessionStatus::Done,
        TaskStatus::Failed => SessionStatus::Failed,
        TaskStatus::Cancelled => SessionStatus::Cancelled,
    }
}

fn contract_dispatch(
    row: ToolDispatchRow,
    screenshots: &std::collections::HashSet<i64>,
    profile_id: Option<&String>,
) -> Dispatch {
    let mut dispatch = Dispatch::new(
        row.id,
        row.created_at,
        row.slug,
        row.agent_label,
        row.session_id,
        row.tool_name,
    );
    dispatch.screenshot_id = screenshots.contains(&row.id).then_some(row.id);
    dispatch.profile_id = profile_id.cloned();
    dispatch.page_id = row.page_id;
    dispatch.tab_id = row.tab_id;
    dispatch.target_id = row.target_id;
    dispatch.url = row.url;
    dispatch.title = row.title;
    dispatch.args_json = row.args_json;
    dispatch.result_meta = row.result_meta;
    dispatch.duration_ms = row.duration_ms;
    dispatch.dispatch_key = row.dispatch_id;
    dispatch.parent_dispatch_id = row.parent_dispatch_id;
    dispatch
}

fn parse_query(
    request_id: &RequestId,
    raw: &HashMap<String, String>,
) -> Result<SessionQuery, CanonicalError> {
    let status = match raw.get("status").map(String::as_str) {
        None => None,
        Some("live") => Some(TaskStatus::Live),
        Some("done") => Some(TaskStatus::Done),
        Some("failed") => Some(TaskStatus::Failed),
        Some("cancelled") => Some(TaskStatus::Cancelled),
        Some(_) => return Err(invalid_query(request_id, "invalid status")),
    };
    let needs_attention = match raw.get("needsAttention").map(String::as_str) {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => return Err(invalid_query(request_id, "invalid needsAttention")),
    };
    Ok(SessionQuery {
        profile_id: raw.get("profileId").cloned(),
        slug: raw.get("slug").cloned(),
        status,
        needs_attention,
        site: raw.get("site").cloned(),
        search: raw.get("search").cloned(),
        since: parse_integer(request_id, raw, "since", 0, i64::MAX)?,
        cursor: parse_integer(request_id, raw, "cursor", 1, i64::MAX)?,
        limit: parse_integer(request_id, raw, "limit", 1, 100)?,
    })
}

fn parse_integer(
    request_id: &RequestId,
    raw: &HashMap<String, String>,
    key: &str,
    minimum: i64,
    maximum: i64,
) -> Result<Option<i64>, CanonicalError> {
    let Some(value) = raw.get(key) else {
        return Ok(None);
    };
    let value = value
        .parse::<i64>()
        .map_err(|_| invalid_query(request_id, "invalid integer query parameter"))?;
    if value < minimum || value > maximum {
        return Err(invalid_query(request_id, "query parameter out of range"));
    }
    Ok(Some(value))
}

fn invalid_query(request_id: &RequestId, message: &str) -> CanonicalError {
    error(
        request_id,
        StatusCode::BAD_REQUEST,
        "invalid_request",
        message,
    )
}
