//! New pages on a real BrowserOS neo (`src/chromium_bridge/discovery.rs`): a popup reaches its listeners and
//! its id serves the next run; target discovery stays on while any run of a `Runtime` listens
//! and is off once the last lets go; a run that listened leaves no session on the pages it was
//! handed.
//!
//! The fixture's `Runtime` shares the test's CDP client, so the test sees that connection's
//! `Target.targetCreated` events exactly when the runs' discovery is on.
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the test drives an independent CDP session as the browser oracle"
)]

#[expect(
    dead_code,
    reason = "the shared fixture also provides navigation helpers"
)]
#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use browseros_cdp::CdpEvent;
use neo_runtime::{PageId, RunOutcome, RunRequest, RunStatus, Runtime, StartPage};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

const OPENER: &str = r#"<!doctype html><html><body>
<button id="open" onclick="window.open('/popup.html')">open</button>
</body></html>"#;

const POPUP: &str =
    "<!doctype html><html><head><title>popup</title></head><body>popup</body></html>";

/// How long the test listens for a `Target.targetCreated` that must not come.
const QUIET: Duration = Duration::from_secs(1);

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: vec![
                    ("/opener.html".into(), OPENER.into()),
                    ("/popup.html".into(), POPUP.into()),
                ],
                ..Fixture::default()
            },
            "/opener.html",
        )
        .await,
    )
}

async fn run(runtime: &Runtime, page: &str, code: &str) -> RunOutcome {
    runtime
        .run(RunRequest {
            code: code.to_string(),
            page: StartPage::Page(PageId::new(page.to_string())),
            timeout: Duration::from_secs(30),
            cancel: CancellationToken::new(),
            artifacts: None,
        })
        .await
}

fn value(outcome: &RunOutcome) -> &Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    outcome.value.as_ref().unwrap_or(&Value::Null)
}

/// Opens an `about:blank` tab with the test's own client and returns its target id.
async fn new_tab(tab: &Tab) -> String {
    let created = tab
        .cdp
        .send("Target.createTarget", json!({ "url": "about:blank" }), None)
        .await
        .unwrap_or_else(|error| panic!("open a tab: {error}"));
    created["targetId"]
        .as_str()
        .unwrap_or_else(|| panic!("Target.createTarget answered {created}"))
        .to_string()
}

/// Whether `events` reports `Target.targetCreated` for `target` within `within`.
async fn reports_created(
    events: &mut broadcast::Receiver<CdpEvent>,
    target: &str,
    within: Duration,
) -> bool {
    let deadline = Instant::now() + within;
    loop {
        let Ok(event) = time::timeout_at(deadline, events.recv()).await else {
            return false;
        };
        let Ok(event) = event else {
            continue;
        };
        if event.method == "Target.targetCreated"
            && event.params["targetInfo"]["targetId"].as_str() == Some(target)
        {
            return true;
        }
    }
}

