//! Screenshots end to end: the writer's jobs, the camera on a scripted browser, and the routes a
//! host reads them through (`wait=` until `complete`, the JPEG route, `hasScreenshot`, inlined
//! images and `visuals`).
//!
//! Real time, like `waiting`: paused Tokio time cannot drive a real store.

use crate::browser;
use crate::support::{
    Harness, Op, Start, TestResult, end_run, harness_with_camera, op_ended, op_started, start_run,
};
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use neo_trace::{Delivery, Effect, OperationId, RetentionPolicy, RunId};
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

fn read(h: &Harness, op: OperationId, tab: i64) {
    let title = Op {
        name: "page.title",
        effect: Effect::Read,
        tab: Some(tab),
        at: 2,
    };
    op_started(&h.sink, op, title);
    op_ended(&h.sink, op, Delivery::Done, None);
}

/// The JPEG served for `op`, or `None` when the route answers anything but 200.
async fn served(h: &Harness, op: OperationId) -> Result<Option<Vec<u8>>, crate::support::BoxError> {
    let (status, _, bytes) = h
        .get_bytes(&format!("/v1/operations/{op}/screenshot"))
        .await?;
    Ok((status == StatusCode::OK).then_some(bytes))
}

/// Holds the camera on a write's capture on tab 5, so the jobs a test makes next wait behind it.
/// Returns that write's run.
async fn busy_camera(h: &Harness) -> Result<RunId, crate::support::BoxError> {
    h.browser.open_tab(5);
    h.browser.hold_captures();
    let (busy, _) = write_run(h, 5);
    h.browser.captures_started(1).await?;
    Ok(busy)
}

/// `GET /v1/runs/{run}?wait=30`, which must answer complete.
async fn complete(h: &Harness, run: RunId) -> Result<serde_json::Value, crate::support::BoxError> {
    let reply = h.get(&format!("/v1/runs/{run}?wait=30")).await?;
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

    let (status, headers, bytes) = h
        .get_bytes(&format!("/v1/operations/{op}/screenshot"))
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("image/jpeg")
    );
    assert_eq!(bytes, browser::jpeg(1));
    assert!(file_of(&h, op).exists());

    let inlined = h
        .get(&format!("/v1/runs/{run}?include=screenshots"))
        .await?;
    let shot = &inlined.body["operations"][0]["screenshot"];
    assert_eq!(shot["contentType"], "image/jpeg");
    assert_eq!(shot["data"], STANDARD.encode(browser::jpeg(1)));
    let tail = h.get("/v1/trace?tab=3&screenshots=last:1").await?;
    assert_eq!(
        tail.body["operations"][0]["screenshot"]["data"],
        STANDARD.encode(browser::jpeg(1))
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
async fn two_writes_on_a_tab_each_keep_their_own_capture_in_order() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    let busy = busy_camera(&h).await?;

    let run = start_run(&h.sink, Start::default());
    let (first, second) = (OperationId::new(run, 1), OperationId::new(run, 2));
    write(&h, first, 3);
    write(&h, second, 3);
    end_run(&h.sink, run, None);
    h.flushed().await?;
    let waiting = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(waiting.body["complete"], false, "both jobs are out");
    h.browser.release_captures();

    complete(&h, busy).await?;
    let body = complete(&h, run).await?;
    assert_eq!(body["operations"][0]["hasScreenshot"], true, "{body}");
    assert_eq!(body["operations"][1]["hasScreenshot"], true, "{body}");
    assert_eq!(
        served(&h, first).await?,
        Some(browser::jpeg(2)),
        "after tab 5's"
    );
    assert_eq!(served(&h, second).await?, Some(browser::jpeg(3)));
    assert_eq!(h.browser.captures(), 3);
    Ok(())
}

#[tokio::test]
async fn a_run_end_shot_folds_into_the_write_job_still_waiting_on_its_tab() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    let busy = busy_camera(&h).await?;

    let run = start_run(&h.sink, Start::default());
    let (click, title) = (OperationId::new(run, 1), OperationId::new(run, 2));
    write(&h, click, 3);
    read(&h, title, 3);
    end_run(&h.sink, run, None);
    h.flushed().await?;
    let waiting = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(waiting.body["complete"], false, "the click's job is out");
    h.browser.release_captures();

    complete(&h, busy).await?;
    let body = complete(&h, run).await?;
    assert_eq!(body["operations"][0]["hasScreenshot"], true, "{body}");
    assert_eq!(body["operations"][1]["hasScreenshot"], false, "{body}");
    assert_eq!(served(&h, click).await?, Some(browser::jpeg(2)));
    h.get(&format!("/v1/operations/{title}/screenshot"))
        .await?
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    assert_eq!(h.browser.captures(), 2, "tab 5, then tab 3 once");
    Ok(())
}

