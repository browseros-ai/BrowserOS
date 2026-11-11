//! The v1 takeover flow's routes as zuzu calls them: the contract's `Takeover`, every refusal,
//! `wait=`, `takeover.state` on `/v1/events`, the takeover's lock refused on the lock route,
//! and a tab that closes. The flow's order and budget are the takeover crate's own tests'; the
//! display's `release=` binding is in `display.rs`.

use crate::support::{BoxError, EventStream, Harness, Sse, TestResult, harness};
use axum::http::StatusCode;
use browseros_core::TabId;
use neo_takeover::{LockTarget, OwnerHasScreen, ScreenHolder, TakeoverId};
use neo_trace::{Effect, PrivateReason};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

async fn start(h: &Harness, request: Value) -> Result<Value, BoxError> {
    let reply = h.post("/v1/takeovers", &request.to_string()).await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    Ok(reply.body)
}

fn id(takeover: &Value) -> Result<&str, BoxError> {
    Ok(takeover["id"].as_str().ok_or("a takeover without an id")?)
}

/// The next `takeover.state` event, skipping the others.
async fn next_takeover_state(stream: &mut EventStream) -> Result<Sse, BoxError> {
    loop {
        let event = stream.next().await?;
        if event.kind == "takeover.state" {
            return Ok(event);
        }
    }
}

#[tokio::test]
async fn a_takeover_starts_and_is_released_through_the_routes() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(12);
    let takeover = start(
        &h,
        json!({"tabId": 12, "viewer": {"width": 390, "height": 844}, "ttlSeconds": 600}),
    )
    .await?;
    let id = id(&takeover)?;
    assert!(id.starts_with("tk_"), "{id}");
    assert_eq!(takeover["tabId"], 12);
    assert_eq!(takeover["state"], "active");
    assert_eq!(
        takeover["display"],
        format!("/v1/display?mode=control&release={id}")
    );
    let created = takeover["createdAt"].as_i64().ok_or("no createdAt")?;
    assert_eq!(takeover["expiresAt"].as_i64(), Some(created + 600_000));
    assert!(takeover.get("releaseReason").is_none());
    assert!(takeover.get("releasedAt").is_none());
    let lock = format!(
        "/v1/locks/{}",
        takeover["lockId"].as_str().ok_or("no lockId")?
    );
    let fit = format!("/v1/fits/{}", takeover["fitId"].as_str().ok_or("no fitId")?);

    assert_eq!(
        h.privacy.is_private(TabId(12)).map(|record| record.reason),
        Some(PrivateReason::Takeover)
    );
    assert!(
        h.service
            .locks()
            .check(LockTarget::Tab(TabId(12)), Effect::Read)
            .is_err(),
        "the agent is locked out, reads too"
    );
    h.delete(&lock)
        .await?
        .assert_error(StatusCode::CONFLICT, "takeover_lock");
    assert_eq!(h.get(&fit).await?.body["screen"]["width"], 390);
    let again = start(&h, json!({"tabId": 12})).await?;
    assert_eq!(
        again, takeover,
        "a second start on the tab returns the first"
    );
    assert_eq!(h.get(&format!("/v1/takeovers/{id}")).await?.body, takeover);

    let release = format!("/v1/takeovers/{id}/release");
    let released = h.post(&release, "").await?;
    assert_eq!(released.status, StatusCode::OK, "{}", released.body);
    assert_eq!(released.body["state"], "released");
    assert_eq!(released.body["releaseReason"], "done", "the default");
    assert!(released.body["releasedAt"].as_i64() >= Some(created));
    assert!(
        h.service
            .locks()
            .check(LockTarget::Tab(TabId(12)), Effect::Write)
            .is_ok()
    );
    assert!(h.get(&lock).await?.body["releasedAt"].is_i64());
    assert!(h.get(&fit).await?.body["restoredAt"].is_i64());
    let twice = h.post(&release, r#"{"reason": "idle"}"#).await?;
    assert_eq!(twice.status, StatusCode::OK);
    assert_eq!(
        twice.body, released.body,
        "the first release's answer stands"
    );
    Ok(())
}

