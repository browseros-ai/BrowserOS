//! The lock routes and `lock.state`, as zuzu sees them. The app's side is called on the
//! `Service` directly: its lock policy (`locks().check`) and the browser link's tab events.

use crate::support::{BoxError, EventStream, Harness, Sse, TestResult, harness};
use axum::http::StatusCode;
use browseros_core::TabId;
use browseros_takeover::{LockBinding, LockId, LockTarget, LockTtl, Reads, TakeoverId};
use browseros_trace::{Effect, EpochMillis};
use serde_json::{Value, json};
use std::time::Duration;

/// Whether the app's lock policy lets the agent's `effect` on `tab` through.
fn agent_may(h: &Harness, tab: i64, effect: Effect) -> bool {
    h.service
        .locks()
        .check(LockTarget::Tab(TabId(tab)), effect)
        .is_ok()
}

async fn take_lock(h: &Harness, request: Value) -> Result<Value, BoxError> {
    let reply = h.post("/v1/locks", &request.to_string()).await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    Ok(reply.body)
}

fn lock_uri(lock: &Value) -> Result<String, BoxError> {
    Ok(format!(
        "/v1/locks/{}",
        lock["id"].as_str().ok_or("a lock without an id")?
    ))
}

#[tokio::test]
async fn a_lock_refuses_the_agent_on_its_tab_until_the_host_releases_it() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(3);
    h.browser.open_tab(4);
    let lock = take_lock(&h, json!({"tabId": 3, "ttlSeconds": 120})).await?;
    let uri = lock_uri(&lock)?;
    assert!(uri.starts_with("/v1/locks/lk_"), "{uri}");
    assert_eq!(lock["tabId"], 3);
    assert_eq!(lock["reads"], "block", "the default");
    assert_eq!(lock["settled"], true, "nothing was in flight");
    let created = lock["createdAt"].as_i64().ok_or("no createdAt")?;
    assert_eq!(lock["expiresAt"].as_i64(), Some(created + 120_000));
    assert!(lock.get("releasedAt").is_none());
    assert!(!agent_may(&h, 3, Effect::Write));
    assert!(!agent_may(&h, 3, Effect::Read));
    assert!(agent_may(&h, 4, Effect::Write), "another tab");

    let read = h.get(&uri).await?;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body, lock);

    let released = h.delete(&uri).await?;
    assert_eq!(released.status, StatusCode::OK, "{}", released.body);
    assert!(released.body["releasedAt"].as_i64() >= Some(created));
    assert_eq!(released.body["id"], lock["id"]);
    assert!(agent_may(&h, 3, Effect::Write));
    let again = h.delete(&uri).await?;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.body, released.body, "releasing twice changes nothing");
    assert_eq!(h.get(&uri).await?.body, released.body);
    Ok(())
}

#[tokio::test]
async fn reads_allow_refuses_only_the_agents_writes() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(3);
    let lock = take_lock(&h, json!({"tabId": 3, "reads": "allow", "ttlSeconds": 60})).await?;
    assert_eq!(lock["reads"], "allow");
    assert!(!agent_may(&h, 3, Effect::Write));
    assert!(agent_may(&h, 3, Effect::Read));
    Ok(())
}