/// Waits until `events` reports any `Target.targetCreated`: discovery's replay when it turns on.
async fn discovery_turned_on(events: &mut broadcast::Receiver<CdpEvent>) {
    time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(event) = events.recv().await
                && event.method == "Target.targetCreated"
            {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the run turned discovery on"));
}

/// Whether any CDP client is attached to `target`, polled until it is not, for a session the
/// run detaches in the background after its outcome.
async fn left_attached(tab: &Tab, target: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let targets = tab
            .cdp
            .send("Target.getTargets", json!({}), None)
            .await
            .unwrap_or_else(|error| panic!("Target.getTargets: {error}"));
        let attached = targets["targetInfos"]
            .as_array()
            .unwrap_or_else(|| panic!("Target.getTargets answered {targets}"))
            .iter()
            .any(|info| info["targetId"] == target && info["attached"] == true);
        if !attached || Instant::now() > deadline {
            return attached;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_popup_reaches_its_listeners_and_its_id_serves_the_next_run() {
    let Some(tab) = open().await else {
        return;
    };
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    let first = run(
        &runtime,
        &tab.target_id,
        r"const path = url => url.replace(/^[a-z]+:\/\/[^/]+/, '')
const [popup] = await Promise.all([page.waitForEvent('popup'), page.click('#open')])
return {
  url: path(popup.url()),
  openerIsPage: (await popup.opener()) === page,
  inPages: context.pages().includes(popup),
  title: await popup.title(),
}",
    )
    .await;
    assert_eq!(
        value(&first),
        &json!({ "url": "/popup.html", "openerIsPage": true, "inPages": true, "title": "popup" })
    );
    let popup = first
        .pages
        .iter()
        .find(|page| page.id != tab.target_id)
        .unwrap_or_else(|| panic!("the popup is in the outcome: {:#?}", first.pages))
        .clone();
    assert!(
        !popup.created && !popup.closed,
        "the page opened it, and it stays open: {popup:?}"
    );

    let mut events = tab.cdp.events();
    let after = new_tab(&tab).await;
    assert!(
        !reports_created(&mut events, &after, QUIET).await,
        "the run turned discovery off when it ended"
    );
    assert!(
        !left_attached(&tab, &popup.id).await,
        "the run left no session on the popup"
    );

    let second = run(
        &runtime,
        &popup.id,
        r"const path = url => url.replace(/^[a-z]+:\/\/[^/]+/, '')
const opener = await page.opener()
return { url: path(page.url()), opener: opener && path(opener.url()) }",
    )
    .await;
    assert_eq!(
        value(&second),
        &json!({ "url": "/popup.html", "opener": "/opener.html" }),
        "the popup's id starts the next run, which still names its opener"
    );
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_discovery_stays_on_until_the_runtimes_last_listening_run_ends() {
    let Some(tab) = open().await else {
        return;
    };
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    let mut events = tab.cdp.events();
    // Listens until a new tab arrives.
    let holding = {
        let runtime = runtime.clone();
        let page = tab.target_id.clone();
        tokio::spawn(async move {
            run(
                &runtime,
                &page,
                r"const urls = []
context.on('page', p => urls.push(p.url()))
while (!urls.length) await new Promise(resolve => setTimeout(resolve, 20))
return urls",
            )
            .await
        })
    };
    discovery_turned_on(&mut events).await;
    // A second listener of the same Runtime comes and goes while the first still listens.
    let passing = run(
        &runtime,
        &tab.target_id,
        r"context.on('page', () => {})
await page.title()
return 'done'",
    )
    .await;
    assert_eq!(value(&passing), &json!("done"));

    let opened = new_tab(&tab).await;
    let held = time::timeout(Duration::from_secs(20), holding)
        .await
        .unwrap_or_else(|_| panic!("the first run saw the new tab"))
        .unwrap_or_else(|error| panic!("run task: {error}"));
    assert_eq!(
        value(&held),
        &json!(["about:blank"]),
        "discovery outlived the run that let go first"
    );
    assert!(
        !left_attached(&tab, &opened).await,
        "the run left no session on the tab it was handed"
    );

    let mut events = tab.cdp.events();
    let after = new_tab(&tab).await;
    assert!(
        !reports_created(&mut events, &after, QUIET).await,
        "discovery is off once the last listening run ended"
    );
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_closing_the_context_lets_go_at_once_and_leaves_every_tab_open() {
    let Some(tab) = open().await else {
        return;
    };
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    let mut events = tab.cdp.events();
    let running = {
        let runtime = runtime.clone();
        let page = tab.target_id.clone();
        tokio::spawn(async move {
            run(
                &runtime,
                &page,
                r"context.on('page', () => {})
const other = await context.newPage()
await context.close({ reason: 'done here' })
const after = { closed: [context.isClosed(), page.isClosed(), other.isClosed()], pages: context.pages().length }
await new Promise(resolve => setTimeout(resolve, 4000))
return after",
            )
            .await
        })
    };
    discovery_turned_on(&mut events).await;
    // Discovery ends with the close, while the script still runs.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut off = false;
    while !off && Instant::now() < deadline {
        let mut probe = tab.cdp.events();
        let opened = new_tab(&tab).await;
        off = !reports_created(&mut probe, &opened, Duration::from_millis(300)).await;
    }
    assert!(off, "the context's close turned discovery off");
    assert!(!running.is_finished(), "before the run ended");
    let outcome = time::timeout(Duration::from_secs(20), running)
        .await
        .unwrap_or_else(|_| panic!("the run ended"))
        .unwrap_or_else(|error| panic!("run task: {error}"));
    assert_eq!(
        value(&outcome),
        &json!({ "closed": [true, true, true], "pages": 0 })
    );
    assert!(
        outcome.pages.iter().all(|page| !page.closed),
        "the run closed no tab: {:#?}",
        outcome.pages
    );
    let targets = tab
        .cdp
        .send("Target.getTargets", json!({}), None)
        .await
        .unwrap_or_else(|error| panic!("Target.getTargets: {error}"));
    for page in &outcome.pages {
        assert!(
            targets["targetInfos"]
                .as_array()
                .unwrap_or_else(|| panic!("Target.getTargets answered {targets}"))
                .iter()
                .any(|info| info["targetId"] == page.id.as_str()),
            "tab {} is still open",
            page.id
        );
    }
    let next = run(&runtime, &tab.target_id, "return await page.title()").await;
    assert_eq!(value(&next), &json!(""), "the next run uses the same tab");
    tab.close().await;
}
