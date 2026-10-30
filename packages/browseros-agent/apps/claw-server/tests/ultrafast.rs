use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use claw_server::{
    AppState, build_router,
    config::Config,
    db::{
        audit_log::{RecordToolDispatchInput, bounded_args_json, result_meta},
        skills::RunMode,
    },
    ids::DispatchId,
    services::{
        flow::{self, FlowStatus},
        skills::{CreateSkill, SkillOrigin},
    },
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tempfile::TempDir;
use tower::ServiceExt;

struct TestApp {
    router: Router,
    _dir: TempDir,
    state: AppState,
}

async fn test_app() -> anyhow::Result<TestApp> {
    let dir = tempfile::tempdir()?;
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49361,
        proxy_port: None,
        resources_dir: dir.path().join("resources"),
        browserclaw_dir: dir.path().join("browserclaw"),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
        native: Default::default(),
        mode: Default::default(),
        behavior: Default::default(),
        behavior_ignored: Vec::new(),
    });
    let state = AppState::new_with_home(config, dir.path().join("home")).await?;
    Ok(TestApp {
        router: build_router(state.clone()),
        _dir: dir,
        state,
    })
}

async fn get(router: &Router, uri: &str) -> anyhow::Result<(StatusCode, Value)> {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::HOST, "localhost")
        .body(Body::empty())?;
    let response = router.clone().oneshot(request).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)?
    };
    Ok((status, value))
}

async fn create_skill(state: &AppState, name: &str) -> anyhow::Result<()> {
    state
        .skills
        .create(CreateSkill {
            name: name.to_string(),
            description: "Add a product to the shop cart".to_string(),
            site: None,
            steps: vec!["Open the search".to_string(), "Add it".to_string()],
            learned_notes: vec![],
            origin: SkillOrigin::Agent,
            source_session_id: None,
        })
        .await?;
    Ok(())
}

fn dispatch(session_id: &str) -> RecordToolDispatchInput {
    RecordToolDispatchInput {
        agent_id: "convo-run".to_string(),
        slug: "codex".to_string(),
        agent_label: "codex/shop".to_string(),
        session_id: session_id.to_string(),
        tool_name: "run".to_string(),
        page_id: None,
        tab_id: None,
        target_id: None,
        url: None,
        title: None,
        args_json: bounded_args_json(&json!({})),
        result_meta: result_meta(false, false, &json!({}), 0),
        duration_ms: 100,
        created_at: None,
        dispatch_id: DispatchId::new(),
        parent_dispatch_id: None,
        tool_input_token_estimate: 10,
        tool_output_token_estimate: 20,
        token_estimator_version: 1,
    }
}

async fn finished_session(state: &AppState, session: &str) -> anyhow::Result<()> {
    state
        .audit_log
        .record_session_start(session, "convo-run", "codex", "codex/shop", "codex", "1")
        .await?;
    state
        .audit_log
        .record_tool_dispatch(dispatch(session))
        .await?;
    state
        .audit_log
        .record_session_end(session, "closed", None)
        .await?;
    Ok(())
}

#[tokio::test]
async fn a_handed_back_run_keeps_its_mode_even_when_the_agent_marks_it_too() -> anyhow::Result<()> {
    let app = test_app().await?;
    let state = &app.state;
    create_skill(state, "neo-shop-add").await?;
    finished_session(state, "uf-sess").await?;

    state
        .skill_runs
        .mark_as(
            "uf-sess",
            "neo-shop-add",
            &RunMode::handed_back(2, "site_changed"),
        )
        .await?;
    // The agent finishing by hand marks the run as well; that must not erase the handover.
    state.skill_runs.mark("uf-sess", "neo-shop-add").await?;
    assert!(state.skill_runs.finalize("uf-sess").await?);

    let run = &state.skills.get("neo-shop-add").await?.runs[0];
    assert_eq!(run.mode, "handed_back");
    assert_eq!(run.handover_step, Some(2));
    assert_eq!(run.handover_cause.as_deref(), Some("site_changed"));

    let (status, runs) = get(&app.router, "/api/v1/skills/neo-shop-add/runs").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(runs["items"][0]["mode"], "handed_back");
    assert_eq!(runs["items"][0]["handoverStep"], 2);
    assert_eq!(runs["items"][0]["handoverCause"], "site_changed");
    Ok(())
}

