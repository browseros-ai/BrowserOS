//! Host keeps through authenticated HTTP and the service's existing idle-janitor seam.

use crate::support::{self, BoxError, EventStream, Harness, Reply, TestResult};
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use browseros_core::TabId;
use browseros_core::viewport::Viewports;
use neo_http::{Service, ServiceInputs};
use neo_screencast::testing::ScriptedTransport;
use neo_takeover::{LockId, LockTarget, TabHold};
use neo_trace::{Effect, TraceStore, screenshot_queue};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;
use tower::ServiceExt;

async fn put(h: &Harness, tab: i64, body: &str) -> Result<Reply, BoxError> {
    h.call(
        Method::PUT,
        &format!("/v1/tabs/{tab}/keep"),
        Body::from(body.to_owned()),
    )
    .await
}

async fn listed_keep(h: &Harness, tab: i64) -> Result<Option<Value>, BoxError> {
    let reply = h.get("/v1/tabs").await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let items = reply.body["items"].as_array().ok_or("items")?;
    Ok(items
        .iter()
        .find(|item| item["tabId"] == tab)
        .and_then(|item| item.get("keep").cloned()))
}

#[tokio::test]
async fn optional_bodies_and_bounded_ttls_are_listed_and_replace_one_another() -> TestResult {
    let h = support::harness().await?;
    h.browser.open_tab(3);
    assert_eq!(listed_keep(&h, 3).await?, None);
    for body in ["", "{}"] {
        let reply = put(&h, 3, body).await?;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(reply.body, json!({"tabId":3,"keep":{}}));
        assert_eq!(listed_keep(&h, 3).await?, Some(json!({})));
        let _: neo_sdk_api::TabKeepResult = serde_json::from_value(reply.body)?;
    }
    for seconds in [1, 2_592_000] {
        let before = neo_trace::EpochMillis::now().millis();
        let reply = put(&h, 3, &json!({"ttlSeconds":seconds}).to_string()).await?;
        let after = neo_trace::EpochMillis::now().millis();
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        let expiry = reply.body["keep"]["expiresAt"].as_i64().ok_or("expiry")?;
        assert!((before + seconds * 1000..=after + seconds * 1000).contains(&expiry));
        assert_eq!(listed_keep(&h, 3).await?, Some(reply.body["keep"].clone()));
    }
    assert_eq!(put(&h, 3, "{}").await?.body["keep"], json!({}));
    let settings = h
        .call(Method::PUT, "/v1/settings", Body::from("{}"))
        .await?;
    assert_eq!(settings.status, StatusCode::OK);
    assert_eq!(listed_keep(&h, 3).await?, Some(json!({})));
    Ok(())
}

#[tokio::test]
async fn malformed_bodies_and_tab_ids_keep_nothing() -> TestResult {
    let h = support::harness().await?;
    h.browser.open_tab(3);
    for body in [
        "not json",
        " ",
        "null",
        "true",
        "[]",
        "1",
        "\"ttl\"",
        r#"{"ttlSeconds":null}"#,
        r#"{"ttlSeconds":true}"#,
        r#"{"ttlSeconds":"60"}"#,
        r#"{"ttlSeconds":1.0}"#,
        r#"{"ttlSeconds":1.5}"#,
        r#"{"ttlSeconds":0}"#,
        r#"{"ttlSeconds":-1}"#,
        r#"{"ttlSeconds":2592001}"#,
        r#"{"ttlSeconds":9223372036854775808}"#,
        r#"{"ttlSeconds":{}}"#,
        r#"{"other":1}"#,
        r#"{"ttlSeconds":60,"other":1}"#,
    ] {
        put(&h, 3, body)
            .await?
            .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
        assert_eq!(h.service.tab_hold(TabId(3)), None, "{body}");
    }
    for tab in ["nope", "1.5", "9223372036854775808"] {
        for method in [Method::PUT, Method::DELETE] {
            h.call(method, &format!("/v1/tabs/{tab}/keep"), Body::empty())
                .await?
                .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
        }
    }
    Ok(())
}

