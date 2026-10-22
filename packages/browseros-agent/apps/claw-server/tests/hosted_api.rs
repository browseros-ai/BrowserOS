//! The hosted API (`/v1`) as a partner backend sees it: requests go through the full router
//! over seeded audit state, the same seam the published spec describes
//! (`contracts/hosted-api/openapi.yaml`).

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use base64::Engine as _;
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

/// Uploads rrweb events the way claw-app's recorder does, for one document in one tab.
async fn upload_recording(
    app: &TestApp,
    tab_id: i64,
    document_id: &str,
    events: &[Value],
) -> anyhow::Result<()> {
    let body = events
        .iter()
        .map(|event| format!("{event}\n"))
        .collect::<String>();
    let tab = tab_id.to_string();
    let batch = format!("batch-{document_id}");
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/recordings/events")
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .header("x-recording-tab-id", tab.as_str())
        .header("x-recording-document-id", document_id)
        .header("x-recording-batch-id", batch.as_str())
        .body(Body::from(body))?;
    let response = app.router.clone().oneshot(request).await?;
    anyhow::ensure!(
        response.status() == StatusCode::OK,
        "recording upload failed: {}",
        response.status()
    );
    Ok(())
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

fn dispatch_input(
    session: &str,
    tool: &str,
    at: i64,
    parent: Option<&str>,
) -> RecordToolDispatchInput {
    Call {
        tool,
        parent,
        ..Call::new(session, at)
    }
    .into_input()
}

impl Call<'_> {
    fn into_input(self) -> RecordToolDispatchInput {
        let slug = claw_server::identity::slugify_client_name(self.client)
            .unwrap_or_else(|| "agent".to_string());
        RecordToolDispatchInput {
            agent_id: format!("{slug}-task"),
            slug,
            agent_label: self.client.to_string(),
            session_id: self.session.to_string(),
            tool_name: self.tool.to_string(),
            page_id: Some(7),
            tab_id: Some(101),
            target_id: Some("target-7".to_string()),
            url: Some("https://news.ycombinator.com/".to_string()),
            title: Some("Hacker News".to_string()),
            args_json: bounded_args_json(&self.args),
            result_meta: result_meta(self.is_error, self.cancelled, &json!({}), 1),
            duration_ms: 40,
            dispatch_id: self.dispatch_id,
            created_at: Some(self.at),
            parent_dispatch_id: self.parent.map(DispatchId::from),
            tool_input_token_estimate: 1,
            tool_output_token_estimate: 1,
            token_estimator_version: 1,
        }
    }
}

