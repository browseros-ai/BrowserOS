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
            "name": "neo-inbox-sweep",
            "description": "Check the inbox and draft replies",
            "site": "mail.google.com",
            "steps": ["Open the inbox", "Draft replies", "Leave drafts unsent"],
            "learnedNotes": ["Read the DOM snapshot, not screenshots"]
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["name"], "neo-inbox-sweep");
    assert_eq!(created["origin"], "manual");
    assert_eq!(created["version"].as_i64(), Some(1));
    assert_eq!(created["runCount"].as_i64(), Some(0));
    assert_eq!(created["site"], "mail.google.com");

    let skill_md = app
        .root
        .join("skills")
        .join("neo-inbox-sweep")
        .join("SKILL.md");
    let content = std::fs::read_to_string(&skill_md)?;
    assert!(content.contains("name: neo-inbox-sweep"));
    assert!(content.contains("tools: browseros-neo"));
    assert!(content.contains("## Steps"));
    assert!(content.contains("Read the DOM snapshot, not screenshots"));

    let (status, detail) = request(router, "GET", "/api/v1/skills/neo-inbox-sweep", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["skill"]["name"], "neo-inbox-sweep");
    assert!(
        detail["body"]
            .as_str()
            .is_some_and(|body| body.contains("name: neo-inbox-sweep"))
    );
    assert_eq!(detail["runs"].as_array().map(Vec::len), Some(0));

    let (_, list) = request(router, "GET", "/api/v1/skills", None).await?;
    assert_eq!(list["items"].as_array().map(Vec::len), Some(1));

    let (status, runs) =
        request(router, "GET", "/api/v1/skills/neo-inbox-sweep/runs", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(runs["items"].as_array().map(Vec::len), Some(0));

    let (status, updated) = request(
        router,
        "PUT",
        "/api/v1/skills/neo-inbox-sweep",
        Some(json!({
            "description": "Updated description",
            "body": "---\nname: neo-inbox-sweep\ndescription: Updated\ntools: browseros-neo\n---\n\n## Steps\n1. A brand new step\n"
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

    let (status, _) = request(router, "DELETE", "/api/v1/skills/neo-inbox-sweep", None).await?;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = request(router, "GET", "/api/v1/skills/neo-inbox-sweep", None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!app.root.join("skills").join("neo-inbox-sweep").exists());

    Ok(())
}

#[tokio::test]
async fn skill_structured_edit_re_renders_frontmatter_and_clears_site() -> anyhow::Result<()> {
    let app = test_app().await?;
    let router = &app.router;

    request(
        router,
        "POST",
        "/api/v1/skills",
        Some(json!({
            "name": "neo-inbox-sweep",
            "description": "First description",
            "site": "mail.google.com",
            "steps": ["Old step"]
        })),
    )
    .await?;

    let (status, updated) = request(
        router,
        "PUT",
        "/api/v1/skills/neo-inbox-sweep",
        Some(json!({
            "description": "Second description",
            "site": "",
            "steps": ["New step"],
            "learnedNotes": ["Prefer the DOM snapshot"]
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["skill"]["description"], "Second description");
    assert_eq!(updated["skill"]["version"].as_i64(), Some(2));
    // An empty site string clears the column, so it is omitted from the DTO.
    assert!(updated["skill"]["site"].is_null());

    // The on-disk frontmatter description tracks the column: agents reading the
    // file see the new description, never the stale one.
    let skill_md = app
        .root
        .join("skills")
        .join("neo-inbox-sweep")
        .join("SKILL.md");
    let content = std::fs::read_to_string(&skill_md)?;
    assert!(content.contains(r#"description: "Second description""#));
    assert!(!content.contains("First description"));
    assert!(content.contains("New step"));
    assert!(content.contains("Prefer the DOM snapshot"));
    assert!(!content.contains("Old step"));

    Ok(())
}

#[tokio::test]
async fn skill_metadata_only_description_edit_patches_frontmatter() -> anyhow::Result<()> {
    let app = test_app().await?;
    let router = &app.router;

    request(
        router,
        "POST",
        "/api/v1/skills",
        Some(json!({
            "name": "neo-daily-brief",
            "description": "Old description",
            "steps": ["Keep this step"]
        })),
    )
    .await?;

    let (status, updated) = request(
        router,
        "PUT",
        "/api/v1/skills/neo-daily-brief",
        Some(json!({ "description": "Fresh description" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["skill"]["description"], "Fresh description");

    // Only the frontmatter description line changes; the body is preserved.
    let skill_md = app
        .root
        .join("skills")
        .join("neo-daily-brief")
        .join("SKILL.md");
    let content = std::fs::read_to_string(&skill_md)?;
    assert!(content.contains(r#"description: "Fresh description""#));
    assert!(!content.contains("Old description"));
    assert!(content.contains("Keep this step"));

    Ok(())
}

#[tokio::test]
async fn skill_update_rejects_ambiguous_field_combinations() -> anyhow::Result<()> {
    let app = test_app().await?;
    let router = &app.router;

    request(
        router,
        "POST",
        "/api/v1/skills",
        Some(json!({ "name": "neo-inbox-sweep", "description": "First", "steps": ["Old step"] })),
    )
    .await?;

    // A raw body plus structured fields is ambiguous: reject it rather than
    // silently discard one representation.
    let (status, _) = request(
        router,
        "PUT",
        "/api/v1/skills/neo-inbox-sweep",
        Some(json!({
            "body": "---\nname: neo-inbox-sweep\ndescription: X\ntools: browseros-neo\n---\n",
            "steps": ["New step"],
            "learnedNotes": []
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Only one of the two structured sections would silently erase the other.
    let (status, _) = request(
        router,
        "PUT",
        "/api/v1/skills/neo-inbox-sweep",
        Some(json!({ "steps": ["New step"] })),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The original steps survive both rejected updates.
    let (_, detail) = request(router, "GET", "/api/v1/skills/neo-inbox-sweep", None).await?;
    assert!(
        detail["body"]
            .as_str()
            .is_some_and(|body| body.contains("Old step"))
    );

    Ok(())
}

#[tokio::test]
async fn skill_description_edit_adds_frontmatter_to_a_raw_body() -> anyhow::Result<()> {
    let app = test_app().await?;
    let router = &app.router;

    request(
        router,
        "POST",
        "/api/v1/skills",
        Some(json!({ "name": "neo-raw-note", "description": "First" })),
    )
    .await?;

    // Store a raw body that has no frontmatter fence.
    let (status, _) = request(
        router,
        "PUT",
        "/api/v1/skills/neo-raw-note",
        Some(json!({ "body": "Just prose, no frontmatter\n" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);

    // A description-only edit must still land in the file: a frontmatter block
    // is prepended so linked agents read the new description.
    let (status, _) = request(
        router,
        "PUT",
        "/api/v1/skills/neo-raw-note",
        Some(json!({ "description": "Now described" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);

    let skill_md = app
        .root
        .join("skills")
        .join("neo-raw-note")
        .join("SKILL.md");
    let content = std::fs::read_to_string(&skill_md)?;
    assert!(content.starts_with("---\nname: neo-raw-note\n"));
    assert!(content.contains(r#"description: "Now described""#));
    assert!(content.contains("Just prose, no frontmatter"));

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

    // The embedded product skill occupies "browserclaw"; the neo- namespace
    // neutralizes the old collision, so a user "browserclaw" now saves cleanly
    // as "neo-browserclaw" instead of being rejected.
    let (status, created) = request(
        router,
        "POST",
        "/api/v1/skills",
        Some(json!({ "name": "browserclaw", "description": "no longer collides" })),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["name"], "neo-browserclaw");

    let (status, _) = request(
        router,
        "POST",
        "/api/v1/skills",
        Some(json!({ "name": "neo-daily-brief", "description": "first" })),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = request(
        router,
        "POST",
        "/api/v1/skills",
        Some(json!({ "name": "neo-daily-brief", "description": "duplicate" })),
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
            "name": "neo-tricky-desc",
            "description": "Reply within 24h: keep it brief\nthen stop"
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);

    let content = std::fs::read_to_string(
        app.root
            .join("skills")
            .join("neo-tricky-desc")
            .join("SKILL.md"),
    )?;
    // The description is emitted as a quoted scalar, so a raw newline or colon
    // cannot break the frontmatter block.
    assert!(content.contains(r#"description: "Reply within 24h: keep it brief\nthen stop""#));
    assert_eq!(content.matches("---").count(), 2);

    Ok(())
}

#[tokio::test]
async fn skill_create_namespaces_a_bare_name_under_neo() -> anyhow::Result<()> {
    let app = test_app().await?;
    let router = &app.router;

    // A bare name is namespaced under neo- on the way in.
    let (status, created) = request(
        router,
        "POST",
        "/api/v1/skills",
        Some(json!({
            "name": "weather",
            "description": "Check today's weather",
            "steps": ["Open the forecast", "Read the high and low"]
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["name"], "neo-weather");

    // The canonical file lives under the prefixed name, and its frontmatter and
    // the mark step both carry the prefixed name so a re-run marks correctly.
    let skill_md = app.root.join("skills").join("neo-weather").join("SKILL.md");
    let content = std::fs::read_to_string(&skill_md)?;
    assert!(content.contains("name: neo-weather"));
    assert!(content.contains("mark_skill_run tool with name: neo-weather"));
    assert!(!app.root.join("skills").join("weather").exists());

    // The read path is exact: the prefixed name resolves, the bare one does not.
    let (status, _) = request(router, "GET", "/api/v1/skills/neo-weather", None).await?;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = request(router, "GET", "/api/v1/skills/weather", None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Re-posting the already-prefixed name hits the same skill (a duplicate),
    // never nesting into neo-neo-weather.
    let (status, _) = request(
        router,
        "POST",
        "/api/v1/skills",
        Some(json!({ "name": "neo-weather", "description": "Now with the hourly view" })),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(!app.root.join("skills").join("neo-neo-weather").exists());

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
            name: "neo-inbox-sweep".to_string(),
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

    state.skill_runs.mark("run-sess", "neo-inbox-sweep").await?;
    assert!(state.skill_runs.finalize("run-sess").await?);
    // Projecting again is a no-op.
    assert!(!state.skill_runs.finalize("run-sess").await?);

    let detail = state.skills.get("neo-inbox-sweep").await?;
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
    assert_eq!(state.skills.get("neo-inbox-sweep").await?.runs.len(), 1);

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
            name: "neo-brief".to_string(),
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
    state.skill_runs.mark("gone-sess", "neo-brief").await?;
    state.skills.delete("neo-brief").await?;
    assert!(!state.skill_runs.finalize("gone-sess").await?);

    Ok(())
}

#[tokio::test]
async fn deleting_a_skill_discards_its_run_history() -> anyhow::Result<()> {
    let app = test_app().await?;
    let state = &app.state;
    let new_skill = || CreateSkill {
        name: "neo-inbox-sweep".to_string(),
        description: "Check the inbox".to_string(),
        site: None,
        steps: vec!["Read the inbox".to_string()],
        learned_notes: vec![],
        origin: SkillOrigin::Agent,
        source_session_id: None,
    };

    state.skills.create(new_skill()).await?;

    // One completed, marked run gives the skill some history.
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
        .record_session_end("run-sess", "closed", None)
        .await?;
    state.skill_runs.mark("run-sess", "neo-inbox-sweep").await?;
    assert!(state.skill_runs.finalize("run-sess").await?);
    assert_eq!(state.skills.get("neo-inbox-sweep").await?.runs.len(), 1);

    // A second session is marked but not yet finalized: a pending mark.
    state
        .skill_runs
        .mark("pending-sess", "neo-inbox-sweep")
        .await?;

    // Delete, then recreate a task under the same name.
    state.skills.delete("neo-inbox-sweep").await?;
    state.skills.create(new_skill()).await?;

    // The recreated task starts clean: it inherits no runs or counts.
    let detail = state.skills.get("neo-inbox-sweep").await?;
    assert!(detail.runs.is_empty());
    assert_eq!(detail.view.stats.run_count, 0);
    assert_eq!(detail.view.stats.clean_run_count, 0);

    // The deleted task's pending mark is gone, so completing that session
    // records nothing against the recreated task.
    state
        .audit_log
        .record_session_start(
            "pending-sess",
            "convo-run",
            "codex",
            "codex/inbox",
            "codex",
            "1",
        )
        .await?;
    state
        .audit_log
        .record_tool_dispatch(dispatch("pending-sess", "read", false))
        .await?;
    state
        .audit_log
        .record_session_end("pending-sess", "closed", None)
        .await?;
    assert!(!state.skill_runs.finalize("pending-sess").await?);
    assert!(state.skills.get("neo-inbox-sweep").await?.runs.is_empty());

    Ok(())
}

#[tokio::test]
async fn concurrent_upserts_of_a_new_name_keep_one_skill_intact() -> anyhow::Result<()> {
    let app = test_app().await?;
    let skills = app.state.skills.clone();
    let input = |description: &str| CreateSkill {
        name: "neo-race-skill".to_string(),
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

    let detail = skills.get("neo-race-skill").await?;
    assert!(
        app.root
            .join("skills")
            .join("neo-race-skill")
            .join("SKILL.md")
            .exists()
    );
    assert!(detail.body.contains("Do the thing"));
    // A create followed by an update leaves the skill at version 2.
    assert_eq!(detail.view.model.version, 2);

    Ok(())
}