#[tokio::test]
async fn invalid_put_does_not_replace_an_existing_keep() -> TestResult {
    let h = support::harness().await?;
    h.browser.open_tab(3);
    put(&h, 3, "{}").await?;
    put(&h, 3, r#"{"ttlSeconds":null}"#)
        .await?
        .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    assert_eq!(listed_keep(&h, 3).await?, Some(json!({})));
    Ok(())
}

#[tokio::test]
async fn put_requires_a_tab_the_lock_inventory_can_name_including_neos_blank_tab() -> TestResult {
    let h = support::harness().await?;
    h.browser.open_tab(3);
    *h.neo_tab.lock().map_err(|_| "poisoned")? = Some(3);
    assert_eq!(put(&h, 3, "{}").await?.status, StatusCode::OK);
    h.browser.open_tab(77);
    h.browser.set_url(77, "chrome-extension://test/panel.html");
    for tab in [77, 99] {
        put(&h, tab, "{}")
            .await?
            .assert_error(StatusCode::NOT_FOUND, "tab_not_found");
        assert_eq!(h.service.tab_hold(TabId(tab)), None);
    }
    h.browser.open_tab(5);
    h.browser.become_unreachable();
    put(&h, 5, "{}")
        .await?
        .assert_error(StatusCode::SERVICE_UNAVAILABLE, "browser_unavailable");
    assert_eq!(h.service.tab_hold(TabId(5)), None);
    assert_eq!(h.service.tab_hold(TabId(3)), Some(TabHold::Now));
    Ok(())
}

#[tokio::test]
async fn delete_is_idempotent_and_never_needs_the_tab_or_browser() -> TestResult {
    let h = support::harness().await?;
    h.browser.open_tab(3);
    assert_eq!(put(&h, 3, "{}").await?.status, StatusCode::OK);
    let before = Instant::now();
    let reply = h.delete("/v1/tabs/3/keep").await?;
    let after = Instant::now();
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, json!({"tabId":3}));
    assert_eq!(listed_keep(&h, 3).await?, None);
    let hold = h.service.tab_hold(TabId(3));
    assert!(matches!(hold, Some(TabHold::Until(at)) if (before..=after).contains(&at)));
    h.browser.close_tab(3);
    h.browser.become_unreachable();
    for tab in [3, 3, 99, 0, -1] {
        let reply = h.delete(&format!("/v1/tabs/{tab}/keep")).await?;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body, json!({"tabId":tab}));
        let _: neo_sdk_api::TabKeepDeleted = serde_json::from_value(reply.body)?;
    }
    assert_eq!(h.service.tab_hold(TabId(3)), hold);
    assert_eq!(h.service.tab_hold(TabId(99)), None);
    Ok(())
}