#[tokio::test]
async fn a_bad_lock_request_is_a_400_and_locks_nothing() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(3);
    for body in [
        r#"{"tabId":3}"#,
        r#"{"ttlSeconds":60}"#,
        r#"{"tabId":3,"ttlSeconds":0}"#,
        r#"{"tabId":3,"ttlSeconds":1201}"#,
        r#"{"tabId":3,"ttlSeconds":-5}"#,
        r#"{"tabId":3,"ttlSeconds":"60"}"#,
        r#"{"tabId":3,"ttlSeconds":1.5}"#,
        r#"{"tabId":"3","ttlSeconds":60}"#,
        r#"{"tabId":3,"ttlSeconds":60,"reads":"none"}"#,
        "not json",
        "",
    ] {
        h.post("/v1/locks", body)
            .await?
            .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
        assert!(agent_may(&h, 3, Effect::Write), "{body} locked the tab");
    }
    for ttl in [1, 1200] {
        let lock = take_lock(&h, json!({"tabId": 3, "ttlSeconds": ttl})).await?;
        assert_eq!(
            h.delete(&lock_uri(&lock)?).await?.status,
            StatusCode::OK,
            "{ttl} s is in bounds"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_tab_the_browser_does_not_have_is_a_404() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(3);
    h.post("/v1/locks", r#"{"tabId":99,"ttlSeconds":60}"#)
        .await?
        .assert_error(StatusCode::NOT_FOUND, "tab_not_found");
    assert!(agent_may(&h, 99, Effect::Write), "no lock was taken");
    h.post("/v1/tabs/999999/private", "")
        .await?
        .assert_error(StatusCode::NOT_FOUND, "tab_not_found");
    assert!(h.privacy.is_private(TabId(999_999)).is_none());
    Ok(())
}

#[tokio::test]
async fn an_id_that_names_no_lock_is_a_404() -> TestResult {
    let h = harness().await?;
    for uri in [
        format!("/v1/locks/{}", LockId::new()),
        format!("/v1/locks/{}", TakeoverId::new()),
        "/v1/locks/nope".to_string(),
    ] {
        h.get(&uri)
            .await?
            .assert_error(StatusCode::NOT_FOUND, "not_found");
        h.delete(&uri)
            .await?
            .assert_error(StatusCode::NOT_FOUND, "not_found");
    }
    Ok(())
}

#[tokio::test]
async fn a_takeovers_lock_is_never_released_through_the_lock_route() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(3);
    let ttl = LockTtl::new(Duration::from_secs(60)).ok_or("ttl")?;
    let lock = h
        .service
        .locks()
        .lock(
            TabId(3),
            Reads::Block,
            ttl,
            LockBinding::Takeover(TakeoverId::new()),
        )
        .await;
    let uri = format!("/v1/locks/{}", lock.id);
    h.delete(&uri)
        .await?
        .assert_error(StatusCode::CONFLICT, "takeover_lock");
    assert!(
        !agent_may(&h, 3, Effect::Write),
        "the owner still has the tab"
    );
    let read = h.get(&uri).await?;
    assert_eq!(read.status, StatusCode::OK);
    assert!(read.body.get("releasedAt").is_none());
    Ok(())
}

#[tokio::test]
async fn lock_state_reaches_the_tabs_event_stream_when_taken_and_when_its_ttl_ends() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(3);
    h.browser.open_tab(4);
    let mut tab3 = EventStream::open(&h, "/v1/events?tab=3", None).await?;
    let mut tab4 = EventStream::open(&h, "/v1/events?tab=4", None).await?;
    let lock = take_lock(&h, json!({"tabId": 3, "ttlSeconds": 1})).await?;

    let taken = tab3.next().await?;
    assert_eq!(taken.kind, "lock.state");
    assert_eq!(taken.data["type"], "lock.state");
    assert_eq!(taken.data["tabId"], 3);
    assert_eq!(taken.data["at"], lock["createdAt"]);
    assert_eq!(taken.data["lock"], lock);

    let expired = tab3.next().await?;
    assert_eq!(expired.kind, "lock.state");
    assert_eq!(expired.data["lock"]["id"], lock["id"]);
    assert_eq!(
        expired.data["lock"]["releasedAt"], lock["expiresAt"],
        "it ended at its TTL"
    );
    assert_eq!(expired.data["at"], lock["expiresAt"]);
    assert_eq!(h.get(&lock_uri(&lock)?).await?.body, expired.data["lock"]);
    assert!(agent_may(&h, 3, Effect::Write));

    assert_eq!(
        h.post("/v1/tabs/4/private", "").await?.status,
        StatusCode::OK
    );
    assert_eq!(
        tab4.next().await?.kind,
        "tab.private",
        "the tab-4 stream saw neither of tab 3's lock events"
    );
    Ok(())
}