#[tokio::test]
async fn takeover_requests_that_do_not_parse_or_name_nothing_are_refused() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(12);
    for body in [
        "",
        "not json",
        r#"{"viewer": {"width": 390, "height": 844}}"#,
        r#"{"tabId": 12, "ttlSeconds": 59}"#,
        r#"{"tabId": 12, "ttlSeconds": 1201}"#,
        r#"{"tabId": 12, "reads": "sometimes"}"#,
        r#"{"tabId": 12, "viewer": {"width": 99, "height": 844}}"#,
    ] {
        h.post("/v1/takeovers", body)
            .await?
            .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    h.post("/v1/takeovers", r#"{"tabId": 99}"#)
        .await?
        .assert_error(StatusCode::NOT_FOUND, "tab_not_found");
    assert!(
        h.service
            .locks()
            .check(LockTarget::Tab(TabId(12)), Effect::Write)
            .is_ok(),
        "no refused start locked anything"
    );

    let unknown = TakeoverId::new();
    for uri in [
        "/v1/takeovers/nope".to_string(),
        format!("/v1/takeovers/{unknown}"),
    ] {
        h.get(&uri)
            .await?
            .assert_error(StatusCode::NOT_FOUND, "not_found");
    }
    h.post(&format!("/v1/takeovers/{unknown}/release"), "")
        .await?
        .assert_error(StatusCode::NOT_FOUND, "not_found");

    let takeover = start(&h, json!({"tabId": 12})).await?;
    let id = id(&takeover)?;
    for query in ["wait=61", "state=gone", "wait=1&wait=2"] {
        h.get(&format!("/v1/takeovers/{id}?{query}"))
            .await?
            .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    for body in [
        "{",
        r#"{"reason": "expired"}"#,
        r#"{"reason": "tab_closed"}"#,
    ] {
        h.post(&format!("/v1/takeovers/{id}/release"), body)
            .await?
            .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    assert_eq!(
        h.get(&format!("/v1/takeovers/{id}")).await?.body["state"],
        "active",
        "a refused release releases nothing"
    );
    Ok(())
}

#[tokio::test]
async fn wait_holds_the_request_until_the_takeover_is_released() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(12);
    let takeover = start(&h, json!({"tabId": 12})).await?;
    let id = id(&takeover)?.to_owned();

    let asked = Instant::now();
    let seen = h
        .get(&format!("/v1/takeovers/{id}?wait=30&state=released"))
        .await?;
    assert_eq!(seen.body["state"], "active");
    assert!(
        asked.elapsed() < Duration::from_secs(1),
        "an active takeover already differs from `released`"
    );

    let router = h.router.clone();
    let waiting = tokio::spawn({
        let uri = format!("/v1/takeovers/{id}?wait=30&state=active");
        async move {
            let request = axum::http::Request::builder()
                .uri(uri)
                .body(axum::body::Body::empty())?;
            let response = tower::ServiceExt::oneshot(router, request).await?;
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
            Ok::<Value, BoxError>(serde_json::from_slice(&bytes)?)
        }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!waiting.is_finished(), "the wait holds while it is active");
    let released = h
        .post(
            &format!("/v1/takeovers/{id}/release"),
            r#"{"reason": "host"}"#,
        )
        .await?;
    let woke = tokio::time::timeout(Duration::from_secs(5), waiting).await???;
    assert_eq!(woke, released.body);
    assert_eq!(woke["releaseReason"], "host");
    Ok(())
}

#[tokio::test]
async fn takeover_state_events_reach_the_tabs_event_stream() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(12);
    let mut stream = EventStream::open(&h, "/v1/events?tab=12", None).await?;
    let takeover = start(&h, json!({"tabId": 12})).await?;
    let id = id(&takeover)?;
    let released = h.post(&format!("/v1/takeovers/{id}/release"), "").await?;

    let active = next_takeover_state(&mut stream).await?;
    assert_eq!(active.data["tabId"], 12);
    assert_eq!(active.data["takeover"], takeover);
    assert_eq!(active.data["at"], takeover["createdAt"]);
    let ended = next_takeover_state(&mut stream).await?;
    assert_eq!(ended.data["takeover"], released.body);
    assert_eq!(ended.data["at"], released.body["releasedAt"]);
    Ok(())
}

#[tokio::test]
async fn a_closed_tab_releases_its_takeover_before_tab_closed() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(12);
    let mut stream = EventStream::open(&h, "/v1/events?tab=12", None).await?;
    let takeover = start(&h, json!({"tabId": 12})).await?;
    let id = id(&takeover)?;
    h.browser.close_tab(12);
    h.service.tab_closed(TabId(12)).await;

    let read = h.get(&format!("/v1/takeovers/{id}")).await?;
    assert_eq!(read.body["state"], "released");
    assert_eq!(read.body["releaseReason"], "tab_closed");
    let mut kinds = Vec::new();
    while kinds.last().map(String::as_str) != Some("tab.closed") {
        kinds.push(stream.next().await?.kind);
    }
    let released_at = kinds.iter().rposition(|kind| kind == "takeover.state");
    assert!(
        released_at.is_some_and(|at| at + 1 < kinds.len()),
        "the release is published before tab.closed: {kinds:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_takeover_holds_the_screen_until_its_release() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(12);
    let front = h.service.front();
    let takeover = start(&h, json!({"tabId": 12})).await?;
    assert_eq!(
        front.check(),
        Err(OwnerHasScreen {
            by: ScreenHolder::Takeover
        }),
        "a takeover with no viewer holds the screen too"
    );
    let released = h
        .post(&format!("/v1/takeovers/{}/release", id(&takeover)?), "")
        .await?;
    assert_eq!(released.status, StatusCode::OK, "{}", released.body);
    assert_eq!(front.check(), Ok(()), "the release gave it back");
    Ok(())
}
