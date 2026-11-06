//! Multi-page routing and session ownership against the fake browser, including late replies.

#[path = "support/fake_browser.rs"]
#[expect(dead_code, reason = "shared fake also supports click lifecycle tests")]
mod fake_browser;

use std::time::{Duration, Instant};

use fake_browser::{FakeBrowser, Replies, Reply};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::time;
use tokio_util::sync::CancellationToken;

const TWO_PAGES: &str = r"
    async function call(guid, method, params = {}) {
      return JSON.parse(await __neo_runtime.call(JSON.stringify({id:1,guid,method,params,metadata:{timeout:0}})));
    }
    const a = await call('context@default', 'newPage');
    const b = await call('context@default', 'newPage');
    if (a.error || b.error) throw new Error(JSON.stringify([a,b]));
";

async fn run(browser: &FakeBrowser, code: &str) -> RunOutcome {
    browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            code,
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await
}

async fn detached(browser: &FakeBrowser, count: usize) {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let exchanges = browser.exchanges();
        let mut attached: Vec<_> = exchanges
            .iter()
            .filter(|e| e.method() == "Target.attachToTarget")
            .filter_map(|e| e.reply.as_ref()?["result"]["sessionId"].as_str())
            .collect();
        let mut detached: Vec<_> = exchanges
            .iter()
            .filter(|e| e.method() == "Target.detachFromTarget")
            .filter_map(|e| e.command["params"]["sessionId"].as_str())
            .collect();
        attached.sort_unstable();
        detached.sort_unstable();
        if attached.len() == count && attached == detached {
            return;
        }
        assert!(
            Instant::now() < until,
            "sessions not detached: {exchanges:#?}"
        );
        time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_finished_multi_page_runs_detach_every_session() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let outcome = run(
        &browser,
        &format!("{TWO_PAGES} return [a.result.page.guid,b.result.page.guid];"),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value,
        Some(json!(["page@new-page-1", "page@new-page-2"]))
    );
    detached(&browser, 3).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_timed_out_multi_page_runs_detach_every_session() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            &format!("{TWO_PAGES} await new Promise(() => {{}});"),
            Duration::from_millis(500),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Timeout, "{outcome:?}");
    detached(&browser, 3).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_cancelled_multi_page_runs_detach_every_session() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let canceller = tokio::spawn(async move {
        time::sleep(Duration::from_millis(500)).await;
        trigger.cancel();
    });
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            &format!("{TWO_PAGES} await new Promise(() => {{}});"),
            Duration::from_secs(5),
            cancel,
        ))
        .await;
    canceller.await.unwrap();
    assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:?}");
    detached(&browser, 3).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_new_page_attaches_answered_after_timeout_are_detached() {
    let browser = FakeBrowser::start(Replies {
        new_page_attach: Some(Reply {
            delay: Duration::from_secs(2),
            ..Reply::default()
        }),
        ..Replies::default()
    })
    .await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            TWO_PAGES,
            Duration::from_millis(250),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Timeout, "{outcome:?}");
    detached(&browser, 2).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_unknown_page_and_frame_guids_leave_target_closed_records() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let outcome = run(
        &browser,
        r"
      for (const guid of ['page@unknown', 'frame@unknown'])
        await __neo_runtime.call(JSON.stringify({id:1,guid,method:'bringToFront',params:{}}));
    ",
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.calls.len(), 2);
    for call in &outcome.calls {
        assert_eq!(
            serde_json::to_value(&call.result).unwrap()["name"],
            "TargetClosedError"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_pages_are_scoped_lazy_and_outcomes_keep_first_seen_order_and_flags() {
    let browser = FakeBrowser::start(Replies::default()).await;
    add_tab(&browser, "other-window", 2, Some("profile"));
    add_tab(&browser, "other-profile", 1, Some("another-profile"));
    let outcome = run(
        &browser,
        &format!(
            r"{TWO_PAGES}
      const get = (guid,field) => JSON.parse(__neo_runtime.get(JSON.stringify({{guid,field}})));
      const pages = get('context@default', 'pages');
      const closed = await call(a.result.page.guid, 'close');
      const again = await call(a.result.page.guid, 'close');
      const still = get('context@default', 'pages');
      return {{pages:pages.map(p=>p.guid), still:still.map(p=>p.guid), closed, again,
               isClosed:get(a.result.page.guid,'isClosed'), url:get(a.result.page.guid,'url')}};
    "
        ),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value,
        Some(json!({
            "pages":["page@fake-page","page@new-page-1","page@new-page-2","page@other-window"],
            "still":["page@fake-page","page@new-page-2","page@other-window"],
            "closed":{"id":1}, "again":{"id":1}, "isClosed":true, "url":null,
        }))
    );
    let value: Value = serde_json::to_value(&outcome).unwrap();
    assert_eq!(
        value["pages"],
        json!([
            {"id":"fake-page","created":false,"closed":false},
            {"id":"new-page-1","created":true,"closed":true},
            {"id":"new-page-2","created":true,"closed":false},
            {"id":"other-window","created":false,"closed":false},
        ])
    );
    detached(&browser, 3).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_new_pages_wait_for_the_initial_blank_document() {
    let browser = FakeBrowser::start(Replies {
        new_page_empty_history: 3,
        ..Replies::default()
    })
    .await;
    let outcome = run(
        &browser,
        &format!(
            r"{TWO_PAGES}
      return JSON.parse(__neo_runtime.get(JSON.stringify({{guid:a.result.page.guid,field:'url'}})));
    "
        ),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!("about:blank")));
    detached(&browser, 3).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_new_page_accepts_a_committed_navigation() {
    let browser = FakeBrowser::start(Replies {
        new_page_url: Some("https://example.test/redirected"),
        ..Replies::default()
    })
    .await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            "const created = await context.newPage(); return created.url();",
            Duration::from_millis(500),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(
        outcome.status,
        RunStatus::Ok,
        "newPage waited for a URL it can no longer reach: {outcome:?}"
    );
    assert_eq!(
        outcome.value,
        Some(json!("https://example.test/redirected"))
    );
    detached(&browser, 2).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_beforeunload_ack_does_not_claim_the_tab_closed() {
    // The fake acknowledges Page.close but keeps the tab open.
    let browser = FakeBrowser::start(Replies::default()).await;
    let outcome = run(&browser, "const created = await context.newPage(); await created.close({runBeforeUnload:true}); return created.isClosed();").await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value,
        Some(json!(false)),
        "the browser still has the tab"
    );
    assert!(
        !outcome
            .pages
            .iter()
            .find(|page| page.created)
            .unwrap()
            .closed,
        "an acknowledgement is not a closure: {outcome:?}"
    );
    detached(&browser, 2).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_dropping_a_multi_page_run_detaches_every_session() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let request = fake_browser::run_request(
        &format!("{TWO_PAGES} await new Promise(() => {{}});"),
        Duration::from_secs(30),
        CancellationToken::new(),
    );
    let task = tokio::spawn(async move { runtime.run(request).await });
    let until = Instant::now() + Duration::from_secs(5);
    while browser
        .exchanges()
        .iter()
        .filter(|e| e.method() == "Target.attachToTarget" && e.reply.is_some())
        .count()
        < 3
    {
        assert!(Instant::now() < until, "new pages never attached");
        time::sleep(Duration::from_millis(10)).await;
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    detached(&browser, 3).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_missing_context_identity_scopes_pages_to_the_original_window() {
    let browser = FakeBrowser::start(Replies::default()).await;
    set_context(&browser, fake_browser::PAGE, None);
    add_tab(&browser, "same-window", 1, None);
    add_tab(&browser, "other-window", 2, None);
    let outcome = run(&browser, r"
      return JSON.parse(__neo_runtime.get(JSON.stringify({guid:'context@default',field:'pages'}))).map(p=>p.guid);
    ").await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value,
        Some(json!(["page@fake-page", "page@same-window"]))
    );
    detached(&browser, 1).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_known_tab_closed_elsewhere_can_be_closed_again() {
    let browser = FakeBrowser::start(Replies::default()).await;
    add_tab(&browser, "gone", 1, Some("profile"));
    let runtime = browser.runtime().await;
    let request = fake_browser::run_request(
        r"
      JSON.parse(__neo_runtime.get(JSON.stringify({guid:'context@default',field:'pages'})));
      console.log('listed');
      await new Promise(r=>setTimeout(r,200));
      return JSON.parse(await __neo_runtime.call(JSON.stringify({id:1,guid:'page@gone',method:'close',params:{}})));
    ",
        Duration::from_secs(5),
        CancellationToken::new(),
    );
    let task = tokio::spawn(async move { runtime.run(request).await });
    let until = Instant::now() + Duration::from_secs(5);
    while !browser
        .exchanges()
        .iter()
        .any(|e| e.method() == "Browser.getTabs")
    {
        assert!(Instant::now() < until, "pages getter never ran");
        time::sleep(Duration::from_millis(5)).await;
    }
    remove_tab(&browser, "gone");
    let outcome = task.await.unwrap();
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!({"id":1})));
    assert_eq!(outcome.calls[0].result, neo_runtime::CallResult::Ok);
    assert!(
        !outcome
            .pages
            .iter()
            .find(|p| p.id == "gone")
            .unwrap()
            .closed
    );
    detached(&browser, 1).await;
}

fn add_tab(browser: &FakeBrowser, id: &str, window: i64, context: Option<&str>) {
    browser
        .page
        .lock()
        .unwrap_or_else(|error| panic!("lock tabs: {error}"))
        .tabs
        .push(json!({
            "targetId":id, "url":fake_browser::PAGE_URL, "windowId":window,
            "liveBrowserContextId":context, "isActive":false,
        }));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_an_unprepared_attached_page_closed_externally_reports_target_closed() {
    let browser = FakeBrowser::start(Replies::default()).await;
    add_tab(&browser, "gone", 1, Some("profile"));
    let runtime = browser.runtime().await;
    let request = fake_browser::run_request(
        r"
      JSON.parse(__neo_runtime.get(JSON.stringify({guid:'context@default',field:'pages'})));
      const msg = JSON.stringify({id:1,guid:'page@gone',method:'bringToFront',params:{}});
      const first = JSON.parse(await __neo_runtime.call(msg));
      if (first.error) throw new Error(JSON.stringify(first));
      await new Promise(r=>setTimeout(r,200));
      return JSON.parse(await __neo_runtime.call(msg));
    ",
        Duration::from_secs(5),
        CancellationToken::new(),
    );
    let task = tokio::spawn(async move { runtime.run(request).await });
    let until = Instant::now() + Duration::from_secs(5);
    while !browser
        .exchanges()
        .iter()
        .any(|e| e.method() == "Browser.activateTab" && e.reply.is_some())
    {
        assert!(Instant::now() < until, "bringToFront never ran");
        time::sleep(Duration::from_millis(5)).await;
    }
    remove_tab(&browser, "gone");
    let outcome = task.await.unwrap();
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value.unwrap()["error"]["error"]["name"],
        "TargetClosedError"
    );
    detached(&browser, 2).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_run_before_unload_keeps_its_watcher_after_the_script_returns() {
    let browser = FakeBrowser::start(Replies {
        before_unload_delay: Some(Duration::from_millis(100)),
        ..Replies::default()
    })
    .await;
    let outcome = run(
        &browser,
        &format!(
            r"{TWO_PAGES}
      const reply = await call(a.result.page.guid, 'runBeforeUnload');
      if (reply.error) throw new Error(JSON.stringify(reply));
    "
        ),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert!(
        !browser
            .page
            .lock()
            .unwrap()
            .tabs
            .iter()
            .any(|tab| tab["targetId"] == "new-page-1"),
        "the watcher must survive until the delayed beforeunload completes"
    );
    assert!(
        outcome
            .pages
            .iter()
            .find(|page| page.id == "new-page-1")
            .unwrap()
            .closed,
        "an observed closure is reported: {outcome:?}"
    );
    detached(&browser, 3).await;
}

fn remove_tab(browser: &FakeBrowser, id: &str) {
    browser
        .page
        .lock()
        .unwrap_or_else(|error| panic!("lock tabs: {error}"))
        .tabs
        .retain(|tab| tab["targetId"] != id);
}

fn set_context(browser: &FakeBrowser, id: &str, context: Option<&str>) {
    if let Some(tab) = browser
        .page
        .lock()
        .unwrap_or_else(|error| panic!("lock tabs: {error}"))
        .tabs
        .iter_mut()
        .find(|tab| tab["targetId"] == id)
    {
        tab["liveBrowserContextId"] = json!(context);
    }
}
