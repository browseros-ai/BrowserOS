use super::{error, internal};
use crate::{
    AppState,
    db::entities::{skill_flows, skill_runs},
    error::{AppError, AppResult, CanonicalError, RequestId},
    services::{
        flow::{self, FlowStatus, ULTRAFAST_ENABLED},
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
    let mut flows: HashMap<String, skill_flows::Model> = if ULTRAFAST_ENABLED {
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
        if ULTRAFAST_ENABLED {
            dto.ultrafast = Some(Box::new(ultrafast_to_dto(flow.as_ref())));
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
    if ULTRAFAST_ENABLED {
        skill.ultrafast = Some(Box::new(ultrafast_to_dto(None)));
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
    let detail = if ULTRAFAST_ENABLED {
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

/// Lets a skill's write flow run without asking again. Only a flow that writes needs it.
pub(super) async fn approve_write(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Skill>, CanonicalError> {
    let row = state
        .skills
        .flow_row(&name)
        .await
        .map_err(|source| map_error(&request_id, source))?;
    let Some(mut row) = row.filter(|row| row.effects.as_deref() == Some("write")) else {
        return Err(map_error(
            &request_id,
            AppError::conflict("this skill has no write flow to approve"),
        ));
    };
    row.write_approved = true;
    row.updated_at = crate::clock::now_epoch_ms();
    state
        .skills
        .put_flow_row(row.clone())
        .await
        .map_err(|source| map_error(&request_id, source))?;
    let detail = state
        .skills
        .get(&name)
        .await
        .map_err(|source| map_error(&request_id, source))?;
    let mut skill = skill_to_dto(detail.view);
    skill.ultrafast = Some(Box::new(ultrafast_to_dto(Some(&row))));
    Ok(Json(skill))
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

/// Fills the detail's ultrafast state, only while ultrafast skills are enabled.
async fn with_ultrafast(
    state: &AppState,
    request_id: &RequestId,
    mut detail: SkillDetail,
) -> Result<SkillDetail, CanonicalError> {
    if ULTRAFAST_ENABLED {
        let flow = state
            .skills
            .flow_row(&detail.skill.name)
            .await
            .map_err(|source| map_error(request_id, source))?;
        detail.skill.ultrafast = Some(Box::new(ultrafast_to_dto(flow.as_ref())));
    }
    Ok(detail)
}

pub(super) fn ultrafast_to_dto(flow: Option<&skill_flows::Model>) -> models::SkillUltrafast {
    let Some(flow) = flow else {
        return models::SkillUltrafast::new(models::SkillUltrafastStatus::None, 0, false);
    };
    let status = match FlowStatus::parse(&flow.status) {
        FlowStatus::None => models::SkillUltrafastStatus::None,
        FlowStatus::Pending => models::SkillUltrafastStatus::Pending,
        FlowStatus::Verified => models::SkillUltrafastStatus::Verified,
        FlowStatus::Degraded => models::SkillUltrafastStatus::Degraded,
        FlowStatus::Broken => models::SkillUltrafastStatus::Broken,
        FlowStatus::Off => models::SkillUltrafastStatus::Off,
    };
    let mut dto =
        models::SkillUltrafast::new(status, flow.flow_version, flow::writes_allowed(flow));
    dto.reason = flow.status_reason.clone();
    dto.effects = match flow.effects.as_deref() {
        Some("read") => Some(models::SkillUltrafastEffects::Read),
        Some("write") => Some(models::SkillUltrafastEffects::Write),
        _ => None,
    };
    dto.p50_ms = flow.p50_ms;
    dto.verified_at = flow.verified_at;
    dto.repaired_at = flow.repaired_at;
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