#[tokio::test]
async fn ttl_releases_the_keep_at_its_deadline_and_removes_the_listing_field() -> TestResult {
    let h = support::harness().await?;
    h.browser.open_tab(3);
    tokio::time::pause();
    let deadline = Instant::now() + Duration::from_secs(2);
    assert_eq!(
        put(&h, 3, r#"{"ttlSeconds":2}"#).await?.status,
        StatusCode::OK
    );
    assert_eq!(h.service.tab_hold(TabId(3)), Some(TabHold::Now));
    tokio::time::advance(Duration::from_secs(3)).await;
    assert_eq!(h.service.tab_hold(TabId(3)), Some(TabHold::Until(deadline)));
    tokio::time::resume();
    assert_eq!(listed_keep(&h, 3).await?, None);
    Ok(())
}

#[tokio::test]
async fn a_keep_and_lock_hold_until_the_last_claim_ends() -> TestResult {
    let h = support::harness().await?;
    h.browser.open_tab(3);
    put(&h, 3, "{}").await?;
    for effect in [Effect::Read, Effect::Write] {
        assert!(
            h.service
                .locks()
                .check(LockTarget::Tab(TabId(3)), effect)
                .is_ok()
        );
    }
    let lock = h
        .post("/v1/locks", r#"{"tabId":3,"ttlSeconds":60}"#)
        .await?;
    assert_eq!(lock.status, StatusCode::OK);
    let id: LockId = lock.body["id"].as_str().ok_or("lock id")?.parse()?;
    h.delete("/v1/tabs/3/keep").await?;
    assert_eq!(h.service.tab_hold(TabId(3)), Some(TabHold::Now));
    h.service.locks().unlock_host(id)?;
    let released = h.service.tab_hold(TabId(3));
    assert!(matches!(released, Some(TabHold::Until(_))));
    put(&h, 3, "{}").await?;
    let lock = h
        .post("/v1/locks", r#"{"tabId":3,"ttlSeconds":60}"#)
        .await?;
    let id: LockId = lock.body["id"].as_str().ok_or("lock id")?.parse()?;
    h.service.locks().unlock_host(id)?;
    assert_eq!(h.service.tab_hold(TabId(3)), Some(TabHold::Now));
    h.delete("/v1/tabs/3/keep").await?;
    assert!(matches!(
        (released, h.service.tab_hold(TabId(3))),
        (Some(TabHold::Until(first)), Some(TabHold::Until(last))) if last >= first
    ));
    Ok(())
}

#[tokio::test]
async fn closes_and_reconnects_clear_keeps_without_losing_changes_newer_than_the_fence()
-> TestResult {
    let h = support::harness().await?;
    for tab in [3, 5] {
        h.browser.open_tab(tab);
        put(&h, tab, "{}").await?;
    }
    h.browser.close_tab(3);
    h.service.tab_closed(TabId(3)).await;
    assert_eq!(h.service.tab_hold(TabId(3)), None);
    let fence = h.service.tab_fence();
    h.browser.open_tab(77);
    put(&h, 77, "{}").await?;
    h.service.retain_tabs(&[], fence).await;
    assert_eq!(h.service.tab_hold(TabId(5)), None);
    assert_eq!(h.service.tab_hold(TabId(77)), Some(TabHold::Now));
    h.service.retain_tabs(&[], h.service.tab_fence()).await;
    assert_eq!(h.service.tab_hold(TabId(77)), None);
    Ok(())
}

#[tokio::test]
async fn a_tab_that_closes_between_the_put_checks_keeps_nothing() -> TestResult {
    let h = support::harness().await?;
    h.browser.open_tab(3);
    h.browser.close_after_next_listing(3);
    put(&h, 3, "{}")
        .await?
        .assert_error(StatusCode::NOT_FOUND, "tab_not_found");
    assert_eq!(h.service.tab_hold(TabId(3)), None);
    Ok(())
}

#[tokio::test]
async fn keeps_write_no_trace_or_events() -> TestResult {
    let h = support::harness().await?;
    h.browser.open_tab(3);
    let mut events = EventStream::open(&h, "/v1/events?tab=3", None).await?;
    put(&h, 3, "{}").await?;
    h.delete("/v1/tabs/3/keep").await?;
    h.browser.close_tab(3);
    h.service.tab_closed(TabId(3)).await;
    assert_eq!(events.next().await?.kind, "tab.closed");
    assert_eq!(h.get("/v1/runs").await?.body["items"], json!([]));
    Ok(())
}

#[tokio::test]
async fn keeps_require_host_auth_even_if_the_routes_are_served_alone() -> TestResult {
    let h = support::harness().await?;
    h.browser.open_tab(3);
    for method in [Method::PUT, Method::DELETE] {
        let unauthenticated = Request::builder()
            .method(method.clone())
            .uri("/v1/tabs/3/keep")
            .body(Body::empty())?;
        Reply::read(h.routes.clone().oneshot(unauthenticated).await?)
            .await?
            .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
        let wrong_token = Request::builder()
            .method(method)
            .uri("/v1/tabs/3/keep")
            .header(header::AUTHORIZATION, "Bearer wrong")
            .body(Body::empty())?;
        Reply::read(h.router.clone().oneshot(wrong_token).await?)
            .await?
            .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    assert_eq!(h.service.tab_hold(TabId(3)), None);
    Ok(())
}

#[tokio::test]
async fn a_new_service_does_not_restore_keeps_onto_the_same_browser() -> TestResult {
    let h = support::harness().await?;
    h.browser.open_tab(3);
    put(&h, 3, "{}").await?;
    let (_queue, screenshots) = screenshot_queue();
    let restarted = Service::start_with_capture(
        ServiceInputs {
            store: TraceStore::open(&h.dir.path().join("trace.db")).await?,
            sink: h.sink.clone(),
            browser: h.browser.session(),
            viewports: Arc::new(Viewports::default()),
            screenshots,
            xvnc: h.xvnc.xvnc(support::HOME_SCREEN),
            cdp_port: 0,
            settings: h.settings.clone(),
            tab_owner: Arc::new(|_| None),
        },
        Arc::new(ScriptedTransport(Arc::clone(&h.capture))),
    );
    assert_eq!(h.service.tab_hold(TabId(3)), Some(TabHold::Now));
    assert_eq!(restarted.tab_hold(TabId(3)), None);
    Ok(())
}