async fn seed(app: &TestApp, call: Call<'_>) -> anyhow::Result<i64> {
    Ok(app
        .state
        .audit_log
        .record_tool_dispatch(call.into_input())
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

    for uri in [
        "/v1/sessions",
        "/v1/sessions/s1",
        "/v1/sessions/s1/recording",
    ] {
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

#[tokio::test]
async fn get_returns_tool_calls_in_call_order() -> anyhow::Result<()> {
    let app = hosted().await?;
    let start = now_ms() - 60_000;
    let script = DispatchId::new();
    let script_id = script.to_string();
    seed(&app, Call::new("s1", start)).await?;
    seed(
        &app,
        Call {
            tool: "run",
            args: json!({ "script": "await browser.click('#more')" }),
            dispatch_id: script,
            ..Call::new("s1", start + 1_000)
        },
    )
    .await?;
    seed(
        &app,
        Call {
            tool: "click",
            args: json!([7, "#more"]),
            parent: Some(&script_id),
            ..Call::new("s1", start + 1_100)
        },
    )
    .await?;
    seed(
        &app,
        Call {
            tool: "snapshot",
            is_error: true,
            ..Call::new("s1", start + 2_000)
        },
    )
    .await?;
    seed(
        &app,
        Call {
            tool: "wait",
            cancelled: true,
            ..Call::new("s1", start + 3_000)
        },
    )
    .await?;

    let (status, body) = get(&app.router, "/v1/sessions/s1").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], "s1");
    assert_eq!(body["status"], "done");
    assert_eq!(body["toolCallCount"], 5);
    let calls = body["toolCalls"].as_array().cloned().unwrap_or_default();
    let tools = calls
        .iter()
        .map(|call| call["tool"].clone())
        .collect::<Vec<_>>();
    assert_eq!(tools, ["navigate", "run", "click", "snapshot", "wait"]);

    let navigate = &calls[0];
    assert!(navigate["id"].as_str().is_some_and(|id| !id.is_empty()));
    assert_eq!(
        navigate["args"],
        json!({ "url": "https://news.ycombinator.com/" })
    );
    assert_eq!(navigate["outcome"], "ok");
    assert_eq!(navigate["startedAt"], start);
    assert_eq!(navigate["durationMs"], 40);
    assert_eq!(
        navigate["page"],
        json!({ "tabId": 101, "url": "https://news.ycombinator.com/", "title": "Hacker News" })
    );
    assert_eq!(navigate["hasScreenshot"], false);
    assert!(navigate.get("screenshot").is_none());
    assert!(navigate.get("parentId").is_none());

    assert_eq!(calls[1]["id"], script_id.as_str());
    assert_eq!(calls[2]["parentId"], script_id.as_str());
    assert_eq!(calls[2]["args"], json!([7, "#more"]));
    assert_eq!(calls[3]["outcome"], "error");
    assert_eq!(calls[4]["outcome"], "cancelled");
    Ok(())
}

#[tokio::test]
async fn get_flags_arguments_too_large_to_keep() -> anyhow::Result<()> {
    let app = hosted().await?;
    seed(
        &app,
        Call {
            args: json!({ "script": "x".repeat(10_000) }),
            ..Call::new("big", now_ms())
        },
    )
    .await?;

    let (_, body) = get(&app.router, "/v1/sessions/big").await?;
    let call = &body["toolCalls"][0];
    assert!(call.get("args").is_none());
    assert_eq!(call["argsTruncated"], true);
    Ok(())
}

#[tokio::test]
async fn get_inlines_screenshots_only_when_asked() -> anyhow::Result<()> {
    let app = hosted().await?;
    let row = seed(&app, Call::new("shot", now_ms() - 60_000)).await?;
    let jpeg = b"\xFF\xD8\xFFfixture-jpeg";
    app.state.screenshots.write("shot", row, jpeg).await?;
    app.state.audit_log.mark_screenshot(row).await?;

    let (_, body) = get(&app.router, "/v1/sessions/shot").await?;
    assert_eq!(body["toolCalls"][0]["hasScreenshot"], true);
    assert!(body["toolCalls"][0].get("screenshot").is_none());

    let (status, body) = get(&app.router, "/v1/sessions/shot?include=screenshots").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["toolCalls"][0]["screenshot"],
        json!({
            "contentType": "image/jpeg",
            "data": base64::engine::general_purpose::STANDARD.encode(jpeg),
        })
    );

    let (status, body) = get(&app.router, "/v1/sessions/shot?include=video").await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_request");
    Ok(())
}

#[tokio::test]
async fn get_unknown_session_is_not_found() -> anyhow::Result<()> {
    let app = hosted().await?;
    for uri in ["/v1/sessions/missing", "/v1/sessions/missing/recording"] {
        let (status, body) = get(&app.router, uri).await?;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(body["code"], "session_not_found", "{uri}");
    }
    Ok(())
}

#[tokio::test]
async fn get_reports_completion() -> anyhow::Result<()> {
    let app = hosted().await?;
    // Ended long ago: no end event, last activity a minute back, like a session the
    // server closed on restart.
    seed(&app, Call::new("old", now_ms() - 60_000)).await?;
    let (_, body) = get(&app.router, "/v1/sessions/old").await?;
    assert_eq!(body["complete"], true);
    assert!(body["endedAt"].is_number());

    app.state
        .sessions
        .insert_for_testing(live_session("open"))
        .await;
    seed(&app, Call::new("open", now_ms())).await?;
    let (_, body) = get(&app.router, "/v1/sessions/open").await?;
    assert_eq!(body["status"], "live");
    assert_eq!(body["complete"], false);
    assert!(body.get("endedAt").is_none());

    // Just ended: the recorder may still be uploading, so not complete yet.
    seed(&app, Call::new("ending", now_ms())).await?;
    app.state
        .audit_log
        .record_session_end("ending", "closed", None)
        .await?;
    let (_, body) = get(&app.router, "/v1/sessions/ending").await?;
    assert_eq!(body["status"], "done");
    assert_eq!(body["complete"], false);
    Ok(())
}

