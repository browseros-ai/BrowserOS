//! Screenshots end to end: the writer's jobs, the camera on a scripted browser, and the routes a
//! host reads them through (`wait=` until `complete`, the JPEG route, `hasScreenshot`, inlined
//! images and `visuals`).
//!
//! Real time, like `waiting`: paused Tokio time cannot drive a real store.

use crate::browser::JPEG;
use crate::support::{
    Harness, Op, Start, TestResult, end_run, harness_with_camera, op_ended, op_started, start_run,
};
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use browseros_trace::{Delivery, OperationId, RetentionPolicy, RunId};
use std::time::{Duration, Instant};

/// A run of one write on `tab`, start to end.
fn write_run(h: &Harness, tab: i64) -> (RunId, OperationId) {
    let run = start_run(&h.sink, Start::default());
    let op = OperationId::new(run, 1);
    write(h, op, tab);
    end_run(&h.sink, run, None);
    (run, op)
}

fn write(h: &Harness, op: OperationId, tab: i64) {
    op_started(&h.sink, op, Op::write(tab, 1));
    op_ended(&h.sink, op, Delivery::Done, None);
}

/// `GET /v1/runs/{run}?wait=10`, which must answer complete.
async fn complete(h: &Harness, run: RunId) -> Result<serde_json::Value, crate::support::BoxError> {
    let reply = h.get(&format!("/v1/runs/{run}?wait=10")).await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["complete"], true, "{}", reply.body);
    Ok(reply.body)
}

fn file_of(h: &Harness, op: OperationId) -> std::path::PathBuf {
    h.dir
        .path()
        .join(format!("screenshots/{}/{}.jpg", op.run, op.seq))
}

#[tokio::test]
async fn a_write_on_an_open_tab_serves_its_jpeg() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    let (run, op) = write_run(&h, 3);
    let body = complete(&h, run).await?;
    assert_eq!(body["operations"][0]["hasScreenshot"], true);
    assert!(
        body["operations"][0].get("screenshot").is_none(),
        "inlined only when asked"
    );

    let (status, headers, jpeg) = h
        .get_bytes(&format!("/v1/operations/{op}/screenshot"))
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("image/jpeg")
    );
    assert_eq!(jpeg, JPEG);
    assert!(file_of(&h, op).exists());

    let inlined = h
        .get(&format!("/v1/runs/{run}?include=screenshots"))
        .await?;
    let shot = &inlined.body["operations"][0]["screenshot"];
    assert_eq!(shot["contentType"], "image/jpeg");
    assert_eq!(shot["data"], STANDARD.encode(JPEG));
    let tail = h.get("/v1/trace?tab=3&screenshots=last:1").await?;
    assert_eq!(
        tail.body["operations"][0]["screenshot"]["data"],
        STANDARD.encode(JPEG)
    );
    assert_eq!(tail.body["visuals"], "available");
    let plain = h.get("/v1/trace?tab=3").await?;
    assert!(plain.body["operations"][0].get("screenshot").is_none());

    let swept = h
        .sink
        .sweep(
            RetentionPolicy {
                screenshot_cap: 0,
                ..RetentionPolicy::default()
            },
            crate::support::at(30_000),
        )
        .await?;
    assert_eq!(swept.screenshots, 1);
    h.get(&format!("/v1/operations/{op}/screenshot"))
        .await?
        .assert_error(StatusCode::GONE, "expired");
    let read = h.get(&format!("/v1/operations/{op}")).await?;
    assert_eq!(read.body["hasScreenshot"], false);
    Ok(())
}

