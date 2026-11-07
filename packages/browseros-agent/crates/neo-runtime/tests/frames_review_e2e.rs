//! Review findings on Neo's frame support, each a value Playwright 26137face gives on the same
//! page: a direct `Frame` of a hidden iframe starts visible, `fill` and `selectOption` through a
//! hop into a hidden iframe wait for visibility, frame getters do not throw while unrelated
//! out-of-process iframes come and go, and they keep the committed URL while a top-level
//! navigation is in flight.

#[expect(
    dead_code,
    reason = "shared browser fixture includes helpers used by other suites"
)]
#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::json;

async fn run(tab: &Tab, source: &str) -> RunOutcome {
    tab.start_run(source.into())
        .await
        .await
        .unwrap_or_else(|error| panic!("run task: {error}"))
}

/// Playwright resolves a selector from a direct `Frame` with the receiver frame visible: only
/// iframes entered by a `frameLocator` hop count (`frameSelectors.ts` `_resolveChainedSelector`,
/// `startFrameVisible = true`). The element is visible in its own document, so `isVisible` is
/// `true` there (`isVisible.test.mjs`, `frameOfHiddenIframe`).
#[tokio::test(flavor = "multi_thread")]
async fn a_direct_frame_of_a_hidden_iframe_starts_visible() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![
                (
                    "/top.html".into(),
                    r#"<iframe name=hid style="visibility:hidden" src=/child.html></iframe>"#
                        .into(),
                ),
                ("/child.html".into(), "<input id=e value=seed>".into()),
            ],
            ..Fixture::default()
        },
        "/top.html",
    )
    .await;
    let outcome = run(
        &tab,
        r"
        let frame = null;
        const deadline = Date.now() + 5000;
        while (Date.now() < deadline && !frame?.url().endsWith('/child.html')) {
            frame = page.frame('hid');
            await new Promise(resolve => setTimeout(resolve, 20));
        }
        return await frame.isVisible('#e');
    ",
    )
    .await;
    tab.close().await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    assert_eq!(outcome.value, Some(json!(true)), "{outcome:#?}");
}

/// Through a `frameLocator` hop into a hidden iframe, Playwright's `fill` and `selectOption` wait
/// for visibility with `frameVisible` false and time out (`dom.ts` `_fill`, `_selectOption`);
/// the page keeps its values.
#[tokio::test(flavor = "multi_thread")]
async fn fill_and_select_through_a_hop_wait_for_a_hidden_iframe() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![
                (
                    "/top.html".into(),
                    r#"<iframe id=hid style="visibility:hidden" src=/child.html></iframe>"#.into(),
                ),
                (
                    "/child.html".into(),
                    "<input id=e value=seed><select id=s><option>a</option><option>b</option></select>"
                        .into(),
                ),
            ],
            ..Fixture::default()
        },
        "/top.html",
    )
    .await;
    let outcome = run(
        &tab,
        r"
        const child = page.frameLocator('#hid');
        await child.locator('#e').waitFor({state: 'attached'});
        const settle = promise => promise.then(() => 'done', error => error.name);
        return {
            fill: await settle(child.locator('#e').fill('typed', {timeout: 500})),
            select: await settle(child.locator('#s').selectOption('b', {timeout: 500})),
        };
    ",
    )
    .await;
    let page = tab
        .evaluate("[...['#e', '#s']].map(selector => document.querySelector('#hid').contentDocument.querySelector(selector).value)")
        .await;
    tab.close().await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    assert_eq!(
        (outcome.value, page),
        (
            Some(json!({"fill": "TimeoutError", "select": "TimeoutError"})),
            json!(["seed", "a"])
        ),
    );
}

/// An ad slot: cross-site iframes created every 15 ms and removed after 40 ms. Playwright's
/// frame getters read its frame tree in memory and never throw; none of these calls may throw.
#[tokio::test(flavor = "multi_thread")]
async fn frame_getters_survive_unrelated_oopif_churn() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![
                (
                    "/top.html".into(),
                    r"<iframe name=stable id=stable></iframe><script>
                    const cross = 'http://localhost:' + location.port + '/child.html';
                    document.getElementById('stable').src = cross;
                    setInterval(() => {
                        const ad = document.createElement('iframe');
                        ad.src = cross;
                        document.body.append(ad);
                        setTimeout(() => ad.remove(), 40);
                    }, 15);
                    </script>"
                        .into(),
                ),
                ("/child.html".into(), "<p>child</p>".into()),
            ],
            ..Fixture::default()
        },
        "/top.html",
    )
    .await;
    let outcome = run(
        &tab,
        r"
        const thrown = [];
        let calls = 0;
        const deadline = Date.now() + 3000;
        while (Date.now() < deadline) {
            calls++;
            try {
                const stable = page.frames().find(frame => frame.name() === 'stable');
                if (stable) stable.isDetached();
            } catch (error) {
                thrown.push(error.message);
            }
        }
        return {calls: calls > 0, thrown: [...new Set(thrown)]};
    ",
    )
    .await;
    tab.close().await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    assert_eq!(
        outcome.value,
        Some(json!({"calls": true, "thrown": []})),
        "{outcome:#?}"
    );
}

/// Playwright's `page.url()` is `mainFrame().url()`, and both keep the committed URL until a
/// navigation commits. Neo's `page.url()` does (navigation history); the frame getters must agree.
/// Neo's tree enumeration can wait until commit, so compare its URLs with the committed page
/// URL when enumeration returns, without requiring that enumeration finish before commit.
#[tokio::test(flavor = "multi_thread")]
async fn frame_getters_keep_the_committed_url_while_a_navigation_is_pending() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            slow_response: Duration::from_secs(3),
            pages: vec![("/start.html".into(), "<a id=go href=/slow>slow</a>".into())],
        },
        "/start.html",
    )
    .await;
    let start = format!("{}/start.html", tab.origin);
    let outcome = run(
        &tab,
        r"
        await page.locator('#go').click({noWaitAfter: true});
        const pageUrl = page.url();
        const mainUrl = page.mainFrame().url();
        const frames = page.frames().map(frame => frame.url());
        return {pageUrl, mainUrl, frames, afterFrames: page.url()};
    ",
    )
    .await;
    tab.close().await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    let values = outcome
        .value
        .as_ref()
        .expect("the script returns URL observations");
    assert_eq!(values["pageUrl"], start, "{outcome:#?}");
    assert_eq!(values["mainUrl"], start, "{outcome:#?}");
    assert_eq!(
        values["frames"],
        json!([values["afterFrames"]]),
        "{outcome:#?}"
    );
}