#[tokio::test]
async fn get_wait_holds_an_incomplete_session_until_the_timeout() -> anyhow::Result<()> {
    let app = hosted().await?;
    seed(&app, Call::new("ending", now_ms())).await?;
    app.state
        .audit_log
        .record_session_end("ending", "closed", None)
        .await?;
    seed(&app, Call::new("old", now_ms() - 60_000)).await?;

    let started = std::time::Instant::now();
    let (status, body) = get(&app.router, "/v1/sessions/ending?wait=1").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["complete"], false);
    assert!(started.elapsed() >= Duration::from_millis(900));

    let started = std::time::Instant::now();
    let (_, body) = get(&app.router, "/v1/sessions/old?wait=30").await?;
    assert_eq!(body["complete"], true);
    assert!(started.elapsed() < Duration::from_secs(5));

    for query in ["wait=61", "wait=-1", "wait=soon"] {
        let (status, _) = get(&app.router, &format!("/v1/sessions/old?{query}")).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
    }
    Ok(())
}

#[tokio::test]
async fn recording_groups_rrweb_events_per_page() -> anyhow::Result<()> {
    let app = hosted().await?;
    let session = live_session("rec");
    app.state.sessions.insert_for_testing(session.clone()).await;
    seed(&app, Call::new("rec", now_ms())).await?;
    for (tab_id, target_id) in [(101, "target-7"), (102, "target-8")] {
        app.state.session_tabs.enqueue_claim_tab_for_session(
            tab_id,
            Some(target_id.to_string()),
            "rec".to_string(),
            session.convo_id().as_str().to_string(),
            0,
        );
    }
    app.state.session_tabs.drain_writes().await;

    let (status, body) = get(&app.router, "/v1/sessions/rec/recording").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "pages": [] }));

    let first = "33D25F3CF060E81B14070BC356FF1871";
    let second = "8395FF2EF4A1D8579F1917B3B54ADECE";
    upload_recording(
        &app,
        101,
        first,
        &[
            json!({ "ts": 100, "type": 4, "data": { "href": "https://news.ycombinator.com/" } }),
            json!({ "ts": 300, "type": 3, "data": { "source": 0 } }),
            // Not replayable without a numeric rrweb type, so it is left out.
            json!({ "ts": 350, "data": { "orphan": true } }),
        ],
    )
    .await?;
    upload_recording(
        &app,
        102,
        second,
        &[json!({ "ts": 200, "type": 2, "data": { "node": {} } })],
    )
    .await?;

    let (status, body) = get(&app.router, "/v1/sessions/rec/recording").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "pages": [
                {
                    "tabId": 101,
                    "documentId": first,
                    "events": [
                        { "type": 4, "data": { "href": "https://news.ycombinator.com/" }, "timestamp": 100 },
                        { "type": 3, "data": { "source": 0 }, "timestamp": 300 },
                    ],
                },
                {
                    "tabId": 102,
                    "documentId": second,
                    "events": [{ "type": 2, "data": { "node": {} }, "timestamp": 200 }],
                },
            ],
        })
    );
    Ok(())
}

async fn record_start(app: &TestApp, session_id: &str, client_name: &str) -> anyhow::Result<()> {
    let slug = claw_server::identity::slugify_client_name(client_name)
        .unwrap_or_else(|| "agent".to_string());
    app.state
        .audit_log
        .record_session_start(
            session_id,
            &format!("{slug}-task"),
            &slug,
            client_name,
            client_name,
            "1.0.0",
        )
        .await?;
    Ok(())
}

#[tokio::test]
async fn a_session_still_tearing_down_is_not_complete() -> anyhow::Result<()> {
    let app = hosted().await?;
    // Out of the live map but without an end event: teardown is still draining calls.
    record_start(&app, "closing", "Acme Assistant").await?;
    seed(&app, Call::new("closing", now_ms() - 60_000)).await?;
    let (_, body) = get(&app.router, "/v1/sessions/closing").await?;
    assert_eq!(body["complete"], false);

    // A reused id: the first life's end must not finish the second life.
    app.state
        .audit_log
        .record_session_end("closing", "closed", None)
        .await?;
    record_start(&app, "closing", "Acme Assistant").await?;
    let (_, body) = get(&app.router, "/v1/sessions/closing").await?;
    assert_eq!(body["complete"], false);
    Ok(())
}

