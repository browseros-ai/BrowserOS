//! Tab privacy and secret registration through the routes.

use crate::support::{
    Op, Start, TestResult, end_run, harness, harness_with, harness_with_secrets, op_ended,
    op_started, start_run,
};
use axum::http::StatusCode;
use browseros_core::TabId;
use futures_util::future::BoxFuture;
use neo_trace::{
    Delivery, Effect, OperationId, PrivacyRecord, PrivacyStore, SecretSetConfig, TraceError,
};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn a_private_tab_returns_its_record_and_keeps_the_first() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(7);
    let first = h.post("/v1/tabs/7/private", "").await?;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    assert_eq!(first.body["private"], true);
    assert_eq!(first.body["reason"], "host");
    let since = first.body["since"].as_i64();
    assert!(since.is_some());
    assert_eq!(
        h.privacy
            .is_private(TabId(7))
            .map(|record| record.since.millis()),
        since
    );
    assert!(
        h.store.private_tabs().await?.contains_key(&TabId(7)),
        "on disk when the route answers"
    );

    tokio::time::sleep(Duration::from_millis(5)).await;
    let second = h.post("/v1/tabs/7/private", r#"{"reason":"host"}"#).await?;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(second.body, first.body, "the first record stays");
    Ok(())
}

#[tokio::test]
async fn a_bad_privacy_request_changes_nothing() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(8);
    for (uri, body) in [
        ("/v1/tabs/8/private", r#"{"reason":"owner"}"#),
        ("/v1/tabs/8/private", "not json"),
        ("/v1/tabs/eight/private", ""),
    ] {
        h.post(uri, body)
            .await?
            .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    assert!(h.privacy.is_private(TabId(8)).is_none());
    Ok(())
}

/// A store whose commits always fail.
struct Unwritable;

impl PrivacyStore for Unwritable {
    fn persist_private(&self, _: TabId, _: PrivacyRecord) -> BoxFuture<'_, Result<(), TraceError>> {
        Box::pin(async { Err(TraceError::Closed) })
    }

    fn forget_private(&self, _: TabId) -> BoxFuture<'_, Result<(), TraceError>> {
        Box::pin(async { Err(TraceError::Closed) })
    }
}

#[tokio::test]
async fn a_flag_that_cannot_be_committed_is_a_503() -> TestResult {
    let h = harness_with(|_| Arc::new(Unwritable)).await?;
    h.browser.open_tab(7);
    h.post("/v1/tabs/7/private", "")
        .await?
        .assert_error(StatusCode::SERVICE_UNAVAILABLE, "privacy_unavailable");
    assert!(
        h.privacy.is_private(TabId(7)).is_some(),
        "captures stay refused even though the flag is not on disk"
    );
    Ok(())
}

#[tokio::test]
async fn a_registered_secret_is_masked_in_what_the_store_keeps() -> TestResult {
    let h = harness().await?;
    let secret = "hunter2-swordfish";
    let registered = h
        .post(
            "/v1/secrets",
            &json!({"value": secret, "ttlSeconds": 600}).to_string(),
        )
        .await?;
    assert_eq!(registered.status, StatusCode::NO_CONTENT);
    assert_eq!(registered.body, Value::Null);
    assert!(registered.request_id().is_some());

    let run = start_run(&h.sink, Start::default());
    let read = OperationId::new(run, 1);
    op_started(
        &h.sink,
        read,
        Op {
            name: "page.title",
            effect: Effect::Read,
            tab: Some(3),
            at: 1,
        },
    );
    op_ended(
        &h.sink,
        read,
        Delivery::Done,
        Some(json!({"title": format!("Hi {secret}")})),
    );
    end_run(&h.sink, run, Some(json!(format!("token {secret}"))));
    h.flushed().await?;

    let stored = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(stored.body["result"], "token [private]");
    assert_eq!(
        stored.body["operations"][0]["result"],
        json!({"title": "Hi [private]"})
    );
    assert!(!stored.body.to_string().contains(secret));
    Ok(())
}

/// sdk-traces decision 53: a page can put any number of values in its fields, so a sweep that
/// reads more than the secret set holds must not unmask what the host registered.
#[tokio::test]
async fn a_host_secret_outlasts_a_flood_of_swept_page_values() -> TestResult {
    let h = harness_with_secrets(SecretSetConfig {
        max_entries: 8,
        ..SecretSetConfig::default()
    })
    .await?;
    let secret = "hunter2-swordfish";
    let registered = h
        .post("/v1/secrets", &json!({"value": secret}).to_string())
        .await?;
    assert_eq!(registered.status, StatusCode::NO_CONTENT);
    h.browser.open_tab(7);
    let flood = (0..10)
        .map(|i| format!("PAGEFLOOD_{i:02}"))
        .collect::<Vec<_>>();
    h.browser
        .type_into_fields(&flood.iter().map(String::as_str).collect::<Vec<_>>());
    let swept = h.post("/v1/tabs/7/sweep", "").await?;
    assert_eq!(swept.status, StatusCode::OK, "{}", swept.body);
    assert_eq!(swept.body["registered"], 10);

    let run = start_run(&h.sink, Start::default());
    end_run(&h.sink, run, Some(json!(format!("token {secret}"))));
    h.flushed().await?;
    let stored = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(stored.body["result"], "token [private]");
    Ok(())
}

#[tokio::test]
async fn a_refused_secret_is_a_400_that_never_echoes_the_body() -> TestResult {
    let h = harness().await?;
    for body in [
        r#"{"value":"abc"}"#,
        r#"{"value":""}"#,
        r#"{"value":"long-enough-value","ttlSeconds":59}"#,
        r#"{"value":"long-enough-value","ttlSeconds":86401}"#,
        r#"{"value":"long-enough-value","ttlSeconds":"later"}"#,
        r#"{"secret":"long-enough-value"}"#,
        r#"{"value":["long-enough-value"]}"#,
        "long-enough-value",
    ] {
        let reply = h.post("/v1/secrets", body).await?;
        reply.assert_error(StatusCode::BAD_REQUEST, "invalid_request");
        let message = reply.body["message"].as_str().unwrap_or_default();
        for part in ["abc", "long-enough-value", "later"] {
            assert!(!message.contains(part), "{message:?} echoes {part:?}");
        }
    }
    assert_eq!(h.privacy.secrets().count(), 0);
    Ok(())
}