#[tokio::test]
async fn wait_answers_once_the_run_is_complete_not_when_it_ends() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    h.browser.hold_captures();
    let (run, _) = write_run(&h, 3);
    h.flushed().await?;
    h.browser.captures_started(1).await?;
    let ended = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(ended.body["status"], "ok");
    assert_eq!(
        ended.body["complete"], false,
        "its screenshot is being taken"
    );

    let started = Instant::now();
    let releasing = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        h.browser.release_captures();
    };
    let uri = format!("/v1/runs/{run}?wait=30");
    let (reply, ()) = tokio::join!(h.get(&uri), releasing);
    let reply = reply?;
    assert_eq!(reply.body["complete"], true);
    assert_eq!(reply.body["operations"][0]["hasScreenshot"], true);
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(300) && waited < Duration::from_secs(10),
        "{waited:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_newer_job_on_a_tab_drops_the_one_waiting() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    h.browser.open_tab(5);
    h.browser.hold_captures();
    let (busy, _) = write_run(&h, 5);
    h.browser.captures_started(1).await?;

    let run = start_run(&h.sink, Start::default());
    let older = OperationId::new(run, 1);
    let newer = OperationId::new(run, 2);
    write(&h, older, 3);
    write(&h, newer, 3);
    end_run(&h.sink, run, None);
    h.flushed().await?;
    h.browser.release_captures();

    complete(&h, busy).await?;
    let body = complete(&h, run).await?;
    assert_eq!(body["operations"][0]["hasScreenshot"], false);
    assert_eq!(body["operations"][1]["hasScreenshot"], true);
    h.get(&format!("/v1/operations/{older}/screenshot"))
        .await?
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    assert_eq!(
        h.get_bytes(&format!("/v1/operations/{newer}/screenshot"))
            .await?
            .0,
        StatusCode::OK
    );
    assert_eq!(h.browser.captures(), 2, "tab 5, then tab 3 once");
    Ok(())
}

#[tokio::test]
async fn a_private_tab_is_withheld_with_no_browser_call_and_no_file() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    assert_eq!(
        h.post("/v1/tabs/3/private", "").await?.status,
        StatusCode::OK
    );
    let (run, op) = write_run(&h, 3);
    let body = complete(&h, run).await?;
    assert_eq!(body["operations"][0]["hasScreenshot"], false);
    h.get(&format!("/v1/operations/{op}/screenshot"))
        .await?
        .assert_error(StatusCode::GONE, "withheld");
    assert_eq!(h.browser.captures(), 0);
    assert!(
        !h.dir.path().join("screenshots").exists(),
        "not a byte of the private tab on disk"
    );
    let tail = h.get("/v1/trace?tab=3").await?;
    assert_eq!(tail.body["visuals"], "withheld_private_input");
    Ok(())
}

#[tokio::test]
async fn a_tab_that_goes_private_during_its_capture_keeps_nothing() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    h.browser.hold_captures();
    let (run, op) = write_run(&h, 3);
    h.browser.captures_started(1).await?;
    assert_eq!(
        h.post("/v1/tabs/3/private", "").await?.status,
        StatusCode::OK
    );
    h.browser.release_captures();
    complete(&h, run).await?;
    h.get(&format!("/v1/operations/{op}/screenshot"))
        .await?
        .assert_error(StatusCode::GONE, "withheld");
    assert!(!file_of(&h, op).exists());
    assert!(
        !h.dir.path().join("screenshots").exists(),
        "nothing written, not even a directory"
    );
    Ok(())
}

#[tokio::test]
async fn a_capture_past_its_timeout_is_dropped() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    h.browser.hang_captures();
    let started = Instant::now();
    let (run, op) = write_run(&h, 3);
    let body = complete(&h, run).await?;
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "the 2 s timeout"
    );
    assert_eq!(body["operations"][0]["hasScreenshot"], false);
    h.get(&format!("/v1/operations/{op}/screenshot"))
        .await?
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    Ok(())
}

#[tokio::test]
async fn a_closed_tab_is_dropped_and_an_operation_without_a_screenshot_is_a_404() -> TestResult {
    let h = harness_with_camera().await?;
    let (run, op) = write_run(&h, 3);
    complete(&h, run).await?;
    h.get(&format!("/v1/operations/{op}/screenshot"))
        .await?
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    assert_eq!(h.browser.captures(), 0);
    for id in [
        OperationId::new(run, 9).to_string(),
        OperationId::new(RunId::new(), 1).to_string(),
        "not-an-operation".to_string(),
    ] {
        h.get(&format!("/v1/operations/{id}/screenshot"))
            .await?
            .assert_error(StatusCode::NOT_FOUND, "not_found");
    }
    Ok(())
}
