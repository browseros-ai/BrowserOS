//! Whether a skill that changes something runs with the user's agent or without it, as the Skills
//! page sees it over HTTP, and how the plan turns ultrafast skills on and locks the choice.

#[path = "support/fake_gateway.rs"]
mod fake_gateway;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use claw_server::{
    AppState, build_router,
    config::Config,
    services::{
        billing::Billing,
        flow::{
            self, FlowStatus,
            store::{Effects, FlowDoc, FlowMeta, FlowStep},
        },
        skills::{CreateSkill, SkillOrigin},
    },
};
use fake_gateway::FakeGateway;
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

const SKILL: &str = "neo-shop-add";

struct App {
    state: AppState,
    router: Router,
    dir: TempDir,
}

impl App {
    async fn call(
        &self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> anyhow::Result<(StatusCode, Value)> {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "localhost");
        let body = match body {
            Some(body) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(body.to_string())
            }
            None => Body::empty(),
        };
        let response = self.router.clone().oneshot(request.body(body)?).await?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await?;
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)?
        };
        Ok((status, value))
    }

    async fn runs(&self) -> anyhow::Result<Value> {
        let (status, detail) = self
            .call("GET", &format!("/api/v1/skills/{SKILL}"), None)
            .await?;
        assert_eq!(status, StatusCode::OK, "{detail}");
        Ok(detail["skill"]["ultrafast"].clone())
    }

    fn rebuild(&mut self) {
        self.router = build_router(self.state.clone());
    }
}

fn config_for(dir: &Path) -> Arc<Config> {
    Arc::new(Config {
        server_port: 9200,
        cdp_port: 49362,
        proxy_port: None,
        resources_dir: dir.join("resources"),
        browserclaw_dir: dir.join("browserclaw"),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
        native: Default::default(),
        mode: Default::default(),
        behavior: Default::default(),
        behavior_ignored: Vec::new(),
    })
}

async fn app(ultrafast: bool, gateway: Option<&FakeGateway>) -> anyhow::Result<App> {
    let dir = tempfile::tempdir()?;
    let state = AppState::new_with_home_and_billing(
        config_for(dir.path()),
        dir.path().join("home"),
        gateway.map(FakeGateway::config).transpose()?,
    )
    .await?;
    state.ultrafast.set(ultrafast);
    Ok(App {
        router: build_router(state.clone()),
        state,
        dir,
    })
}

fn write_flow(commit_src: &str) -> FlowDoc {
    FlowDoc {
        meta: FlowMeta {
            name: SKILL.to_string(),
            flow_version: 1,
            effects: Effects::Write,
            hosts: vec!["www.amazon.in".to_string()],
            inputs: Default::default(),
            output: Default::default(),
            examples: vec![],
            source_session: None,
            prose_hash: None,
        },
        steps: vec![
            FlowStep {
                n: 1,
                commit: false,
                src: "async (s) => { await s.page.goto('https://www.amazon.in') }".to_string(),
            },
            FlowStep {
                n: 2,
                commit: true,
                src: commit_src.to_string(),
            },
        ],
    }
}

/// A saved skill whose verified flow ends in one click that changes something.
async fn write_skill(state: &AppState, last_step: &str, commit_src: &str) -> anyhow::Result<()> {
    state
        .skills
        .create(CreateSkill {
            name: SKILL.to_string(),
            description: "Add a product to the Amazon cart".to_string(),
            site: Some("amazon.in".to_string()),
            steps: vec!["Open Amazon.".to_string(), last_step.to_string()],
            learned_notes: vec![],
            origin: SkillOrigin::Agent,
            source_session_id: None,
        })
        .await?;
    flow::store::write(&state.skills.skill_dir(SKILL), &write_flow(commit_src))?;
    let mut row = flow::new_row(SKILL, FlowStatus::Verified);
    row.effects = Some("write".to_string());
    row.flow_version = 1;
    row.p50_ms = Some(2_100);
    state.skills.put_flow_row(row).await?;
    Ok(())
}

const ADD_TO_CART: &str = "async (s) => { await s.page.click({ name: 'Add to cart' }) }";

