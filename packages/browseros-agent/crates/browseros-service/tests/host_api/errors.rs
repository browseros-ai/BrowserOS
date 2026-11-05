//! Every response carries `x-request-id`, and every error is the contract's `Error` with that
//! id in `requestId`, whichever layer refused the request.

use crate::support::{TestResult, harness};
use axum::body::Body;
use axum::http::{Method, StatusCode};
use browseros_trace::RunId;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

#[tokio::test]
async fn every_response_carries_its_own_request_id() -> TestResult {
    let h = harness().await?;
    let first = h.get("/v1/runs").await?;
    let second = h.get("/v1/runs").await?;
    assert_eq!(first.status, StatusCode::OK);
    let (Some(first), Some(second)) = (first.request_id(), second.request_id()) else {
        panic!("a response without x-request-id");
    };
    assert_eq!(first.len(), 26, "a ULID: {first}");
    assert_ne!(first, second);
    Ok(())
}

#[tokio::test]
async fn errors_are_the_contracts_body_with_the_same_request_id() -> TestResult {
    let h = harness().await?;
    let not_found = [
        "/v1/nothing-here".to_string(),
        "/v2/runs".to_string(),
        format!("/v1/runs/{}", RunId::new()),
        "/v1/runs/not-a-run".to_string(),
        format!("/v1/operations/{}.1", RunId::new()),
        "/v1/operations/not-an-operation".to_string(),
    ];
    for uri in not_found {
        h.get(&uri)
            .await?
            .assert_error(StatusCode::NOT_FOUND, "not_found");
    }
    h.get("/v1/runs?limit=0")
        .await?
        .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    h.call(Method::DELETE, "/v1/runs", Body::empty())
        .await?
        .assert_error(StatusCode::METHOD_NOT_ALLOWED, "invalid_request");
    Ok(())
}

#[tokio::test]
async fn a_store_failure_is_a_500_that_names_no_storage_detail() -> TestResult {
    let h = harness().await?;
    let other = SqlitePool::connect_with(
        SqliteConnectOptions::new().filename(h.dir.path().join("trace.db")),
    )
    .await?;
    sqlx::query("DROP TABLE runs").execute(&other).await?;
    let reply = h.get("/v1/runs").await?;
    reply.assert_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error");
    assert_eq!(reply.body["message"], "internal error");
    Ok(())
}
