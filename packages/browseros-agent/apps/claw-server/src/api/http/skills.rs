use super::{error, internal};
use crate::{
    AppState,
    db::entities::{skill_flows, skill_runs},
    error::{AppError, AppResult, CanonicalError, RequestId},
    services::{
        flow::{
            FlowStatus,
            consent::{self, Decision},
            prose, store,
        },
        skills::{CreateSkill, SkillDetailView, SkillOrigin, SkillView, UpdateSkill},
    },
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use claw_api::models::{self, Skill, SkillCreate, SkillDetail, SkillList, SkillRun, SkillRunList};
use harness_integrations::AgentId;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub(super) struct RunsQuery {
    cursor: Option<i64>,
    limit: Option<u64>,
}

pub(super) async fn list(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<SkillList>, CanonicalError> {
    let views = state
        .skills
        .list()
        .await
        .map_err(|source| map_error(&request_id, source))?;
    let all_runs = state
        .skills
        .all_runs()
        .await
        .map_err(|source| map_error(&request_id, source))?;
    let mut runs_by_skill: HashMap<String, Vec<skill_runs::Model>> = HashMap::new();
    for run in all_runs {
        runs_by_skill
            .entry(run.skill_name.clone())
            .or_default()
            .push(run);
    }
    let mut flows: HashMap<String, skill_flows::Model> = if shows_ultrafast(&state) {
        state
            .skills
            .flow_rows()
            .await
            .map_err(|source| map_error(&request_id, source))?
            .into_iter()
            .map(|row| (row.skill_name.clone(), row))
            .collect()
    } else {
        HashMap::new()
    };
    let mut items = Vec::with_capacity(views.len());
    for view in views {
        let flow = flows.remove(&view.model.name);
        let runs = runs_by_skill
            .get(&view.model.name)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let token_savings = skill_token_savings(&state, runs)
            .await
            .map_err(|source| map_error(&request_id, source))?;
        let mut dto = skill_to_dto(view);
        dto.token_savings = Some(Box::new(token_savings));
        if shows_ultrafast(&state) {
            dto.ultrafast = Some(Box::new(ultrafast_to_dto(&state, flow.as_ref(), None)));
        }
        items.push(dto);
    }
    Ok(Json(SkillList::new(items)))
}

pub(super) async fn get(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<SkillDetail>, CanonicalError> {
    let detail = state
        .skills
        .get(&name)
        .await
        .map_err(|source| map_error(&request_id, source))?;
    let token_savings = skill_token_savings(&state, &detail.runs)
        .await
        .map_err(|source| map_error(&request_id, source))?;
    Ok(Json(
        with_ultrafast(&state, &request_id, detail_to_dto(detail, token_savings)).await?,
    ))
}

pub(super) async fn create(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Json(body): Json<SkillCreate>,
) -> Result<(StatusCode, Json<Skill>), CanonicalError> {
    let view = state
        .skills
        .create(CreateSkill {
            name: body.name,
            description: body.description,
            site: body.site,
            steps: body.steps.unwrap_or_default(),
            learned_notes: body.learned_notes.unwrap_or_default(),
            origin: SkillOrigin::Manual,
            source_session_id: None,
        })
        .await
        .map_err(|source| map_error(&request_id, source))?;
    let mut skill = skill_to_dto(view);
    if shows_ultrafast(&state) {
        skill.ultrafast = Some(Box::new(ultrafast_to_dto(&state, None, None)));
    }
    Ok((StatusCode::CREATED, Json(skill)))
}

pub(super) async fn update(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<models::SkillUpdate>,
) -> Result<Json<SkillDetail>, CanonicalError> {
    let detail = state
        .skills
        .update(
            &name,
            UpdateSkill {
                description: body.description,
                site: body.site,
                steps: body.steps,
                learned_notes: body.learned_notes,
                body: body.body,
            },
        )
        .await
        .map_err(|source| map_error(&request_id, source))?;
    let detail = if state.ultrafast_enabled() {
        state
            .skills
            .reconcile_flow_after_edit(&name)
            .await
            .map_err(|source| map_error(&request_id, source))?;
        state
            .skills
            .get(&name)
            .await
            .map_err(|source| map_error(&request_id, source))?
    } else {
        detail
    };
    let token_savings = skill_token_savings(&state, &detail.runs)
        .await
        .map_err(|source| map_error(&request_id, source))?;
    Ok(Json(
        with_ultrafast(&state, &request_id, detail_to_dto(detail, token_savings)).await?,
    ))
}

pub(super) async fn delete(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, CanonicalError> {
    state
        .skills
        .delete(&name)
        .await
        .map_err(|source| map_error(&request_id, source))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn list_runs(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<RunsQuery>,
) -> Result<Json<SkillRunList>, CanonicalError> {
    let (runs, next_cursor) = state
        .skills
        .list_runs(&name, query.cursor, query.limit)
        .await
        .map_err(|source| map_error(&request_id, source))?;
    let mut list = SkillRunList::new(runs.into_iter().map(run_to_dto).collect());
    list.next_cursor = next_cursor;
    Ok(Json(list))
}

pub(super) async fn flow(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<models::SkillFlowSource>, CanonicalError> {
    if !state.ultrafast_enabled() {
        return Err(map_error(
            &request_id,
            AppError::not_found("this skill has no ultrafast flow"),
        ));
    }
    let path = crate::services::flow::store::flow_path(&state.skills.skill_dir(&name));
    match tokio::fs::read_to_string(&path).await {
        Ok(source) => Ok(Json(models::SkillFlowSource::new(source))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(map_error(
            &request_id,
            AppError::not_found("this skill has no ultrafast flow"),
        )),
        Err(error) => Err(internal(
            &request_id,
            AppError::Io {
                path: Some(path),
                source: error,
            },
        )),
    }
}

/// Runs the skill without the agent: neo does it by itself, including its one click that changes
/// something. Chosen in the app, for the skill's current click.
pub(super) async fn approve_write(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Skill>, CanonicalError> {
    choose(&state, &request_id, &name, true).await.map(Json)
}

/// Runs the skill with the agent again, so it reads each page as it goes.
pub(super) async fn revoke_write(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Skill>, CanonicalError> {
    choose(&state, &request_id, &name, false).await.map(Json)
}

async fn choose(
    state: &AppState,
    request_id: &RequestId,
    name: &str,
    without_agent: bool,
) -> Result<Skill, CanonicalError> {
    if !state.ultrafast_enabled() {
        return Err(map_error(
            request_id,
            AppError::not_found("this skill has no ultrafast flow"),
        ));
    }
    let detail = state
        .skills
        .get(name)
        .await
        .map_err(|source| map_error(request_id, source))?;
    let row = state
        .skills
        .flow_row(name)
        .await
        .map_err(|source| map_error(request_id, source))?;
    let doc = store::read(&state.skills.skill_dir(name)).ok().flatten();
    let labels = prose::prose_steps(&detail.body);
    let (Some(mut row), Some(doc)) = (row, doc) else {
        return Err(map_error(
            request_id,
            AppError::conflict("this skill has no write flow to choose for"),
        ));
    };
    let Some(fingerprint) = consent::commit_fingerprint(&doc, &labels) else {
        return Err(map_error(
            request_id,
            AppError::conflict("this skill has no write flow to choose for"),
        ));
    };
    if without_agent && let Some(kind) = consent::always_with_agent(&doc, &labels) {
        return Err(map_error(
            request_id,
            AppError::conflict(format!(
                "{} Skills like this always run with your agent.",
                kind.reason()
            )),
        ));
    }
    let now = crate::clock::now_epoch_ms();
    row.write_approved = without_agent;
    row.approved_commit = without_agent.then(|| fingerprint.clone());
    row.with_agent_commit = (!without_agent).then_some(fingerprint);
    row.approved_via = Some("app".to_string());
    row.approved_at = Some(now);
    row.approved_answer = None;
    row.approved_client = None;
    row.updated_at = now;
    state
        .skills
        .put_flow_row(row.clone())
        .await
        .map_err(|source| map_error(request_id, source))?;
    // SKILL.md follows: "Run it fast" while neo runs it alone, plain steps while the agent does.
    state
        .skills
        .reconcile_flow_after_edit(name)
        .await
        .map_err(|source| map_error(request_id, source))?;
    let runs = runs_view(Some(&row), &doc, &labels, detail.view.model.site.as_deref());
    let mut skill = skill_to_dto(detail.view);
    skill.ultrafast = Some(Box::new(ultrafast_to_dto(state, Some(&row), runs)));
    Ok(skill)
}

fn map_error(request_id: &RequestId, source: AppError) -> CanonicalError {
    match source.status() {
        StatusCode::NOT_FOUND => error(
            request_id,
            StatusCode::NOT_FOUND,
            "skill_not_found",
            &source.to_string(),
        ),
        StatusCode::CONFLICT => error(
            request_id,
            StatusCode::CONFLICT,
            "skill_conflict",
            &source.to_string(),
        ),
        StatusCode::BAD_REQUEST => error(
            request_id,
            StatusCode::BAD_REQUEST,
            "invalid_skill",
            &source.to_string(),
        ),
        _ => internal(request_id, source),
    }
}

fn skill_to_dto(view: SkillView) -> Skill {
    let SkillView {
        model,
        linked_agents,
        stats,
    } = view;
    let mut skill = Skill::new(
        model.name,
        model.description,
        origin_to_dto(&model.origin),
        model.version,
        linked_agents.into_iter().map(agent_to_harness).collect(),
        stats.run_count,
        stats.clean_run_count,
        model.created_at,
        model.updated_at,
    );
    skill.site = model.site;
    skill.source_session_id = model.source_session_id;
    skill.first_run_tokens = stats.first_run_tokens;
    skill.latest_run_tokens = stats.latest_run_tokens;
    skill.last_run_at = stats.last_run_at;
    skill
}

/// Whether skill views carry their ultrafast state: while it is on, and on a computer that can
/// buy it, so a skill whose plan ended still shows what it would do.
fn shows_ultrafast(state: &AppState) -> bool {
    state.ultrafast_enabled() || (state.billing.is_available() && !state.config.mode.is_hosted())
}

/// Fills the detail's ultrafast state.
async fn with_ultrafast(
    state: &AppState,
    request_id: &RequestId,
    mut detail: SkillDetail,
) -> Result<SkillDetail, CanonicalError> {
    if shows_ultrafast(state) {
        let flow = state
            .skills
            .flow_row(&detail.skill.name)
            .await
            .map_err(|source| map_error(request_id, source))?;
        let runs = store::read(&state.skills.skill_dir(&detail.skill.name))
            .ok()
            .flatten()
            .and_then(|doc| {
                let labels = prose::prose_steps(&detail.body);
                runs_view(flow.as_ref(), &doc, &labels, detail.skill.site.as_deref())
            });
        detail.skill.ultrafast = Some(Box::new(ultrafast_to_dto(state, flow.as_ref(), runs)));
    }
    Ok(detail)
}

/// How a write skill runs and who chose that; `None` for a skill that only reads pages.
fn runs_view(
    row: Option<&skill_flows::Model>,
    doc: &store::FlowDoc,
    labels: &[String],
    skill_site: Option<&str>,
) -> Option<models::SkillUltrafastRuns> {
    let fingerprint = consent::commit_fingerprint(doc, labels)?;
    let decision = consent::decision(row, doc, labels);
    let mode = match decision {
        Decision::WithoutAgent => models::SkillUltrafastRunsMode::WithoutAgent,
        _ => models::SkillUltrafastRunsMode::WithAgent,
    };
    let chosen = match decision {
        Decision::WithoutAgent => {
            row.and_then(|row| row.approved_commit.as_deref()) == Some(fingerprint.as_str())
        }
        Decision::WithAgent => true,
        Decision::Undecided | Decision::AlwaysWith(_) => false,
    };
    let mut runs =
        models::SkillUltrafastRuns::new(mode, chosen, consent::is_reask(row, doc, labels));
    if let Decision::AlwaysWith(kind) = decision {
        runs.always_with_agent = Some(match kind {
            consent::AlwaysWith::Pay => models::SkillUltrafastAlwaysWith::Pay,
            consent::AlwaysWith::Order => models::SkillUltrafastAlwaysWith::Order,
            consent::AlwaysWith::Send => models::SkillUltrafastAlwaysWith::Send,
            consent::AlwaysWith::Delete => models::SkillUltrafastAlwaysWith::Delete,
        });
    }
    runs.click = consent::commit_label(doc, labels);
    runs.site = Some(consent::site_label(doc, skill_site));
    if chosen && let Some(row) = row {
        runs.via = match row.approved_via.as_deref() {
            Some("chat") => Some(models::SkillUltrafastChoiceVia::Chat),
            Some("app") => Some(models::SkillUltrafastChoiceVia::App),
            _ => None,
        };
        runs.chosen_at = row.approved_at;
        runs.answer = row.approved_answer.clone();
        runs.client = row.approved_client.clone();
    }
    Some(runs)
}

pub(super) fn ultrafast_to_dto(
    state: &AppState,
    flow: Option<&skill_flows::Model>,
    runs: Option<models::SkillUltrafastRuns>,
) -> models::SkillUltrafast {
    let locked = !state.ultrafast_enabled();
    // While locked every skill runs with the agent; the stored choice waits for the plan.
    let runs = runs.map(|mut runs| {
        if locked {
            runs.mode = models::SkillUltrafastRunsMode::WithAgent;
        }
        runs
    });
    let Some(flow) = flow else {
        let mut dto = models::SkillUltrafast::new(models::SkillUltrafastStatus::None, 0, false);
        dto.locked = Some(locked);
        return dto;
    };
    let status = match FlowStatus::parse(&flow.status) {
        FlowStatus::None => models::SkillUltrafastStatus::None,
        FlowStatus::Pending => models::SkillUltrafastStatus::Pending,
        FlowStatus::Verified => models::SkillUltrafastStatus::Verified,
        FlowStatus::Degraded => models::SkillUltrafastStatus::Degraded,
        FlowStatus::Broken => models::SkillUltrafastStatus::Broken,
        FlowStatus::Off => models::SkillUltrafastStatus::Off,
    };
    let without_agent = runs
        .as_ref()
        .is_some_and(|runs| runs.mode == models::SkillUltrafastRunsMode::WithoutAgent);
    let mut dto = models::SkillUltrafast::new(status, flow.flow_version, without_agent);
    dto.reason = flow.status_reason.clone();
    dto.effects = match flow.effects.as_deref() {
        Some("read") => Some(models::SkillUltrafastEffects::Read),
        Some("write") => Some(models::SkillUltrafastEffects::Write),
        _ => None,
    };
    dto.p50_ms = flow.p50_ms;
    dto.verified_at = flow.verified_at;
    dto.repaired_at = flow.repaired_at;
    dto.locked = Some(locked);
    dto.runs = runs.map(Box::new);
    dto
}

fn detail_to_dto(detail: SkillDetailView, token_savings: models::SkillTokenSavings) -> SkillDetail {
    let SkillDetailView { view, body, runs } = detail;
    SkillDetail::new(
        skill_to_dto(view),
        body,
        runs.into_iter().map(run_to_dto).collect(),
        token_savings,
    )
}

/// Aggregates the token efficiency of the skill's run sessions into the detail
/// DTO: how many tokens BrowserOS neo saved versus a screenshot-first agent,
/// what those runs used, and what other browsers would have spent. Unmeasured
/// runs contribute nothing and are excluded from `measuredRunCount`.
async fn skill_token_savings(
    state: &AppState,
    runs: &[skill_runs::Model],
) -> AppResult<models::SkillTokenSavings> {
    let session_ids: Vec<String> = runs.iter().map(|run| run.session_id.clone()).collect();
    let (window, measured_run_count) = state
        .session_efficiency
        .aggregate_for_sessions(&session_ids)
        .await?;
    Ok(models::SkillTokenSavings::new(
        window.raw_token_savings_estimate,
        window.screenshot_first_token_estimate,
        window.browser_claw_token_estimate,
        measured_run_count,
    ))
}

fn run_to_dto(run: skill_runs::Model) -> SkillRun {
    let mut dto = SkillRun::new(
        run.id,
        run.skill_name,
        run.session_id,
        run.run_number,
        run.agent_id,
        run.clean,
        run.created_at,
    );
    dto.tokens = run.tokens;
    dto.duration_ms = run.duration_ms;
    dto.tool_count = run.tool_count;
    dto.errored_tool = run.errored_tool;
    dto.mode = Some(match run.mode.as_str() {
        "ultrafast" => models::SkillRunMode::Ultrafast,
        "handed_back" => models::SkillRunMode::HandedBack,
        _ => models::SkillRunMode::Agent,
    });
    dto.handover_step = run.handover_step;
    dto.handover_cause = run.handover_cause.as_deref().map(|cause| match cause {
        "site_changed" => models::SkillRunHandoverCause::SiteChanged,
        "input_only" => models::SkillRunHandoverCause::InputOnly,
        "transient" => models::SkillRunHandoverCause::Transient,
        _ => models::SkillRunHandoverCause::Unknown,
    });
    dto
}

fn origin_to_dto(origin: &str) -> models::SkillOrigin {
    match origin {
        "manual" => models::SkillOrigin::Manual,
        "directory" => models::SkillOrigin::Directory,
        _ => models::SkillOrigin::Agent,
    }
}

fn agent_to_harness(agent: AgentId) -> models::Harness {
    match agent {
        AgentId::ClaudeCode => models::Harness::ClaudeCode,
        AgentId::Codex => models::Harness::Codex,
        AgentId::Cursor => models::Harness::Cursor,
        AgentId::OpenCode => models::Harness::OpenCode,
        AgentId::Antigravity => models::Harness::Antigravity,
        AgentId::VsCode => models::Harness::VsCode,
        AgentId::Zed => models::Harness::Zed,
    }
}
