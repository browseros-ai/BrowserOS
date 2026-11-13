//! Screenshots end to end: the writer's jobs, the camera on a scripted browser, and the routes a
//! host reads them through (`wait=` until `complete`, the JPEG route, `hasScreenshot`, inlined
//! images).
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
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use std::time::{Duration, Instant};

/// The camera's budget for one capture, attempts and pauses included (`limits::CAPTURE_BUDGET`,
/// private to the crate).
const CAPTURE_BUDGET: Duration = Duration::from_secs(8);

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
    assert!(tail.body.get("visuals").is_none());
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
async fn a_trace_capture_in_flight_keeps_its_jpeg_during_a_takeover() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    h.browser.hold_captures();
    let (run, op) = write_run(&h, 3);
    h.browser.captures_started(1).await?;
    let takeover = h.post("/v1/takeovers", r#"{"tabId":3}"#).await?;
    assert_eq!(takeover.status, StatusCode::OK, "{}", takeover.body);
    h.browser.release_captures();

    assert_eq!(
        complete(&h, run).await?["operations"][0]["hasScreenshot"],
        true
    );
    assert_eq!(served(&h, op).await?, Some(browser::jpeg(1)));
    Ok(())
}

#[tokio::test]
async fn a_legacy_withheld_row_reads_as_an_operation_without_a_jpeg() -> TestResult {
    let h = crate::support::harness().await?;
    let (run, op) = write_run(&h, 3);
    h.flushed().await?;
    // Seed the old persisted spelling through SQLite so this also exercises the row decoder.
    let db = SqlitePool::connect_with(
        SqliteConnectOptions::new().filename(h.dir.path().join("trace.db")),
    )
    .await?;
    let updated = sqlx::query("UPDATE operations SET screenshot = 'withheld' WHERE run_id = ?")
        .bind(run.to_string())
        .execute(&db)
        .await?;
    assert_eq!(updated.rows_affected(), 1);

    h.get(&format!("/v1/operations/{op}/screenshot"))
        .await?
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    let tail = h.get("/v1/trace?tab=3&screenshots=all").await?;
    assert_eq!(tail.status, StatusCode::OK, "{}", tail.body);
    let operation = &tail.body["operations"][0];
    assert_eq!(operation["hasScreenshot"], false);
    assert!(operation.get("screenshot").is_none());
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
    // Real time on a real store (see the file's header), so a loaded machine runs long. The
    // bound only catches a capture retried without end; `complete` waits 30 s for the run.
    assert!(
        waited < CAPTURE_BUDGET + Duration::from_secs(20),
        "dropped once its budget is spent: {waited:?}"
    );
    assert_eq!(body["operations"][0]["hasScreenshot"], false);
    h.get(&format!("/v1/operations/{op}/screenshot"))
        .await?
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    assert_eq!(h.browser.captures(), 3, "the first try and two retries");
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

/// A tab that took a host's private typing is never pictured (protected-fill design, section
/// 10.3): the camera asks before a capture and records `withheld`, writing nothing, while another
/// tab's write is pictured as usual.
#[tokio::test]
async fn a_tab_that_took_a_private_fill_is_withheld() -> TestResult {
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    h.browser.open_tab(4);
    h.service
        .arm_private_tabs(std::sync::Arc::new(|tab| tab == browseros_core::TabId(3)));
    let (run, op) = write_run(&h, 3);
    let body = complete(&h, run).await?;
    assert_eq!(body["operations"][0]["hasScreenshot"], false, "{body}");
    let withheld = h.get(&format!("/v1/operations/{op}/screenshot")).await?;
    withheld.assert_error(StatusCode::NOT_FOUND, "not_found");
    assert!(
        withheld.body["message"]
            .as_str()
            .is_some_and(|message| message.contains("withheld")),
        "{}",
        withheld.body
    );
    assert!(!file_of(&h, op).exists(), "nothing written");
    assert_eq!(
        h.browser.captures(),
        0,
        "the camera asked before the capture, so none started"
    );
    let (other_run, other) = write_run(&h, 4);
    complete(&h, other_run).await?;
    assert!(
        served(&h, other).await?.is_some(),
        "another tab is pictured"
    );
    Ok(())
}

/// A capture already running when a fill starts on its tab carries the value in its pixels: the
/// camera asks again after the capture, and throws it away unwritten.
#[tokio::test]
async fn a_capture_a_private_fill_straddles_is_thrown_away() -> TestResult {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    let filled = Arc::new(AtomicBool::new(false));
    let took = Arc::clone(&filled);
    h.service
        .arm_private_tabs(Arc::new(move |_tab| took.load(Ordering::SeqCst)));
    h.browser.hold_captures();
    let (run, op) = write_run(&h, 3);
    h.browser.captures_started(1).await?;
    // The fill's first character goes while the capture is out.
    filled.store(true, Ordering::SeqCst);
    h.browser.release_captures();
    let body = complete(&h, run).await?;
    assert_eq!(body["operations"][0]["hasScreenshot"], false, "{body}");
    assert!(
        !file_of(&h, op).exists(),
        "the straddling capture was not written"
    );
    assert!(served(&h, op).await?.is_none());
    Ok(())
}

/// Ruling R45: the app reads what a tab shows before a capture. A frame showing a private box's
/// value withholds the picture with no capture started; a tab reading clear is pictured.
#[tokio::test]
async fn a_tab_showing_a_private_value_is_withheld_before_any_capture() -> TestResult {
    use std::sync::Arc;
    let h = harness_with_camera().await?;
    h.browser.open_tab(3);
    h.browser.open_tab(4);
    h.service.arm_picture_check(Arc::new(|tab| {
        Box::pin(async move {
            if tab == browseros_core::TabId(3) {
                neo_http::PictureVerdict::Private
            } else {
                neo_http::PictureVerdict::Clear {
                    reading: "doc".to_owned(),
                }
            }
        })
    }));
    let (run, op) = write_run(&h, 3);
    let body = complete(&h, run).await?;
    assert_eq!(body["operations"][0]["hasScreenshot"], false, "{body}");
    assert!(!file_of(&h, op).exists(), "nothing written");
    assert_eq!(h.browser.captures(), 0, "no capture started");
    let (other_run, other) = write_run(&h, 4);
    complete(&h, other_run).await?;
    assert!(
        served(&h, other).await?.is_some(),
        "a clear tab is pictured"
    );
    Ok(())
}

/// Ruling R45: read again after the capture. A private value that showed while the capture ran
/// throws the pixels away; a page that moved to another document meanwhile drops the capture,
/// which neither reading covered.
#[tokio::test]
async fn a_capture_is_kept_only_if_both_readings_are_clear_on_one_document() -> TestResult {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    for (second, what) in [
        (
            neo_http::PictureVerdict::Private,
            "a value shown during the capture",
        ),
        (
            neo_http::PictureVerdict::Clear {
                reading: "another document".to_owned(),
            },
            "a document change during the capture",
        ),
    ] {
        let h = harness_with_camera().await?;
        h.browser.open_tab(3);
        let asked = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&asked);
        h.service.arm_picture_check(Arc::new(move |_tab| {
            let first = count.fetch_add(1, Ordering::SeqCst) == 0;
            let answer = if first {
                neo_http::PictureVerdict::Clear {
                    reading: "doc".to_owned(),
                }
            } else {
                second.clone()
            };
            Box::pin(async move { answer })
        }));
        let (run, op) = write_run(&h, 3);
        let body = complete(&h, run).await?;
        assert_eq!(
            body["operations"][0]["hasScreenshot"], false,
            "{what}: {body}"
        );
        assert!(!file_of(&h, op).exists(), "{what}: nothing written");
        assert_eq!(h.browser.captures(), 1, "{what}: one capture, thrown away");
        assert_eq!(
            asked.load(Ordering::SeqCst),
            2,
            "{what}: read before and after"
        );
    }
    Ok(())
}
