//! Creation cancellation, registry races, and shared input turns against the fake browser.

#[path = "support/fake_browser.rs"]
mod fake_browser;

use std::time::{Duration, Instant};

use fake_browser::{FakeBrowser, Replies};
use neo_runtime::{RunStatus, Runtime};
use serde_json::json;
use tokio::time;
use tokio_util::sync::CancellationToken;

const WIRE: &str = r"
  let next = 0;
  function call(guid, method, params = {}) {
    return __neo_runtime.call(JSON.stringify({id:++next,guid,method,params,metadata:{timeout:0}})).then(JSON.parse);
  }
  const get = (guid, field) => JSON.parse(__neo_runtime.get(JSON.stringify({guid,field})));
  const sleep = ms => new Promise(r => setTimeout(r, ms));
";

fn tab_ids(browser: &FakeBrowser) -> Vec<String> {
    browser
        .page
        .lock()
        .unwrap_or_else(|error| panic!("lock fake tabs: {error}"))
        .tabs
        .iter()
        .map(|tab| tab["targetId"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// F1a: the run's deadline lands while `Browser.createTab` is answering. The tab exists, but
/// the outcome never names it, so the host cannot close or reuse it.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_tab_created_as_the_deadline_lands_is_in_the_outcome() {
    let browser = FakeBrowser::start(Replies {
        create_tab_delay: Some(Duration::from_millis(600)),
        ..Replies::default()
    })
    .await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            &format!("{WIRE} await call('context@default','newPage');"),
            Duration::from_millis(300),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Timeout, "{outcome:?}");
    time::sleep(Duration::from_millis(500)).await;
    let tabs = tab_ids(&browser);
    let pages: Vec<_> = outcome.pages.iter().map(|p| p.id.clone()).collect();
    assert!(
        pages.contains(&"new-page-1".to_string()),
        "outcome.pages must name every tab this run created; got {pages:?}, browser has {tabs:?}"
    );
}

/// F1b: same, when the script returns without awaiting `context.newPage()`.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_tab_created_by_an_unawaited_new_page_is_in_the_outcome() {
    let browser = FakeBrowser::start(Replies {
        create_tab_delay: Some(Duration::from_millis(600)),
        ..Replies::default()
    })
    .await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            &format!("{WIRE} call('context@default','newPage'); await sleep(150); return 1;"),
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    time::sleep(Duration::from_millis(800)).await;
    let tabs = tab_ids(&browser);
    let pages: Vec<_> = outcome.pages.iter().map(|p| p.id.clone()).collect();
    assert!(
        pages.contains(&"new-page-1".to_string()),
        "a tab the run created is missing from outcome.pages; got {pages:?}, browser has {tabs:?}"
    );
}

/// F2: `context.pages()` read while `newPage` is answering registers the new tab first, as not
/// created, and `newPage` reuses that entry: the outcome reports `created: false`.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_created_survives_a_pages_read_during_new_page() {
    let browser = FakeBrowser::start(Replies {
        create_tab_delay: Some(Duration::from_millis(300)),
        ..Replies::default()
    })
    .await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            &format!(
                "{WIRE}
                 const pending = call('context@default','newPage');
                 await sleep(100);
                 const listed = get('context@default','pages').map(p => p.guid);
                 const created = await pending;
                 return {{listed, created}};"
            ),
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    let value = outcome.value.clone().unwrap();
    assert_eq!(
        value["created"]["result"]["page"]["guid"],
        "page@new-page-1"
    );
    assert!(
        value["listed"]
            .as_array()
            .unwrap()
            .contains(&json!("page@new-page-1")),
        "precondition: the listing saw the tab mid-create: {value}"
    );
    let page = outcome.pages.iter().find(|p| p.id == "new-page-1").unwrap();
    assert!(
        page.created,
        "newPage created this tab, but outcome.pages says created=false: {:?}",
        outcome.pages
    );
}

/// Check: fd6f70822's per-tab turn holds for a page a run reached through `pages()` (a lazily
/// attached second `PageSession`) while another run's trial is armed on that tab.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_listed_page_forced_click_waits_for_another_runs_trial() {
    let browser = FakeBrowser::start(Replies {
        arm: fake_browser::Reply {
            delay: Duration::from_millis(500),
            ..fake_browser::Reply::default()
        },
        ..Replies::default()
    })
    .await;
    browser
        .page
        .lock()
        .unwrap()
        .tabs
        .push(fake_browser::tab_record(
            "other",
            fake_browser::PAGE_URL,
            1,
            Some("profile"),
            false,
        ));
    let runtime = browser.runtime().await;
    let trial = tokio::spawn({
        let runtime = runtime.clone();
        async move {
            runtime
                .run(fake_browser::run_request(
                    "await page.click('#go', { trial: true })",
                    Duration::from_secs(10),
                    CancellationToken::new(),
                ))
                .await
        }
    });
    let until = Instant::now() + Duration::from_secs(5);
    while browser.armed_interceptor().is_none() {
        assert!(Instant::now() < until, "trial never armed");
        time::sleep(Duration::from_millis(5)).await;
    }
    let forced = runtime
        .run(neo_runtime::RunRequest {
            code: format!(
                "{WIRE}
                 get('context@default','pages');
                 const r = await call('frame@{}', 'click', {{selector:'#go', force:true}});
                 if (r.error) throw new Error(JSON.stringify(r));
                 return 'clicked';",
                fake_browser::PAGE
            ),
            page: neo_runtime::PageId::new("other".to_string()),
            timeout: Duration::from_secs(10),
            cancel: CancellationToken::new(),
            artifacts: None,
        })
        .await;
    let trial = trial.await.unwrap();
    assert_eq!(trial.status, RunStatus::Ok, "{trial:?}");
    assert_eq!(forced.value, Some(json!("clicked")), "{forced:?}");
    let exchanges = browser.exchanges();
    let stop = exchanges.iter().position(|exchange| {
        exchange.command["params"]["arguments"][0]["value"] == "stopHitTargetInterceptor"
    });
    let press = exchanges.iter().position(|exchange| {
        exchange.method() == "Input.dispatchMouseEvent"
            && exchange.command["params"]["type"] == "mousePressed"
    });
    assert!(
        stop.is_some() && press > stop,
        "the forced click pressed only after the trial's stop: stop {stop:?}, press {press:?}"
    );
    assert_eq!(
        browser.page_clicks(),
        1,
        "the forced click waited for the trial's stop"
    );
}

