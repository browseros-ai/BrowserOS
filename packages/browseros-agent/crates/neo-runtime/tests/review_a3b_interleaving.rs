//! One run ending must not cancel another live run's left-button drag.
#[expect(dead_code, reason = "shared browser helpers cover other suites")]
#[path = "support/browser.rs"]
mod browser;

use std::time::{Duration, Instant};

use browser::{Fixture, Tab};
use neo_runtime::{PageId, RunRequest, RunStatus, Runtime};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::time;
use tokio_util::sync::CancellationToken;

async fn wait(tab: &Tab, expression: &str) {
    let started = Instant::now();
    while tab.evaluate(expression).await != json!(true) {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "never ready: {expression}"
        );
        time::sleep(Duration::from_millis(10)).await;
    }
}

fn request(tab: &Tab, code: &str) -> RunRequest {
    RunRequest {
        page: PageId::new(tab.target_id.clone()),
        code: code.to_string(),
        timeout: Duration::from_secs(30),
        cancel: CancellationToken::new(),
        artifacts: None,
    }
}

async fn check(hold_right: bool) {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![(
                "/drag.html".into(),
                include_str!("../../neo-sdk/tests/pages/drag.html").into(),
            )],
            ..Fixture::default()
        },
        "/drag.html",
    )
    .await;
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("runtime: {error}"));
    let (finished, first_finished) = oneshot::channel();
    let first = async {
        wait(&tab, "window.__readyB === true").await;
        let holding = if hold_right {
            "try { await page.mouse.down({button:'right'}); } catch (_) {}"
        } else {
            ""
        };
        let code = format!(
            "{holding} await page.evaluate(() => {{ window.__readyA = true }}); await page.locator('#finishA').waitFor({{state:'attached'}}); return 'A'"
        );
        let outcome = runtime.run(request(&tab, &code)).await;
        finished
            .send(())
            .unwrap_or_else(|()| panic!("first finished"));
        outcome
    };
    let second = async {
        runtime
            .run(request(
                &tab,
                "
            await page.mouse.move(60,40);
            await page.mouse.down();
            await page.mouse.move(250,60,{steps:3});
            await page.evaluate(() => { window.__readyB = true });
            await page.locator('#finishB').waitFor({state:'attached'});
            await page.mouse.up(); return 'B';
        ",
            ))
            .await
    };
    let observe = async {
        wait(&tab, "window.__readyA === true").await;
        let starts = tab
            .evaluate("window.__events.filter(e => e.type === 'dragstart').length")
            .await;
        tab.evaluate(
            "document.body.insertAdjacentHTML('beforeend','<span id=finishA></span>'); true",
        )
        .await;
        first_finished
            .await
            .unwrap_or_else(|error| panic!("first drained: {error}"));
        let ended_early = tab
            .evaluate("window.__events.filter(e => e.type === 'dragend').length")
            .await;
        tab.evaluate(
            "document.body.insertAdjacentHTML('beforeend','<span id=finishB></span>'); true",
        )
        .await;
        (starts, ended_early)
    };
    let (first, second, (starts, ended_early)) = tokio::join!(first, second, observe);
    let events: Value = tab.evaluate("window.__events").await;
    tab.close().await;
    assert_eq!(first.status, RunStatus::Ok, "{first:?}");
    assert_eq!(second.status, RunStatus::Ok, "{second:?}");
    assert_eq!(starts, json!(1), "a real drag started: {events}");
    assert_eq!(
        ended_early,
        json!(0),
        "A ended B's live drag; hold_right={hold_right}: {events}"
    );
    assert!(
        events
            .as_array()
            .unwrap_or_else(|| panic!("events"))
            .iter()
            .any(|event| event["type"] == "drop"),
        "B must still drop: {events}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn review_a3b_read_only_run_end_preserves_another_runs_drag() {
    check(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn review_a3b_right_button_owner_end_preserves_another_runs_drag() {
    check(true).await;
}
