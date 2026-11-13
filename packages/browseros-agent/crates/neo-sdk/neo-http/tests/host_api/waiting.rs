//! `GET /v1/runs/{runId}?wait=`: held until the run ends, or answered at the deadline with
//! what the store holds then.
//!
//! These run in real time. Paused Tokio time cannot drive a real store: SQLite answers from its
//! own thread, so the paused clock jumps past the pool's acquire timeout while a query is in
//! flight, and the store fails with `PoolTimedOut`.

use crate::support::{Start, TestResult, end_run, harness, start_known_run, start_run};
use axum::http::StatusCode;
use neo_trace::RunId;
use std::time::Duration;

#[tokio::test]
async fn wait_answers_running_at_its_deadline() -> TestResult {
    let h = harness().await?;
    let run = start_run(&h.sink, Start::default());
    h.flushed().await?;
    let started = std::time::Instant::now();
    let reply = h.get(&format!("/v1/runs/{run}?wait=1")).await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["status"], "running");
    assert!(started.elapsed() >= Duration::from_secs(1));
    Ok(())
}

#[tokio::test]
async fn wait_answers_as_soon_as_the_run_ends() -> TestResult {
    let h = harness().await?;
    let run = start_run(&h.sink, Start::default());
    h.flushed().await?;
    let started = std::time::Instant::now();
    let uri = format!("/v1/runs/{run}?wait=30");
    let ending = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        end_run(&h.sink, run, None);
    };
    let (reply, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(h.get(&uri), ending)
    })
    .await?;
    let reply = reply?;
    assert_eq!(reply.body["status"], "ok");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "held to the deadline"
    );
    Ok(())
}

#[tokio::test]
async fn wait_finds_a_run_the_writer_has_not_committed_yet() -> TestResult {
    let h = harness().await?;
    let run = RunId::new();
    let uri = format!("/v1/runs/{run}?wait=30");
    let recording = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        start_known_run(&h.sink, run, Start::default());
        end_run(&h.sink, run, None);
    };
    let (reply, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(h.get(&uri), recording)
    })
    .await?;
    let reply = reply?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["status"], "ok");
    Ok(())
}

#[tokio::test]
async fn a_bad_wait_is_a_400() -> TestResult {
    let h = harness().await?;
    for wait in ["61", "-1", "soon"] {
        h.get(&format!("/v1/runs/{}?wait={wait}", RunId::new()))
            .await?
            .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    Ok(())
}