#[tokio::test]
async fn a_closed_tab_loses_its_locks_and_flag_then_says_so() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(3);
    let mut stream = EventStream::open(&h, "/v1/events?tab=3", None).await?;
    assert_eq!(
        h.post("/v1/tabs/3/private", "").await?.status,
        StatusCode::OK
    );
    assert_eq!(stream.next().await?.kind, "tab.private");
    let lock = take_lock(&h, json!({"tabId": 3, "ttlSeconds": 60})).await?;
    assert_eq!(stream.next().await?.kind, "lock.state");

    h.browser.close_tab(3);
    h.service.tab_closed(TabId(3)).await;
    assert!(agent_may(&h, 3, Effect::Write));
    assert!(
        h.privacy.is_private(TabId(3)).is_none(),
        "its flag ended with it"
    );
    let read = h.get(&lock_uri(&lock)?).await?;
    assert!(read.body["releasedAt"].is_i64(), "{}", read.body);

    // The lock's end and `tab.closed` come from two sources, so either may be numbered first.
    let mut ends = stream.take(2).await?;
    ends.sort_by(|a, b| a.kind.cmp(&b.kind));
    let kinds = ends.iter().map(|end| end.kind.as_str()).collect::<Vec<_>>();
    assert_eq!(kinds, ["lock.state", "tab.closed"]);
    let [ended, closed]: [Sse; 2] = ends.try_into().map_err(|_| "two events")?;
    assert_eq!(ended.data["lock"], read.body);
    assert_eq!(closed.data["tabId"], 3);
    assert_eq!(closed.data["type"], "tab.closed");

    h.post("/v1/locks", r#"{"tabId":3,"ttlSeconds":60}"#)
        .await?
        .assert_error(StatusCode::NOT_FOUND, "tab_not_found");
    Ok(())
}

#[tokio::test]
async fn a_reconnected_browser_ends_the_locks_of_tabs_it_no_longer_has() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(3);
    h.browser.open_tab(5);
    let gone = take_lock(&h, json!({"tabId": 3, "ttlSeconds": 60})).await?;
    let kept = take_lock(&h, json!({"tabId": 5, "ttlSeconds": 60})).await?;
    h.service.retain_tabs(&[TabId(5)]).await;
    assert!(agent_may(&h, 3, Effect::Write));
    assert!(!agent_may(&h, 5, Effect::Write));
    assert!(h.get(&lock_uri(&gone)?).await?.body["releasedAt"].is_i64());
    assert!(
        h.get(&lock_uri(&kept)?)
            .await?
            .body
            .get("releasedAt")
            .is_none()
    );
    Ok(())
}

#[tokio::test]
async fn a_browser_that_does_not_answer_is_a_503() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(3);
    h.browser.become_unreachable();
    h.post("/v1/locks", r#"{"tabId":3,"ttlSeconds":60}"#)
        .await?
        .assert_error(StatusCode::SERVICE_UNAVAILABLE, "browser_unavailable");
    assert!(agent_may(&h, 3, Effect::Write));
    h.post("/v1/tabs/3/private", "")
        .await?
        .assert_error(StatusCode::SERVICE_UNAVAILABLE, "browser_unavailable");
    assert!(h.privacy.is_private(TabId(3)).is_none());
    Ok(())
}

#[tokio::test]
async fn a_tab_that_opens_during_control_starts_private() -> TestResult {
    let h = harness().await?;
    let display = h.privacy.control_display_opened();
    h.service.tab_opened(TabId(9), EpochMillis::now()).await?;
    assert!(h.privacy.is_private(TabId(9)).is_some());
    drop(display);
    h.service.tab_opened(TabId(10), EpochMillis::now()).await?;
    assert!(h.privacy.is_private(TabId(10)).is_none());
    Ok(())
}
