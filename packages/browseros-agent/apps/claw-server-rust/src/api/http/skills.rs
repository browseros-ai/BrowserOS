use super::{error, internal};
use crate::{
    AppState,
    db::entities::skill_runs,
    error::{AppError, CanonicalError, RequestId},
    services::skills::{CreateSkill, SkillDetailView, SkillOrigin, SkillView, UpdateSkill},
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use claw_api::models::{self, Skill, SkillCreate, SkillDetail, SkillList, SkillRun, SkillRunList};
use harness_integrations::AgentId;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct RunsQuery {
    cursor: Option<i64>,
    limit: Option<u64>,
}

pub(super) async fn list(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<SkillList>, CanonicalError> {
    let items = state
        .skills
        .list()
        .await
        .map_err(|source| map_error(&request_id, source))?
        .into_iter()
        .map(skill_to_dto)
        .collect();
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
    Ok(Json(detail_to_dto(detail)))
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
    Ok((StatusCode::CREATED, Json(skill_to_dto(view))))
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
    Ok(Json(detail_to_dto(detail)))
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

fn detail_to_dto(detail: SkillDetailView) -> SkillDetail {
    let SkillDetailView { view, body, runs } = detail;
    SkillDetail::new(
        skill_to_dto(view),
        body,
        runs.into_iter().map(run_to_dto).collect(),
    )
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
