//! `POST /v1/tabs/{tabId}/sweep`: what a tab's sensitive fields hold reaches the known secrets
//! list, masked in every record from then on, and the host learns only a count.

use crate::support::{Start, TestResult, end_run, harness, harness_with_secrets, start_run};
use axum::http::StatusCode;
use neo_trace::SecretSetConfig;
use serde_json::json;

#[tokio::test]
async fn a_sweep_registers_what_the_fields_hold_and_answers_only_a_count() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(7);
    h.browser
        .type_into_fields(&["PWTYPED_4f7h", "4111111111111111", "987"]);

    let swept = h.post("/v1/tabs/7/sweep", "").await?;
    assert_eq!(swept.status, StatusCode::OK, "{}", swept.body);
    assert_eq!(
        swept.body,
        json!({ "registered": 2, "refused": 0 }),
        "a 3-character value is not registered"
    );
    assert!(swept.request_id().is_some());

    let run = start_run(&h.sink, Start::default());
    end_run(
        &h.sink,
        run,
        Some(json!("typed PWTYPED_4f7h and paid with 4111111111111111")),
    );
    h.flushed().await?;
    let stored = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(
        stored.body["result"],
        "typed [private] and paid with [private]"
    );
    Ok(())
}

/// sdk-traces decision 53: a set full of values it never drops refuses what a sweep reads.
/// Those values are masked nowhere, so the host learns how many there were.
#[tokio::test]
async fn a_sweep_into_a_set_full_of_host_secrets_answers_how_many_it_refused() -> TestResult {
    let h = harness_with_secrets(SecretSetConfig {
        max_entries: 2,
        ..SecretSetConfig::default()
    })
    .await?;
    for secret in ["host-secret-one", "host-secret-two"] {
        let registered = h
            .post("/v1/secrets", &json!({"value": secret}).to_string())
            .await?;
        assert_eq!(registered.status, StatusCode::NO_CONTENT);
    }
    h.browser.open_tab(7);
    h.browser
        .type_into_fields(&["PWTYPED_4f7h", "4111111111111111"]);

    let swept = h.post("/v1/tabs/7/sweep", "").await?;
    assert_eq!(swept.status, StatusCode::OK, "{}", swept.body);
    assert_eq!(swept.body, json!({ "registered": 0, "refused": 2 }));
    assert_eq!(
        h.privacy.secrets().count(),
        2,
        "the set holds the host's secrets alone"
    );
    Ok(())
}

#[tokio::test]
async fn a_sweep_of_a_tab_that_is_not_open_is_a_404_and_of_a_bad_tab_id_a_400() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(7);
    h.post("/v1/tabs/9/sweep", "")
        .await?
        .assert_error(StatusCode::NOT_FOUND, "tab_not_found");
    h.post("/v1/tabs/nine/sweep", "")
        .await?
        .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    assert_eq!(h.privacy.secrets().count(), 0);
    Ok(())
}

#[tokio::test]
async fn a_sweep_when_the_browser_does_not_answer_is_a_503() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(7);
    h.browser.type_into_fields(&["PWTYPED_4f7h"]);
    h.browser.become_unreachable();
    h.post("/v1/tabs/7/sweep", "")
        .await?
        .assert_error(StatusCode::SERVICE_UNAVAILABLE, "browser_unavailable");
    assert_eq!(h.privacy.secrets().count(), 0);
    Ok(())
}