/// A host can close its CDP link after drain even when the run returned before the tab id
/// was acknowledged. This is a shutdown boundary, separate from the outcome's short drain.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_runtime_drain_waits_for_late_creation_cleanup() {
    let browser = FakeBrowser::start(Replies {
        create_tab_delay: Some(Duration::from_secs(3)),
        ..Replies::default()
    })
    .await;
    let cdp = browser.cdp().await;
    let runtime = Runtime::connect(cdp.clone())
        .await
        .expect("connect runtime");
    let outcome = runtime
        .run(fake_browser::run_request(
            &format!("{WIRE} await call('context@default','newPage');"),
            Duration::from_millis(100),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Timeout);
    assert!(tab_ids(&browser).contains(&"new-page-1".to_string()));
    time::timeout(Duration::from_secs(5), runtime.drain())
        .await
        .expect("runtime cleanup did not drain");
    assert!(!tab_ids(&browser).contains(&"new-page-1".to_string()));
    cdp.disconnect().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_late_creation_does_not_extend_finish_and_is_closed() {
    let browser = FakeBrowser::start(Replies {
        create_tab_delay: Some(Duration::from_secs(3)),
        ..Replies::default()
    })
    .await;
    let runtime = browser.runtime().await;
    let started = Instant::now();
    let outcome = runtime
        .run(fake_browser::run_request(
            &format!("{WIRE} await call('context@default','newPage');"),
            Duration::from_millis(100),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Timeout, "{outcome:?}");
    assert!(
        started.elapsed() < Duration::from_millis(1600),
        "finish extended the 100ms run beyond its cleanup budget: {:?}",
        started.elapsed()
    );
    assert!(
        tab_ids(&browser).contains(&"new-page-1".to_string()),
        "the tab exists before its delayed acknowledgement"
    );
    assert!(
        !outcome.pages.iter().any(|page| page.id == "new-page-1"),
        "an unanswered creation has no known ID yet: {outcome:?}"
    );
    let until = Instant::now() + Duration::from_secs(5);
    while tab_ids(&browser).contains(&"new-page-1".to_string()) {
        assert!(Instant::now() < until, "the late unkept tab never closed");
        time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        browser
            .exchanges()
            .iter()
            .any(|exchange| exchange.method() == "Target.closeTarget"
                && exchange.command["params"]["targetId"] == "new-page-1"),
        "the owned creation task must close the late tab"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_cancellation_during_creation_records_and_closes_the_tab() {
    let browser = FakeBrowser::start(Replies {
        create_tab_delay: Some(Duration::from_millis(600)),
        ..Replies::default()
    })
    .await;
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let canceller = tokio::spawn(async move {
        time::sleep(Duration::from_millis(100)).await;
        trigger.cancel();
    });
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            &format!("{WIRE} await call('context@default','newPage');"),
            Duration::from_secs(5),
            cancel,
        ))
        .await;
    canceller.await.unwrap();
    assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:?}");
    assert!(
        outcome.pages.iter().any(|page| page.created && page.closed),
        "{outcome:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_failed_new_page_is_closed_after_creation() {
    let browser = FakeBrowser::start(Replies {
        new_page_empty_history: usize::MAX,
        ..Replies::default()
    })
    .await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            &format!("{WIRE} await call('context@default','newPage');"),
            Duration::from_millis(100),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Timeout, "{outcome:?}");
    assert!(
        !tab_ids(&browser).contains(&"new-page-1".to_string()),
        "failed readiness left its tab open"
    );
    assert!(
        outcome.pages.iter().any(|page| page.created && page.closed),
        "{outcome:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_close_ack_after_timeout_still_reports_a_gone_tab() {
    let browser = FakeBrowser::start(Replies {
        close_tab_delay: Some(Duration::from_millis(600)),
        ..Replies::default()
    })
    .await;
    let outcome = browser.runtime().await.run(fake_browser::run_request(
        &format!("{WIRE} const created=(await call('context@default','newPage')).result.page; await call(created.guid,'close');"),
        Duration::from_millis(150), CancellationToken::new(),
    )).await;
    assert_eq!(outcome.status, RunStatus::Timeout, "{outcome:?}");
    assert!(
        !tab_ids(&browser).contains(&"new-page-1".to_string()),
        "browser already closed the tab"
    );
    assert!(
        outcome.pages.iter().any(|page| page.created && page.closed),
        "close status was lost: {outcome:?}"
    );
}