#[tokio::test]
async fn name_and_client_do_not_change_when_a_session_ends() -> anyhow::Result<()> {
    let app = hosted().await?;
    let session = live_session("named");
    app.state.sessions.insert_for_testing(session.clone()).await;
    session.rename("invoice-processing".to_string()).await;
    // The client sent clientInfo { name: "acme-wire", title: "Acme Assistant" }: rows carry
    // the name's slug for ownership and the title as their display label.
    record_start(&app, "named", "acme-wire").await?;
    app.state
        .audit_log
        .record_tool_dispatch(RecordToolDispatchInput {
            slug: "acme-wire".to_string(),
            ..Call {
                client: "Acme Assistant",
                ..Call::new("named", now_ms())
            }
            .into_input()
        })
        .await?;

    let (_, live) = get(&app.router, "/v1/sessions/named").await?;
    app.state
        .sessions
        .remove(session.id(), "closed", None)
        .await?;
    let (_, ended) = get(&app.router, "/v1/sessions/named").await?;
    assert_eq!(live["name"], "Browsed news.ycombinator.com");
    assert_eq!(ended["name"], live["name"]);
    assert_eq!(live["client"], "acme-wire");
    assert_eq!(ended["client"], "acme-wire");

    let (_, body) = get(&app.router, "/v1/sessions?client=acme-wire").await?;
    assert_eq!(ids(&body), ["named"]);
    assert_eq!(body["items"][0]["client"], "acme-wire");
    let (_, body) = get(&app.router, "/v1/sessions?q=Browsed%20news").await?;
    assert_eq!(ids(&body), ["named"]);
    Ok(())
}

#[tokio::test]
async fn concurrent_run_steps_are_ordered_by_start() -> anyhow::Result<()> {
    let app = hosted().await?;
    let start_at = now_ms() - 60_000;
    let script = DispatchId::new();
    let script_id = script.to_string();
    seed(
        &app,
        Call {
            tool: "run",
            dispatch_id: script,
            ..Call::new("parallel", start_at)
        },
    )
    .await?;
    // Steps are recorded when they finish; `seed` stamps 40 ms durations, so the slow step
    // that started first is recorded last.
    for (tool, finished_at) in [("read", start_at + 280), ("click", start_at + 300)] {
        seed(
            &app,
            Call {
                tool,
                parent: Some(&script_id),
                ..Call::new("parallel", finished_at)
            },
        )
        .await?;
    }
    let slow = app
        .state
        .audit_log
        .record_tool_dispatch(RecordToolDispatchInput {
            duration_ms: 250,
            ..dispatch_input("parallel", "screenshot", start_at + 310, Some(&script_id))
        })
        .await?;
    assert!(slow > 0);

    let (_, body) = get(&app.router, "/v1/sessions/parallel").await?;
    let calls = body["toolCalls"].as_array().cloned().unwrap_or_default();
    let order = calls
        .iter()
        .map(|call| (call["tool"].clone(), call["startedAt"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        [
            (json!("run"), json!(start_at)),
            (json!("screenshot"), json!(start_at + 60)),
            (json!("read"), json!(start_at + 240)),
            (json!("click"), json!(start_at + 260)),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn client_filter_and_search_use_the_reported_client_name() -> anyhow::Result<()> {
    let app = hosted().await?;
    // clientInfo { name: "acme-wire", title: "Acme Assistant" } with `agentName: finance`:
    // ownership (the slug) follows the declared agent, the label the title, and the start
    // event records the reported name.
    record_start(&app, "declared", "acme-wire").await?;
    app.state
        .audit_log
        .record_tool_dispatch(RecordToolDispatchInput {
            slug: "finance".to_string(),
            ..Call {
                client: "Acme Assistant",
                ..Call::new("declared", now_ms())
            }
            .into_input()
        })
        .await?;

    record_start(&app, "other", "Other Bot").await?;
    seed(
        &app,
        Call {
            client: "Other Bot",
            ..Call::new("other", now_ms())
        },
    )
    .await?;

    let (_, body) = get(&app.router, "/v1/sessions?client=acme-wire").await?;
    assert_eq!(ids(&body), ["declared"]);
    assert_eq!(body["items"][0]["client"], "acme-wire");
    let (_, body) = get(&app.router, "/v1/sessions?client=Other%20Bot").await?;
    assert_eq!(ids(&body), ["other"]);
    // `client` is the exact value a session reports, not a fuzzy match.
    for near_miss in ["finance", "Acme%20Assistant", "other-bot"] {
        let (_, body) = get(&app.router, &format!("/v1/sessions?client={near_miss}")).await?;
        assert_eq!(ids(&body), Vec::<String>::new(), "{near_miss}");
    }
    let (_, body) = get(&app.router, "/v1/sessions?q=acme-wire").await?;
    assert_eq!(ids(&body), ["declared"]);
    Ok(())
}