#[tokio::test]
async fn a_plain_agent_run_is_recorded_as_agent() -> anyhow::Result<()> {
    let app = test_app().await?;
    let state = &app.state;
    create_skill(state, "neo-shop-add").await?;
    finished_session(state, "plain-sess").await?;
    state.skill_runs.mark("plain-sess", "neo-shop-add").await?;
    assert!(state.skill_runs.finalize("plain-sess").await?);
    let run = &state.skills.get("neo-shop-add").await?.runs[0];
    assert_eq!(run.mode, "agent");
    assert_eq!(run.handover_step, None);
    Ok(())
}

#[tokio::test]
async fn ultrafast_state_is_reported_on_skills() -> anyhow::Result<()> {
    let app = test_app().await?;
    let state = &app.state;
    create_skill(state, "neo-shop-add").await?;

    let (_, detail) = get(&app.router, "/api/v1/skills/neo-shop-add").await?;
    assert_eq!(detail["skill"]["ultrafast"]["status"], "none");

    let mut row = flow::new_row("neo-shop-add", FlowStatus::Verified);
    row.flow_version = 2;
    row.effects = Some("write".to_string());
    row.p50_ms = Some(1_200);
    state.skills.put_flow_row(row).await?;
    let (_, list) = get(&app.router, "/api/v1/skills").await?;
    let ultrafast = &list["items"][0]["ultrafast"];
    assert_eq!(ultrafast["status"], "verified");
    assert_eq!(ultrafast["flowVersion"], 2);
    assert_eq!(ultrafast["effects"], "write");
    assert_eq!(ultrafast["writeApproved"], false);
    assert_eq!(ultrafast["p50Ms"], 1_200);

    state.skills.delete("neo-shop-add").await?;
    assert!(state.skills.flow_row("neo-shop-add").await?.is_none());
    Ok(())
}

async fn put(router: &Router, uri: &str, body: Value) -> anyhow::Result<(StatusCode, Value)> {
    let request = Request::builder()
        .method("PUT")
        .uri(uri)
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))?;
    let response = router.clone().oneshot(request).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

fn two_step_flow() -> flow::store::FlowDoc {
    use flow::store::{Effects, FlowDoc, FlowMeta, FlowStep};
    FlowDoc {
        meta: FlowMeta {
            name: "neo-shop-add".to_string(),
            flow_version: 1,
            effects: Effects::Read,
            hosts: vec![],
            inputs: [("query".to_string(), "string".to_string())].into(),
            output: Default::default(),
            examples: vec![],
            source_session: None,
        },
        steps: (1..=2)
            .map(|n| FlowStep {
                n,
                commit: false,
                src: "async () => {}".to_string(),
            })
            .collect(),
    }
}

#[tokio::test]
async fn editing_steps_keeps_run_it_fast_only_while_they_line_up() -> anyhow::Result<()> {
    let app = test_app().await?;
    let state = &app.state;
    create_skill(state, "neo-shop-add").await?;
    flow::store::write(&state.skills.skill_dir("neo-shop-add"), &two_step_flow())?;
    state
        .skills
        .put_flow_row(flow::new_row("neo-shop-add", FlowStatus::Verified))
        .await?;

    let (status, detail) = put(
        &app.router,
        "/api/v1/skills/neo-shop-add",
        json!({ "steps": ["Open the search", "Add the first match"], "learnedNotes": [] }),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let body = detail["body"].as_str().unwrap_or_default();
    assert!(body.contains("## Run it fast"), "{body}");
    assert!(body.contains("1. Open the search"), "{body}");

    let (_, detail) = put(
        &app.router,
        "/api/v1/skills/neo-shop-add",
        json!({ "steps": ["Open", "Look", "Add"], "learnedNotes": [] }),
    )
    .await?;
    assert_eq!(detail["skill"]["ultrafast"]["status"], "off");
    let body = detail["body"].as_str().unwrap_or_default();
    assert!(!body.contains("Run it fast"), "{body}");
    assert!(body.contains("1. Call the mark_skill_run tool"), "{body}");
    Ok(())
}