#[tokio::test]
async fn a_write_skill_runs_with_the_agent_until_the_user_chooses_for_its_click()
-> anyhow::Result<()> {
    let app = app(true, None).await?;
    write_skill(&app.state, "Add it to the cart.", ADD_TO_CART).await?;

    let ultrafast = app.runs().await?;
    assert_eq!(ultrafast["locked"], false);
    assert_eq!(ultrafast["writeApproved"], false);
    let runs = &ultrafast["runs"];
    assert_eq!(runs["mode"], "with_agent", "{ultrafast}");
    assert_eq!(runs["chosen"], false);
    assert_eq!(runs["reask"], false);
    assert_eq!(runs["click"], "Add it to the cart.");
    assert_eq!(runs["site"], "amazon.in");

    let (status, skill) = app
        .call(
            "POST",
            &format!("/api/v1/skills/{SKILL}/ultrafast/approve-write"),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{skill}");
    assert_eq!(skill["ultrafast"]["runs"]["mode"], "without_agent");
    assert_eq!(skill["ultrafast"]["runs"]["chosen"], true);
    assert_eq!(skill["ultrafast"]["runs"]["via"], "app");
    assert!(skill["ultrafast"]["runs"]["chosenAt"].as_i64().is_some());
    let body = app.state.skills.get(SKILL).await?.body;
    assert!(body.contains("## Run it fast"), "{body}");

    let (status, skill) = app
        .call(
            "DELETE",
            &format!("/api/v1/skills/{SKILL}/ultrafast/approve-write"),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{skill}");
    assert_eq!(skill["ultrafast"]["runs"]["mode"], "with_agent");
    assert_eq!(skill["ultrafast"]["runs"]["chosen"], true);
    let body = app.state.skills.get(SKILL).await?.body;
    assert!(
        !body.contains("## Run it fast"),
        "a skill that runs with the agent never points it at run_skill: {body}"
    );

    // Choose "without" again, then the click changes: the answer no longer covers it.
    app.call(
        "POST",
        &format!("/api/v1/skills/{SKILL}/ultrafast/approve-write"),
        None,
    )
    .await?;
    flow::store::write(
        &app.state.skills.skill_dir(SKILL),
        &write_flow("async (s) => { await s.page.click({ name: 'Add to basket' }) }"),
    )?;
    let runs = app.runs().await?["runs"].clone();
    assert_eq!(runs["mode"], "with_agent", "{runs}");
    assert_eq!(runs["chosen"], false);
    assert_eq!(runs["reask"], true);
    Ok(())
}

#[tokio::test]
async fn a_skill_that_places_an_order_always_runs_with_the_agent() -> anyhow::Result<()> {
    let app = app(true, None).await?;
    write_skill(
        &app.state,
        "Place the order.",
        "async (s) => { await s.page.click({ name: 'Place your order' }) }",
    )
    .await?;

    let (status, body) = app
        .call(
            "POST",
            &format!("/api/v1/skills/{SKILL}/ultrafast/approve-write"),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains("places an order")),
        "{body}"
    );
    let ultrafast = app.runs().await?;
    assert_eq!(ultrafast["runs"]["alwaysWithAgent"], "order", "{ultrafast}");
    assert_eq!(ultrafast["runs"]["mode"], "with_agent");
    let row = app.state.skills.flow_row(SKILL).await?;
    assert_eq!(row.map(|row| row.write_approved), Some(false));
    Ok(())
}

#[tokio::test]
async fn a_read_only_skill_has_nothing_to_choose() -> anyhow::Result<()> {
    let app = app(true, None).await?;
    write_skill(&app.state, "Add it to the cart.", ADD_TO_CART).await?;
    let mut read = write_flow("async (s) => { return 1 }");
    read.meta.effects = Effects::Read;
    read.steps[1].commit = false;
    flow::store::write(&app.state.skills.skill_dir(SKILL), &read)?;

    let ultrafast = app.runs().await?;
    assert!(ultrafast.get("runs").is_none(), "{ultrafast}");
    Ok(())
}

#[tokio::test]
async fn the_plan_turns_ultrafast_on_and_an_ended_plan_locks_running_without_the_agent()
-> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let mut app = app(false, Some(&gateway)).await?;
    write_skill(&app.state, "Add it to the cart.", ADD_TO_CART).await?;
    let mut changes = app.state.ultrafast.subscribe();

    assert!(
        !app.state.sync_ultrafast_with_plan().await,
        "no plan, nothing changes"
    );
    assert!(!app.state.ultrafast_enabled());

    let (status, started) = app
        .call(
            "POST",
            "/api/v1/plan/checkout",
            Some(json!({"plan": "ultrafast", "interval": "month"})),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{started}");
    let id = started["checkoutId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    gateway.complete_checkout(&id);
    let (_, complete) = app
        .call("GET", &format!("/api/v1/plan/checkout/{id}"), None)
        .await?;
    assert_eq!(complete["plan"]["features"], json!(["claw.ultrafast"]));

    assert!(app.state.sync_ultrafast_with_plan().await);
    assert!(app.state.ultrafast_enabled());
    assert!(
        changes.has_changed()?,
        "connected agents hear that their tool list changed"
    );
    changes.borrow_and_update();
    let body = app.state.skills.get(SKILL).await?.body;
    assert!(body.contains("## Run it fast"), "{body}");
    let (status, _) = app
        .call(
            "POST",
            &format!("/api/v1/skills/{SKILL}/ultrafast/approve-write"),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::OK);

    // The plan ends: this computer starts over with no licence.
    app.state.billing = Arc::new(
        Billing::open(
            Some(gateway.config()?),
            app.dir.path().join("fresh"),
            CancellationToken::new(),
        )
        .await,
    );
    app.rebuild();
    assert!(app.state.sync_ultrafast_with_plan().await);
    assert!(!app.state.ultrafast_enabled());
    assert!(changes.has_changed()?);

    let ultrafast = app.runs().await?;
    assert_eq!(ultrafast["locked"], true, "{ultrafast}");
    assert_eq!(ultrafast["runs"]["mode"], "with_agent");
    let body = app.state.skills.get(SKILL).await?.body;
    assert!(
        !body.contains("run_skill"),
        "with the plan ended the skill runs with the agent: {body}"
    );
    let (status, _) = app
        .call(
            "POST",
            &format!("/api/v1/skills/{SKILL}/ultrafast/approve-write"),
            None,
        )
        .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let row = app.state.skills.flow_row(SKILL).await?;
    assert!(
        row.as_ref().is_some_and(|row| row.write_approved),
        "the choice is kept for when the plan comes back"
    );
    Ok(())
}