#[tokio::test]
async fn a_run_end_shot_is_still_taken_once_the_writes_picture_is_in() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    let run = start_run(&h.sink, Start::default());
    let (click, title) = (OperationId::new(run, 1), OperationId::new(run, 2));
    write(&h, click, 3);
    tokio::time::timeout(Duration::from_secs(5), async {
        while served(&h, click).await.ok().flatten().is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    read(&h, title, 3);
    end_run(&h.sink, run, None);

    complete(&h, run).await?;
    assert_eq!(served(&h, click).await?, Some(browser::jpeg(1)));
    assert_eq!(
        served(&h, title).await?,
        Some(browser::jpeg(2)),
        "the page may have changed since the click's picture"
    );
    Ok(())
}

#[tokio::test]
async fn past_four_jobs_waiting_on_a_tab_the_oldest_is_dropped() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    let busy = busy_camera(&h).await?;

    let run = start_run(&h.sink, Start::default());
    let writes: Vec<OperationId> = (1..=5).map(|seq| OperationId::new(run, seq)).collect();
    for op in &writes {
        write(&h, *op, 3);
    }
    end_run(&h.sink, run, None);
    h.flushed().await?;
    h.browser.release_captures();

    complete(&h, busy).await?;
    let body = complete(&h, run).await?;
    let kept: Vec<serde_json::Value> = (0..writes.len())
        .map(|index| body["operations"][index]["hasScreenshot"].clone())
        .collect();
    assert_eq!(kept, [false, true, true, true, true], "{body}");
    assert_eq!(served(&h, writes[0]).await?, None);
    for (op, capture) in writes[1..].iter().zip(2..) {
        assert_eq!(served(&h, *op).await?, Some(browser::jpeg(capture)), "{op}");
    }
    assert_eq!(h.browser.captures(), 5, "tab 5, then four on tab 3");
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
    assert_eq!(h.browser.captures(), 1, "a withheld capture is not retried");
    Ok(())
}

#[tokio::test]
async fn a_capture_that_times_out_once_is_retried_and_taken() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    h.browser.hang_capture(1);
    let started = Instant::now();
    let (run, op) = write_run(&h, 3);
    let body = complete(&h, run).await?;
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(2500), "{waited:?}");
    assert_eq!(body["operations"][0]["hasScreenshot"], true, "{body}");
    assert_eq!(
        served(&h, op).await?,
        Some(browser::jpeg(2)),
        "the retry's picture"
    );
    assert_eq!(h.browser.captures(), 2);
    Ok(())
}

#[tokio::test]
async fn a_capture_that_times_out_every_time_is_dropped_within_its_budget() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    h.browser.hang_captures();
    let started = Instant::now();
    let (run, op) = write_run(&h, 3);
    let body = complete(&h, run).await?;
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_secs(7),
        "three 2 s attempts and two 0.5 s pauses: {waited:?}"
    );
    assert!(
        waited < Duration::from_secs(9),
        "the 8 s budget: {waited:?}"
    );
    assert_eq!(body["operations"][0]["hasScreenshot"], false);
    h.get(&format!("/v1/operations/{op}/screenshot"))
        .await?
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    assert_eq!(h.browser.captures(), 3, "the first try and two retries");
    Ok(())
}

#[tokio::test]
async fn a_tab_that_goes_private_before_a_retry_gets_no_new_capture() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    h.browser.hang_capture(1);
    let (run, op) = write_run(&h, 3);
    h.browser.captures_started(1).await?;
    assert_eq!(
        h.post("/v1/tabs/3/private", "").await?.status,
        StatusCode::OK
    );
    complete(&h, run).await?;
    h.get(&format!("/v1/operations/{op}/screenshot"))
        .await?
        .assert_error(StatusCode::GONE, "withheld");
    assert_eq!(h.browser.captures(), 1);
    Ok(())
}

#[tokio::test]
async fn a_run_waits_for_every_answer_while_one_capture_is_retried() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    h.browser.hang_capture(2);
    let run = start_run(&h.sink, Start::default());
    let (first, second) = (OperationId::new(run, 1), OperationId::new(run, 2));
    write(&h, first, 3);
    write(&h, second, 3);
    end_run(&h.sink, run, None);
    // The second capture started once the first job was answered; its retry is 2.5 s away.
    h.browser.captures_started(2).await?;
    h.flushed().await?;
    let midway = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(midway.body["status"], "ok");
    assert_eq!(midway.body["complete"], false, "{}", midway.body);
    assert_eq!(midway.body["operations"][0]["hasScreenshot"], true);
    assert_eq!(midway.body["operations"][1]["hasScreenshot"], false);

    let body = complete(&h, run).await?;
    assert_eq!(body["operations"][1]["hasScreenshot"], true, "{body}");
    assert_eq!(served(&h, first).await?, Some(browser::jpeg(1)));
    assert_eq!(served(&h, second).await?, Some(browser::jpeg(3)));
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
