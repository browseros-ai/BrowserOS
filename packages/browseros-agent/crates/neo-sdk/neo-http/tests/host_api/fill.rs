//! `POST /v1/tabs/{tabId}/fill` and `/fill/check` at the HTTP edge: who is refused before the
//! app's door runs, what the door is handed (the body's bytes, unread), and how its refusals look
//! on the wire. What a fill does is the app's (neo-server's `host/fill_tests.rs`).

use std::sync::{Arc, Mutex, PoisonError};

use crate::support::{TestResult, harness};
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use browseros_core::TabId;
use neo_http::{FillAsk, FillDoor, FillRefusal, FillReply, FillVerb};
use neo_sdk_api as api;
use tower::ServiceExt;

const BODY: &str =
    r#"{"fields":[{"ref":"e5","value":"CANARY-edge-pw","expect":{"origin":"https://a.example"}}]}"#;

/// What a door was handed: each call's route, tab and body bytes.
type Seen = Arc<Mutex<Vec<(FillVerb, TabId, Vec<u8>)>>>;

/// A door's scripted answer.
type Answer = fn(FillVerb) -> Result<FillReply, FillRefusal>;

/// A door that remembers what it was handed and answers `answer`.
fn door(seen: &Seen, answer: Answer) -> FillDoor {
    let seen = Arc::clone(seen);
    Arc::new(move |ask: FillAsk| {
        seen.lock().unwrap_or_else(PoisonError::into_inner).push((
            ask.verb,
            ask.tab,
            ask.body.to_vec(),
        ));
        Box::pin(async move { answer(ask.verb) })
    })
}

fn entered(verb: FillVerb) -> Result<FillReply, FillRefusal> {
    Ok(match verb {
        FillVerb::Fill => FillReply::Fill(api::FillResult {
            typed: Some(vec![14]),
            ..api::FillResult::new("run-1".into(), "entered".into(), "done".into())
        }),
        FillVerb::Check => FillReply::Check(api::FillCheckResult::new("run-2".into(), Vec::new())),
    })
}

#[tokio::test]
async fn an_unarmed_route_answers_unavailable() -> TestResult {
    let h = harness().await?;
    h.post("/v1/tabs/7/fill", BODY)
        .await?
        .assert_error(StatusCode::SERVICE_UNAVAILABLE, "browser_unavailable");
    Ok(())
}

#[tokio::test]
async fn the_door_gets_the_tab_and_the_body_unread() -> TestResult {
    let h = harness().await?;
    let seen = Arc::default();
    h.service.arm_fill(door(&seen, entered));
    let filled = h.post("/v1/tabs/7/fill", BODY).await?;
    assert_eq!(filled.status, StatusCode::OK, "{:?}", filled.body);
    assert_eq!(filled.body["outcome"], "entered");
    assert_eq!(filled.body["typed"], serde_json::json!([14]));
    let checked = h.post("/v1/tabs/7/fill/check", "{}").await?;
    assert_eq!(checked.status, StatusCode::OK);
    assert_eq!(checked.body["runId"], "run-2");
    let seen = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(
        seen,
        vec![
            (FillVerb::Fill, TabId(7), BODY.as_bytes().to_vec()),
            (FillVerb::Check, TabId(7), b"{}".to_vec()),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn a_pages_request_or_a_query_is_refused_before_the_door() -> TestResult {
    let h = harness().await?;
    let seen = Arc::default();
    h.service.arm_fill(door(&seen, entered));
    for (name, value) in [
        ("origin", "http://127.0.0.1:9000"),
        ("sec-fetch-site", "same-origin"),
    ] {
        for path in ["/v1/tabs/7/fill", "/v1/tabs/7/fill/check"] {
            let request = Request::builder()
                .method(Method::POST)
                .uri(path)
                .header("content-type", "application/json")
                .header(name, value)
                .body(Body::from(BODY))?;
            let reply =
                crate::support::Reply::read(h.router.clone().oneshot(request).await?).await?;
            reply.assert_error(StatusCode::FORBIDDEN, "forbidden");
        }
    }
    h.post("/v1/tabs/7/fill?value=CANARY", BODY)
        .await?
        .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    h.post("/v1/tabs/not-a-tab/fill", BODY)
        .await?
        .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    assert!(
        seen.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty(),
        "no refused request reached the door"
    );
    Ok(())
}

#[tokio::test]
async fn the_doors_refusals_are_the_contracts_errors() -> TestResult {
    let cases: [(Answer, StatusCode, &str); 6] = [
        (
            |_| Err(FillRefusal::Invalid("body.fields: x".into())),
            StatusCode::BAD_REQUEST,
            "invalid_request",
        ),
        (
            |_| Err(FillRefusal::TabNotFound),
            StatusCode::NOT_FOUND,
            "tab_not_found",
        ),
        (
            |_| Err(FillRefusal::LockReleased("not held".into())),
            StatusCode::CONFLICT,
            "lock_released",
        ),
        (
            |_| Err(FillRefusal::BrowserUnavailable("later".into())),
            StatusCode::SERVICE_UNAVAILABLE,
            "browser_unavailable",
        ),
        (
            |_| Err(FillRefusal::TraceUnavailable("no record".into())),
            StatusCode::SERVICE_UNAVAILABLE,
            "trace_unavailable",
        ),
        (
            |_| Err(FillRefusal::Internal("a cause".into())),
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
        ),
    ];
    for (answer, status, code) in cases {
        let h = harness().await?;
        h.service.arm_fill(door(&Arc::default(), answer));
        let reply = h.post("/v1/tabs/7/fill", BODY).await?;
        reply.assert_error(status, code);
        assert!(
            !reply.body.to_string().contains("a cause"),
            "a failure inside Neo is logged, never sent: {:?}",
            reply.body
        );
    }
    Ok(())
}
