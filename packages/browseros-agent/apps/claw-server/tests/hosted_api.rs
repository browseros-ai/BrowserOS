//! The hosted API (`/v1`) as a partner backend sees it: requests go through the full router
//! over seeded audit state, the same seam the published spec describes
//! (`contracts/hosted-api/openapi.yaml`).

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use claw_server::{
    AppState, build_router,
    config::{Config, ServerMode},
    db::audit_log::{RecordToolDispatchInput, bounded_args_json, result_meta},
    identity::{ClientIdentity, ConversationIdentity},
    ids::{DispatchId, SessionId},
    services::sessions::Session,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tempfile::TempDir;
use tower::ServiceExt;

struct TestApp {
    router: Router,
    state: AppState,
    _dir: TempDir,
}

async fn app(mode: ServerMode) -> anyhow::Result<TestApp> {
    let dir = tempfile::tempdir()?;
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49337,
        proxy_port: None,
        resources_dir: dir.path().join("resources"),
        browserclaw_dir: dir.path().join("browserclaw"),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
        native: Default::default(),
        mode,
    });
    let state = AppState::new_with_home(config, dir.path().join("home")).await?;
    Ok(TestApp {
        router: build_router(state.clone()),
        state,
        _dir: dir,
    })
}

async fn hosted() -> anyhow::Result<TestApp> {
    app(ServerMode::Hosted).await
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
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)?
    };
    Ok((status, body))
}

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

/// One recorded tool call. Defaults describe a successful `navigate` by "Acme Assistant".
struct Call<'a> {
    session: &'a str,
    tool: &'a str,
    client: &'a str,
    at: i64,
    args: Value,
    is_error: bool,
    cancelled: bool,
    parent: Option<&'a str>,
    dispatch_id: DispatchId,
}

impl<'a> Call<'a> {
    fn new(session: &'a str, at: i64) -> Self {
        Self {
            session,
            tool: "navigate",
            client: "Acme Assistant",
            at,
            args: json!({ "url": "https://news.ycombinator.com/" }),
            is_error: false,
            cancelled: false,
            parent: None,
            dispatch_id: DispatchId::new(),
        }
    }
}

async fn seed(app: &TestApp, call: Call<'_>) -> anyhow::Result<i64> {
    let slug = claw_server::identity::slugify_client_name(call.client)
        .unwrap_or_else(|| "agent".to_string());
    Ok(app
        .state
        .audit_log
        .record_tool_dispatch(RecordToolDispatchInput {
            agent_id: format!("{slug}-task"),
            slug,
            agent_label: call.client.to_string(),
            session_id: call.session.to_string(),
            tool_name: call.tool.to_string(),
            page_id: Some(7),
            tab_id: Some(101),
            target_id: Some("target-7".to_string()),
            url: Some("https://news.ycombinator.com/".to_string()),
            title: Some("Hacker News".to_string()),
            args_json: bounded_args_json(&call.args),
            result_meta: result_meta(call.is_error, call.cancelled, &json!({}), 1),
            duration_ms: 40,
            dispatch_id: call.dispatch_id,
            created_at: Some(call.at),
            parent_dispatch_id: call.parent.map(DispatchId::from),
            tool_input_token_estimate: 1,
            tool_output_token_estimate: 1,
            token_estimator_version: 1,
        })
        .await?)
}

fn live_session(session_id: &str) -> Arc<Session> {
    Session::new(
        SessionId::new(session_id),
        ClientIdentity::Ephemeral {
            slug: "acme-assistant".to_string(),
            label: "Acme Assistant".to_string(),
        },
        ConversationIdentity::new("acme-assistant", "read-hn".to_string()),
        "Acme Assistant".to_string(),
        tokio::time::Instant::now(),
    )
}

