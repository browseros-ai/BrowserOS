use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use claw_server_rust::{
    AppState, build_router,
    config::Config,
    db::audit_log::{RecordToolDispatchInput, bounded_args_json, result_meta},
    ids::DispatchId,
    services::skills::{CreateSkill, SkillOrigin},
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tempfile::TempDir;
use tower::ServiceExt;

struct TestApp {
    router: Router,
    _dir: TempDir,
    root: PathBuf,
    state: AppState,
}

async fn test_app() -> anyhow::Result<TestApp> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("browserclaw");
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49361,
        proxy_port: None,
        resources_dir: dir.path().join("resources"),
        browserclaw_dir: root.clone(),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
        auth_token: None,
    });
    let state = AppState::new_with_home(config, dir.path().join("home")).await?;
    Ok(TestApp {
        router: build_router(state.clone()),
        _dir: dir,
        root,
        state,
    })
}

async fn request(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost");
    let request_body = if let Some(body) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = router.clone().oneshot(builder.body(request_body)?).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)?
    };
    Ok((status, value))
}

#[tokio::test]
async fn skill_crud_round_trips_and_writes_the_canonical_file() -> anyhow::Result<()> {
    let app = test_app().await?;
    let router = &app.router;

    let (status, list) = request(router, "GET", "/api/v1/skills", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["items"].as_array().map(Vec::len), Some(0));

    let (status, created) = request(
        router,
        "POST",
        "/api/v1/skills",
        Some(json!({
            "name": "inbox-sweep",
            "description": "Check the inbox and draft replies",
            "site": "mail.google.com",
            "steps": ["Open the inbox", "Draft replies", "Leave drafts unsent"],
            "learnedNotes": ["Read the DOM snapshot, not screenshots"]
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["name"], "inbox-sweep");
    assert_eq!(created["origin"], "manual");
    assert_eq!(created["version"].as_i64(), Some(1));
    assert_eq!(created["runCount"].as_i64(), Some(0));
    assert_eq!(created["site"], "mail.google.com");

    let skill_md = app.root.join("skills").join("inbox-sweep").join("SKILL.md");
    let content = std::fs::read_to_string(&skill_md)?;
    assert!(content.contains("name: inbox-sweep"));
    assert!(content.contains("tools: browseros-neo"));
    assert!(content.contains("## Steps"));
    assert!(content.contains("Read the DOM snapshot, not screenshots"));

    let (status, detail) = request(router, "GET", "/api/v1/skills/inbox-sweep", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["skill"]["name"], "inbox-sweep");
    assert!(
        detail["body"]
            .as_str()
            .is_some_and(|body| body.contains("name: inbox-sweep"))
    );
    assert_eq!(detail["runs"].as_array().map(Vec::len), Some(0));

    let (_, list) = request(router, "GET", "/api/v1/skills", None).await?;
    assert_eq!(list["items"].as_array().map(Vec::len), Some(1));

    let (status, runs) = request(router, "GET", "/api/v1/skills/inbox-sweep/runs", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(runs["items"].as_array().map(Vec::len), Some(0));

    let (status, updated) = request(
        router,
        "PUT",
        "/api/v1/skills/inbox-sweep",
        Some(json!({
            "description": "Updated description",
            "body": "---\nname: inbox-sweep\ndescription: Updated\ntools: browseros-neo\n---\n\n## Steps\n1. A brand new step\n"
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["skill"]["version"].as_i64(), Some(2));
    assert_eq!(updated["skill"]["description"], "Updated description");
    assert!(
        updated["body"]
            .as_str()
            .is_some_and(|body| body.contains("A brand new step"))
    );
    let content = std::fs::read_to_string(&skill_md)?;
    assert!(content.contains("A brand new step"));

    let (status, _) = request(router, "DELETE", "/api/v1/skills/inbox-sweep", None).await?;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = request(router, "GET", "/api/v1/skills/inbox-sweep", None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!app.root.join("skills").join("inbox-sweep").exists());

    Ok(())
}

#[tokio::test]
async fn skill_create_rejects_bad_names_and_duplicates() -> anyhow::Result<()> {
    let app = test_app().await?;
    let router = &app.router;

    let (status, _) = request(
        router,
        "POST",
        "/api/v1/skills",
        Some(json!({ "name": "Bad Name", "description": "invalid slug" })),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = request(
        router,
        "POST",
        "/api/v1/skills",
        Some(json!({ "name": "browserclaw", "description": "reserved name" })),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _) = request(
        router,
        "POST",
        "/api/v1/skills",
        Some(json!({ "name": "daily-brief", "description": "first" })),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = request(
        router,
        "POST",
        "/api/v1/skills",
        Some(json!({ "name": "daily-brief", "description": "duplicate" })),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _) = request(router, "GET", "/api/v1/skills/missing", None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn skill_create_escapes_yaml_sensitive_descriptions() -> anyhow::Result<()> {
    let app = test_app().await?;

    let (status, _) = request(
        &app.router,
        "POST",
        "/api/v1/skills",
        Some(json!({
            "name": "tricky-desc",
            "description": "Reply within 24h: keep it brief\nthen stop"
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);

    let content =
        std::fs::read_to_string(app.root.join("skills").join("tricky-desc").join("SKILL.md"))?;
    // The description is emitted as a quoted scalar, so a raw newline or colon
    // cannot break the frontmatter block.
    assert!(content.contains(r#"description: "Reply within 24h: keep it brief\nthen stop""#));
    assert_eq!(content.matches("---").count(), 2);

    Ok(())
}

fn dispatch(session_id: &str, tool: &str, is_error: bool) -> RecordToolDispatchInput {
    RecordToolDispatchInput {
        agent_id: "convo-run".to_string(),
        slug: "codex".to_string(),
        agent_label: "codex/inbox".to_string(),
        session_id: session_id.to_string(),
        tool_name: tool.to_string(),
        page_id: None,
        tab_id: None,
        target_id: None,
        url: None,
        title: None,
        args_json: bounded_args_json(&json!({})),
        result_meta: result_meta(is_error, false, &json!({}), 0),
        duration_ms: 100,
        created_at: None,
        dispatch_id: DispatchId::new(),
        parent_dispatch_id: None,
        tool_input_token_estimate: 10,
        tool_output_token_estimate: 20,
        token_estimator_version: 1,
    }
}

#[tokio::test]
async fn skill_run_recorded_from_a_marked_session() -> anyhow::Result<()> {
    let app = test_app().await?;
    let state = &app.state;

    state
        .skills
        .create(CreateSkill {
            name: "inbox-sweep".to_string(),
            description: "Check the inbox".to_string(),
            site: None,
            steps: vec!["Read the inbox".to_string()],
            learned_notes: vec![],
            origin: SkillOrigin::Agent,
            source_session_id: None,
        })
        .await?;

    // A completed session with one clean dispatch and one that errored.
    state
        .audit_log
        .record_session_start(
            "run-sess",
            "convo-run",
            "codex",
            "codex/inbox",
            "codex",
            "1",
        )
        .await?;
    state
        .audit_log
        .record_tool_dispatch(dispatch("run-sess", "read", false))
        .await?;
    state
        .audit_log
        .record_tool_dispatch(dispatch("run-sess", "act", true))
        .await?;
    state
        .audit_log
        .record_session_end("run-sess", "closed", None)
        .await?;

    state.skill_runs.mark("run-sess", "inbox-sweep").await?;
    assert!(state.skill_runs.finalize("run-sess").await?);
    // Projecting again is a no-op.
    assert!(!state.skill_runs.finalize("run-sess").await?);

    let detail = state.skills.get("inbox-sweep").await?;
    assert_eq!(detail.runs.len(), 1);
    let run = &detail.runs[0];
    assert_eq!(run.run_number, 1);
    assert_eq!(run.tool_count, Some(2));
    assert_eq!(run.tokens, Some(60));
    assert!(!run.clean);
    assert_eq!(run.errored_tool.as_deref(), Some("act"));
    assert_eq!(detail.view.stats.run_count, 1);
    assert_eq!(detail.view.stats.clean_run_count, 0);

    // A completed but unmarked session records nothing.
    state
        .audit_log
        .record_session_start(
            "plain-sess",
            "convo-run",
            "codex",
            "codex/plain",
            "codex",
            "1",
        )
        .await?;
    state
        .audit_log
        .record_tool_dispatch(dispatch("plain-sess", "read", false))
        .await?;
    state
        .audit_log
        .record_session_end("plain-sess", "closed", None)
        .await?;
    assert!(!state.skill_runs.finalize("plain-sess").await?);
    assert_eq!(state.skills.get("inbox-sweep").await?.runs.len(), 1);

    Ok(())
}

#[tokio::test]
async fn mark_rejects_unknown_skill_and_finalize_skips_a_deleted_one() -> anyhow::Result<()> {
    let app = test_app().await?;
    let state = &app.state;

    // A name that does not resolve to a saved skill is rejected at mark time.
    assert!(
        state
            .skill_runs
            .mark("some-session", "not-a-skill")
            .await
            .is_err()
    );

    // A skill deleted between the mark and completion records no run.
    state
        .skills
        .create(CreateSkill {
            name: "brief".to_string(),
            description: "Daily brief".to_string(),
            site: None,
            steps: vec![],
            learned_notes: vec![],
            origin: SkillOrigin::Agent,
            source_session_id: None,
        })
        .await?;
    state
        .audit_log
        .record_session_start(
            "gone-sess",
            "convo-run",
            "codex",
            "codex/brief",
            "codex",
            "1",
        )
        .await?;
    state
        .audit_log
        .record_tool_dispatch(dispatch("gone-sess", "read", false))
        .await?;
    state
        .audit_log
        .record_session_end("gone-sess", "closed", None)
        .await?;
    state.skill_runs.mark("gone-sess", "brief").await?;
    state.skills.delete("brief").await?;
    assert!(!state.skill_runs.finalize("gone-sess").await?);

    Ok(())
}

#[tokio::test]
async fn concurrent_upserts_of_a_new_name_keep_one_skill_intact() -> anyhow::Result<()> {
    let app = test_app().await?;
    let skills = app.state.skills.clone();
    let input = |description: &str| CreateSkill {
        name: "race-skill".to_string(),
        description: description.to_string(),
        site: None,
        steps: vec!["Do the thing".to_string()],
        learned_notes: vec![],
        origin: SkillOrigin::Agent,
        source_session_id: None,
    };

    let (first, second) = tokio::join!(skills.upsert(input("A")), skills.upsert(input("B")));
    // Both calls settle without error: one creates, the other updates in place;
    // neither call's rollback removes the other's installed skill.
    first?;
    second?;

    let detail = skills.get("race-skill").await?;
    assert!(
        app.root
            .join("skills")
            .join("race-skill")
            .join("SKILL.md")
            .exists()
    );
    assert!(detail.body.contains("Do the thing"));
    // A create followed by an update leaves the skill at version 2.
    assert_eq!(detail.view.model.version, 2);

    Ok(())
}