fn ids(body: &Value) -> Vec<String> {
    body["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["id"].as_str().map(str::to_string))
        .collect()
}

#[tokio::test]
async fn v1_is_not_served_outside_hosted_mode() -> anyhow::Result<()> {
    let app = app(ServerMode::Desktop).await?;
    seed(&app, Call::new("s1", now_ms())).await?;

    for uri in ["/v1/sessions", "/v1/sessions/s1", "/v1/sessions/s1/recording"] {
        let (status, _) = get(&app.router, uri).await?;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
    Ok(())
}

#[tokio::test]
async fn list_returns_session_summaries_most_recently_active_first() -> anyhow::Result<()> {
    let app = hosted().await?;
    let start = now_ms() - 60_000;
    seed(&app, Call::new("older", start)).await?;
    seed(&app, Call::new("newer", start + 1_000)).await?;
    let (_, body) = get(&app.router, "/v1/sessions").await?;
    assert_eq!(ids(&body), ["newer", "older"]);

    seed(&app, Call::new("older", start + 2_000)).await?;
    app.state
        .audit_log
        .set_task_summary("older", "Read the top Hacker News posts")
        .await?;

    let (status, body) = get(&app.router, "/v1/sessions").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&body), ["older", "newer"]);
    assert!(body.get("nextCursor").is_none());
    let older = &body["items"][0];
    assert_eq!(older["client"], "Acme Assistant");
    assert_eq!(older["status"], "done");
    assert_eq!(older["summary"], "Read the top Hacker News posts");
    assert_eq!(older["startedAt"], start);
    assert_eq!(older["toolCallCount"], 2);
    assert!(older["name"].is_string());
    Ok(())
}

#[tokio::test]
async fn list_reports_an_open_session_as_live() -> anyhow::Result<()> {
    let app = hosted().await?;
    app.state
        .sessions
        .insert_for_testing(live_session("open"))
        .await;
    seed(&app, Call::new("open", now_ms())).await?;

    let (_, body) = get(&app.router, "/v1/sessions").await?;
    assert_eq!(body["items"][0]["status"], "live");
    assert!(body["items"][0].get("endedAt").is_none());
    Ok(())
}

#[tokio::test]
async fn list_filters_narrow_the_results() -> anyhow::Result<()> {
    let app = hosted().await?;
    let start = now_ms() - 60_000;
    seed(&app, Call::new("acme-early", start)).await?;
    seed(&app, Call::new("acme-late", start + 10_000)).await?;
    seed(
        &app,
        Call {
            client: "Other Bot",
            ..Call::new("other", start + 5_000)
        },
    )
    .await?;
    app.state
        .audit_log
        .set_task_summary("acme-late", "Search Google News for US politics")
        .await?;

    let (_, body) = get(&app.router, "/v1/sessions?client=acme-assistant").await?;
    assert_eq!(ids(&body), ["acme-late", "acme-early"]);
    let (_, body) = get(&app.router, "/v1/sessions?client=Other%20Bot").await?;
    assert_eq!(ids(&body), ["other"]);

    let since = start + 5_000;
    let (_, body) = get(&app.router, &format!("/v1/sessions?since={since}")).await?;
    assert_eq!(ids(&body), ["other", "acme-late"]);
    let (_, body) = get(&app.router, &format!("/v1/sessions?until={since}")).await?;
    assert_eq!(ids(&body), ["other", "acme-early"]);

    let (_, body) = get(&app.router, "/v1/sessions?q=politics").await?;
    assert_eq!(ids(&body), ["acme-late"]);
    Ok(())
}

#[tokio::test]
async fn list_cursor_pages_without_gaps_or_repeats() -> anyhow::Result<()> {
    let app = hosted().await?;
    let start = now_ms() - 60_000;
    for index in 0..5 {
        seed(&app, Call::new(&format!("s{index}"), start + index)).await?;
    }

    let mut seen = Vec::new();
    let mut uri = "/v1/sessions?limit=2".to_string();
    loop {
        let (status, body) = get(&app.router, &uri).await?;
        assert_eq!(status, StatusCode::OK);
        let page = ids(&body);
        assert!(page.len() <= 2);
        seen.extend(page);
        match body["nextCursor"].as_str() {
            Some(cursor) => uri = format!("/v1/sessions?limit=2&cursor={cursor}"),
            None => break,
        }
    }
    assert_eq!(seen, ["s4", "s3", "s2", "s1", "s0"]);
    Ok(())
}

#[tokio::test]
async fn list_rejects_invalid_parameters() -> anyhow::Result<()> {
    let app = hosted().await?;
    for query in [
        "limit=0",
        "limit=101",
        "limit=ten",
        "since=-1",
        "until=soon",
        "cursor=not-a-cursor",
    ] {
        let (status, body) = get(&app.router, &format!("/v1/sessions?{query}")).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(body["code"], "invalid_request", "{query}");
        assert!(body["message"].is_string());
        assert!(body["requestId"].is_string());
    }
    Ok(())
}
