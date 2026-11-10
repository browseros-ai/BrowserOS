//! Tab lifecycle and per-target routing on a throwaway BrowserOS profile.
#![expect(
    clippy::disallowed_methods,
    reason = "the test drives an independent CDP session as the browser oracle"
)]

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "shared browser harness also supports navigation and click tests"
)]
mod browser;

use std::io::{self, Write};
use std::time::{Duration, Instant};

use browser::{Fixture, Tab};
use browseros_cdp::SessionId;
use neo_runtime::{CallResult, PageId, RunOutcome, RunRequest, RunStatus, Runtime, StartPage};
use serde_json::{Value, json};
use tokio::time;
use tokio_util::sync::CancellationToken;

const WIRE: &str = r"
  let next = 0;
  async function call(guid, method, params = {}) {
    const reply = JSON.parse(await __neo_runtime.call(JSON.stringify({id:++next,guid,method,params,metadata:{timeout:5000}})));
    if (reply.error) { const e = new Error(reply.error.error.message); e.name = reply.error.error.name; throw e; }
    return reply.result;
  }
  const get = (guid, field) => JSON.parse(__neo_runtime.get(JSON.stringify({guid,field})));
  const pages = () => get('context@default','pages');
";

async fn run(tab: &Tab, target: &str, code: &str) -> RunOutcome {
    tab.runtime()
        .await
        .run(RunRequest {
            code: format!("{WIRE}\n{code}"),
            page: StartPage::Page(PageId::new(target.to_string())),
            timeout: Duration::from_secs(15),
            cancel: CancellationToken::new(),
            artifacts: None,
        })
        .await
}

async fn info(tab: &Tab, target: &str) -> Value {
    tab.cdp
        .send("Browser.getTabInfo", json!({"targetId":target}), None)
        .await
        .unwrap_or_else(|error| panic!("browser command failed: {error}"))["tab"]
        .clone()
}

async fn create(tab: &Tab, count: usize) -> Vec<String> {
    let outcome = run(
        tab,
        &tab.target_id,
        &format!("for (let i=0;i<{count};i++) await call('context@default','newPage');"),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    let ids: Vec<_> = outcome
        .pages
        .into_iter()
        .filter(|p| p.created)
        .map(|p| p.id)
        .collect();
    assert_eq!(ids.len(), count, "one outcome ID per created page");
    ids
}

async fn driver(tab: &Tab, target: &str, body: &str) -> SessionId {
    let reply = tab
        .cdp
        .send(
            "Target.attachToTarget",
            json!({"targetId":target,"flatten":true}),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("browser command failed: {error}"));
    let session = SessionId::from(
        reply["sessionId"]
            .as_str()
            .unwrap_or_else(|| panic!("missing sessionId: {reply}")),
    );
    evaluate(tab, &session, &format!("document.body.innerHTML = {}; window.clicks = 0; document.querySelector('button').onclick = () => window.clicks++;", json!(body))).await;
    session
}

async fn evaluate(tab: &Tab, session: &SessionId, expression: &str) -> Value {
    let reply = tab
        .cdp
        .send(
            "Runtime.evaluate",
            json!({"expression":expression,"returnByValue":true}),
            Some(session),
        )
        .await
        .unwrap_or_else(|error| panic!("browser command failed: {error}"));
    assert!(reply.get("exceptionDetails").is_none(), "{reply}");
    reply["result"]["value"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_new_pages_open_in_background_with_distinct_initializers() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/start").await;
    tab.cdp
        .send(
            "Browser.activateTab",
            json!({"targetId":tab.target_id}),
            None,
        )
        .await
        .unwrap();
    let outcome = run(
        &tab,
        &tab.target_id,
        r"
      const a = (await call('context@default','newPage')).page;
      const b = (await call('context@default','newPage')).page;
      return {a:get(a.guid,'initializer'), b:get(b.guid,'initializer'), pages:pages()};
    ",
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    let value = outcome.value.unwrap();
    assert_ne!(value["a"]["guid"], value["b"]["guid"]);
    for entry in [&value["a"], &value["b"]] {
        assert_eq!(entry["initializer"]["isClosed"], false);
        assert_eq!(entry["mainFrame"]["initializer"]["url"], "about:blank");
        assert_eq!(
            entry["initializer"]["mainFrame"]["guid"],
            entry["mainFrame"]["guid"]
        );
        assert!(
            value["pages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|page| page["guid"] == entry["guid"])
        );
        let target = entry["guid"]
            .as_str()
            .unwrap()
            .strip_prefix("page@")
            .unwrap();
        assert_eq!(info(&tab, target).await["isActive"], false);
    }
    assert_eq!(info(&tab, &tab.target_id).await["isActive"], true);
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[expect(
    clippy::too_many_lines,
    reason = "one scenario across two runs; each RunRequest names its artifact directory"
)]
async fn tests_that_sdk_created_pages_keep_their_targets_across_runs() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/sdk-start").await;
    tab.cdp
        .send(
            "Browser.activateTab",
            json!({"targetId":tab.target_id}),
            None,
        )
        .await
        .unwrap();
    tab.evaluate("window.runClicks=0; document.documentElement.addEventListener('click',()=>window.runClicks++);").await;
    let original = info(&tab, &tab.target_id).await;
    let initial = tab
        .cdp
        .send("Browser.getTabs", json!({}), None)
        .await
        .unwrap();
    let initial_ids: Vec<_> = initial["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["targetId"].clone())
        .collect();
    let runtime = Runtime::connect(tab.cdp.clone()).await.unwrap();
    let request = RunRequest {
        code: r"
          const before = context.pages();
          const first = await context.newPage();
          const second = await context.newPage();
          const listed = context.pages();
          const distinct = first !== second;
          const bothListed = listed.includes(first) && listed.includes(second);
          await second.locator('#continue').click();
          await second.locator('html').click();
          await second.bringToFront();
          await first.close();
          return {beforeCount:before.length, distinct, bothListed, firstClosed:first.isClosed(),
                  secondListed:context.pages().includes(second), firstListed:context.pages().includes(first),
                  secondUrl:second.url()};
        ".to_string(),
        page: StartPage::Page(PageId::new(tab.target_id.clone())),
        timeout: Duration::from_secs(15),
        cancel: CancellationToken::new(),
        artifacts: None,
    };
    let running = tokio::spawn({
        let runtime = runtime.clone();
        async move { runtime.run(request).await }
    });
    // The missing selector holds the SDK flow while CDP observes both background tabs.
    let created = two_new_tabs(&tab, &initial_ids).await;
    assert!(
        created.iter().all(|p| p["isActive"] == false),
        "new tabs stole activation: {created:?}"
    );
    assert_eq!(info(&tab, &tab.target_id).await["isActive"], true);
    driver(
        &tab,
        created[1]["targetId"].as_str().unwrap(),
        "<button id='continue'>Continue</button>",
    )
    .await;
    let outcome = running.await.unwrap();
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value,
        Some(json!({
            "beforeCount":initial_ids.len(), "distinct":true, "bothListed":true,
            "firstClosed":true,"secondListed":true,"firstListed":false,"secondUrl":"about:blank",
        }))
    );
    let pages: Vec<_> = outcome.pages.iter().filter(|p| p.created).collect();
    assert_eq!(pages.len(), 2);
    let (first, second) = (pages[0], pages[1]);
    assert_eq!(created[0]["targetId"], first.id);
    assert_eq!(created[1]["targetId"], second.id);
    assert!(first.closed && !second.closed, "{pages:?}");
    assert_sdk_routes(&outcome, &first.id, &second.id);
    assert!(
        outcome.calls.iter().all(|c| c.result == CallResult::Ok),
        "{outcome:?}"
    );
    wait_gone(&tab, &first.id).await;
    assert_eq!(info(&tab, &second.id).await["isActive"], true);
    assert_eq!(info(&tab, &tab.target_id).await["url"], original["url"]);
    assert_eq!(tab.evaluate("window.runClicks").await, 0);

    let next=runtime.run(RunRequest {
        code:"await page.locator('html').click(); return {url:page.url(),closed:page.isClosed()};".to_string(),
        page:StartPage::Page(PageId::new(second.id.clone())), timeout:Duration::from_secs(10), cancel:CancellationToken::new(), artifacts:None,
    }).await;
    assert_eq!(next.status, RunStatus::Ok, "{next:?}");
    assert_eq!(
        next.value,
        Some(json!({"url":"about:blank","closed":false}))
    );
    assert_eq!(next.calls.len(), 1);
    assert_eq!(next.calls[0].guid, format!("frame@{}", second.id));
    assert_eq!(next.calls[0].result, CallResult::Ok);
    assert_eq!(tab.evaluate("window.runClicks").await, 0);
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_activation_close_and_a_later_run_keep_the_correct_tab() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/start").await;
    let ids = create(&tab, 2).await;
    let outcome = run(&tab, &tab.target_id, &format!(r"
      const all = pages();
      const a = all.find(p => p.guid === 'page@{a}');
      const b = all.find(p => p.guid === 'page@{b}');
      const front = await call(a.guid,'bringToFront');
      const close = await call(b.guid,'close');
      const twice = await call(b.guid,'close');
      return {{voids:front === undefined && close === undefined && twice === undefined,
               remaining:pages().map(p=>p.guid), closed:get(b.guid,'isClosed'), url:get(b.guid,'url')}};
    ", a=ids[0], b=ids[1])).await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    let value = outcome.value.unwrap();
    assert_eq!(value["voids"], true);
    assert_eq!(value["closed"], true);
    assert_eq!(value["url"], Value::Null);
    let remaining = value["remaining"].as_array().unwrap();
    assert!(remaining.contains(&json!(format!("page@{}", ids[0]))));
    assert!(!remaining.contains(&json!(format!("page@{}", ids[1]))));
    assert_eq!(info(&tab, &ids[0]).await["isActive"], true);
    assert!(outcome.pages.iter().any(|p| p.id == ids[1] && p.closed));

    let next = run(
        &tab,
        &ids[0],
        "return {url:page.url(),closed:page.isClosed()};",
    )
    .await;
    assert_eq!(next.status, RunStatus::Ok, "{next:?}");
    assert_eq!(next.value.as_ref().unwrap()["url"], "about:blank");
    assert_eq!(next.pages[0].id, ids[0]);
    let gone = run(&tab, &ids[1], "return 1").await;
    assert_eq!(gone.status, RunStatus::Error, "{gone:?}");
    assert_eq!(gone.error.unwrap().name, "TargetClosedError");
    tab.close().await;
}

/// A run with no page starts in a background tab of the active window, and a later run given
/// that tab's id continues where the first left it.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_new_tab_start_runs_in_the_background_and_a_later_run_continues_there() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/start").await;
    tab.cdp
        .send(
            "Browser.activateTab",
            json!({"targetId":tab.target_id}),
            None,
        )
        .await
        .unwrap();
    let runtime = Runtime::connect(tab.cdp.clone()).await.unwrap();
    let request = |code: String, page: StartPage| RunRequest {
        code,
        page,
        timeout: Duration::from_secs(15),
        cancel: CancellationToken::new(),
        artifacts: None,
    };
    let url = format!("{}/continued", tab.origin);
    let first = runtime
        .run(request(
            format!("await page.goto({}); return page.url();", json!(url)),
            StartPage::NewTab,
        ))
        .await;
    assert_eq!(first.status, RunStatus::Ok, "{first:?}");
    assert_eq!(first.value, Some(json!(url)));
    let started = &first.pages[0];
    assert!(started.created && !started.closed, "{first:?}");
    assert_eq!(
        (started.url.as_deref(), started.title.as_deref()),
        (Some(url.as_str()), Some("/continued")),
        "the page reports where the run left it"
    );
    let created = info(&tab, &started.id).await;
    let original = info(&tab, &tab.target_id).await;
    assert_eq!(
        created["isActive"], false,
        "the run's tab opened in the background"
    );
    assert_eq!(original["isActive"], true, "the user's tab stayed selected");
    assert_eq!(
        created["windowId"], original["windowId"],
        "the run's tab opened in the active window"
    );

    let next = runtime
        .run(request(
            "return page.url()".to_string(),
            StartPage::Page(PageId::new(started.id.clone())),
        ))
        .await;
    assert_eq!(next.status, RunStatus::Ok, "{next:?}");
    assert_eq!(next.value, Some(json!(url)));
    assert_eq!(next.pages[0].id, started.id);
    assert!(!next.pages[0].created, "{next:?}");
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_concurrent_clicks_only_reach_their_own_pages() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/start").await;
    let ids = create(&tab, 2).await;
    let a = driver(&tab, &ids[0], "<button id='a'>A</button>").await;
    let b = driver(&tab, &ids[1], "<button id='b'>B</button>").await;
    let outcome = run(
        &tab,
        &tab.target_id,
        &format!(
            r"
      const all = pages();
      const a = all.find(p=>p.guid==='page@{a}');
      const b = all.find(p=>p.guid==='page@{b}');
      for (let i=0;i<3;i++) await Promise.all([
        call(a.mainFrame.guid,'click',{{selector:'#a',strict:true}}),
        call(b.mainFrame.guid,'click',{{selector:'#b',strict:true}})
      ]);
    ",
            a = ids[0],
            b = ids[1]
        ),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.calls.len(), 6);
    assert_eq!(evaluate(&tab, &a, "window.clicks").await, 3);
    assert_eq!(evaluate(&tab, &b, "window.clicks").await, 3);
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_closing_one_page_cancels_its_click_without_stopping_the_other() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/start").await;
    let ids = create(&tab, 2).await;
    let a = driver(&tab, &ids[0], "<button id='go'>A</button>").await;
    driver(&tab, &ids[1], "<button id='go' disabled>B</button>").await;
    let outcome = run(&tab, &tab.target_id, &format!(r"
      const all = pages();
      const a = all.find(p=>p.guid==='page@{a}');
      const b = all.find(p=>p.guid==='page@{b}');
      const clicking = call(b.mainFrame.guid,'click',{{selector:'#go',strict:true}}).then(()=> 'clicked', e=>e.name);
      await new Promise(r=>setTimeout(r,200));
      await Promise.all([call(b.guid,'close'),call(a.mainFrame.guid,'click',{{selector:'#go',strict:true}})]);
      return await clicking;
    ", a=ids[0], b=ids[1])).await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!("TargetClosedError")));
    assert_eq!(evaluate(&tab, &a, "window.clicks").await, 1);
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_context_can_create_after_the_original_page_closes() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/start").await;
    let outcome = run(
        &tab,
        &tab.target_id,
        &format!(
            r"
      await call('page@{}','close');
      const created = (await call('context@default','newPage')).page;
      return get(created.guid,'initializer');
    ",
            tab.target_id
        ),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value.as_ref().unwrap()["mainFrame"]["initializer"]["url"],
        "about:blank"
    );
    assert!(outcome.pages[0].closed);
    assert!(outcome.pages[1].created);
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_run_before_unload_closes_after_answering_its_dialog() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/start").await;
    let ids = create(&tab, 1).await;
    let session = driver(&tab, &ids[0], "<button id='go'>Go</button>").await;
    evaluate(&tab, &session, "window.addEventListener('beforeunload', e => { e.preventDefault(); e.returnValue = 'stay'; });").await;
    let outcome = run(
        &tab,
        &tab.target_id,
        &format!(
            r"
      const target = pages().find(p=>p.guid==='page@{}');
      await call(target.mainFrame.guid,'click',{{selector:'#go',strict:true}});
      return (await call(target.guid,'runBeforeUnload')) === undefined;
    ",
            ids[0]
        ),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!(true)));
    wait_gone(&tab, &ids[0]).await;
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_browser_tab_primitives_match_the_contract() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/probe").await;
    let info = tab
        .cdp
        .send(
            "Browser.getTabInfo",
            json!({"targetId":tab.target_id}),
            None,
        )
        .await
        .unwrap();
    let window = info["tab"]["windowId"].clone();
    let before = tab
        .cdp
        .send("Browser.getTabs", json!({}), None)
        .await
        .unwrap();
    let created = create_in_window(&tab, &window).await;
    let target = created["tab"]["targetId"]
        .as_str()
        .expect("createTab returns tab.targetId");
    let background = tab
        .cdp
        .send("Browser.getTabInfo", json!({"targetId":target}), None)
        .await
        .unwrap();
    assert_eq!(background["tab"]["isActive"], false, "{background}");
    let session = tab
        .cdp
        .send(
            "Target.attachToTarget",
            json!({"targetId":target,"flatten":true}),
            None,
        )
        .await
        .unwrap();
    let session = SessionId::from(session["sessionId"].as_str().unwrap());
    tab.cdp
        .send("Page.bringToFront", json!({}), Some(&session))
        .await
        .unwrap();
    let page_front = tab
        .cdp
        .send("Browser.getTabInfo", json!({"targetId":target}), None)
        .await
        .unwrap();
    tab.cdp
        .send("Browser.activateTab", json!({"targetId":target}), None)
        .await
        .unwrap();
    let foreground = tab
        .cdp
        .send("Browser.getTabInfo", json!({"targetId":target}), None)
        .await
        .unwrap();
    assert_eq!(foreground["tab"]["isActive"], true, "{foreground}");
    let start = Instant::now();
    let close = tab
        .cdp
        .send("Target.closeTarget", json!({"targetId":target}), None)
        .await;
    let closed = tab
        .cdp
        .send("Browser.getTabInfo", json!({"targetId":target}), None)
        .await;
    let close_ms = start.elapsed().as_millis();
    wait_gone(&tab, target).await;
    let gone_ms = start.elapsed().as_millis();
    let created2 = create_in_window(&tab, &window).await;
    let start = Instant::now();
    let close_tab = tab
        .cdp
        .send(
            "Browser.closeTab",
            json!({"targetId":created2["tab"]["targetId"]}),
            None,
        )
        .await;
    let closed2 = tab
        .cdp
        .send(
            "Browser.getTabInfo",
            json!({"targetId":created2["tab"]["targetId"]}),
            None,
        )
        .await;
    wait_gone(&tab, created2["tab"]["targetId"].as_str().unwrap()).await;
    writeln!(io::stderr(), "TAB_PROBES info={info} before={before} created={created} background={background} Page.bringToFront={page_front} Browser.activateTab={foreground} closeTarget={close:?} immediatelyAfter={closed:?} replyMs={close_ms} goneMs={gone_ms} closeTab={close_tab:?} immediatelyAfter={closed2:?} goneMs={}", start.elapsed().as_millis()).unwrap();
    let scoped = create_in_context(&tab, &info["tab"]["liveBrowserContextId"]).await;
    assert_eq!(
        scoped["tab"]["liveBrowserContextId"],
        info["tab"]["liveBrowserContextId"]
    );
    assert_eq!(scoped["tab"]["isActive"], false);
    writeln!(io::stderr(), "TAB_CONTEXT_PROBE contextOnly={scoped}").unwrap();
    tab.close().await;
}

async fn wait_gone(tab: &Tab, target: &str) {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if tab
            .cdp
            .send("Browser.getTabInfo", json!({"targetId":target}), None)
            .await
            .is_err()
        {
            return;
        }
        assert!(Instant::now() < until, "tab {target} never closed");
        time::sleep(Duration::from_millis(5)).await;
    }
}

async fn create_in_window(tab: &Tab, window: &Value) -> Value {
    tab.cdp
        .send(
            "Browser.createTab",
            json!({"url":"about:blank","background":true,"windowId":window}),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("create tab in window: {error}"))
}

async fn create_in_context(tab: &Tab, context: &Value) -> Value {
    tab.cdp
        .send(
            "Browser.createTab",
            json!({"url":"about:blank","background":true,"browserContextId":context}),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("create tab in live context: {error}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_pages_order_survives_window_activation_and_appends_new_pages() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/start").await;
    let window = tab
        .cdp
        .send(
            "Browser.createWindow",
            json!({"url":format!("{}/second",tab.origin)}),
            None,
        )
        .await
        .unwrap();
    let all = tab
        .cdp
        .send("Browser.getTabs", json!({}), None)
        .await
        .unwrap();
    let other = all["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|page| page["windowId"] == window["window"]["windowId"])
        .unwrap()["targetId"]
        .as_str()
        .unwrap();
    let outcome = run(
        &tab,
        &tab.target_id,
        &format!(
            r"
      const order = () => pages().map(p=>p.guid);
      const before = order();
      await call('page@{me}','bringToFront'); const afterRun = order();
      await call('page@{other}','bringToFront'); const afterOther = order();
      await call('page@{me}','bringToFront');
      const created=(await call('context@default','newPage')).page.guid;
      return {{before,afterRun,afterOther,created,after:order()}};
    ",
            me = tab.target_id
        ),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    let value = outcome.value.unwrap();
    assert_eq!(value["before"][0], format!("page@{}", tab.target_id));
    assert_eq!(
        value["before"], value["afterRun"],
        "activation changed order"
    );
    assert_eq!(
        value["before"], value["afterOther"],
        "window activation changed order"
    );
    assert_eq!(
        value["after"].as_array().unwrap().last(),
        Some(&value["created"]),
        "new pages must be last: {value}"
    );
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_new_page_recreates_an_inactive_window_after_the_last_tab_closes() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/start").await;
    let all = tab
        .cdp
        .send("Browser.getTabs", json!({}), None)
        .await
        .unwrap();
    for page in all["tabs"].as_array().unwrap() {
        let id = page["targetId"].as_str().unwrap();
        if id != tab.target_id {
            tab.cdp
                .send("Target.closeTarget", json!({"targetId":id}), None)
                .await
                .unwrap();
            wait_gone(&tab, id).await;
        }
    }
    let outcome = run(
        &tab,
        &tab.target_id,
        &format!(
            r"
      await call('page@{}','close');
      const created=(await call('context@default','newPage')).page.guid;
      return {{created,pages:pages().map(p=>p.guid)}};
    ",
            tab.target_id
        ),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    let windows = tab
        .cdp
        .send("Browser.getWindows", json!({}), None)
        .await
        .unwrap();
    writeln!(
        io::stderr(),
        "TAB_LAST_WINDOW_PROBE windows={windows} outcome={:?}",
        outcome.value
    )
    .unwrap();
    assert!(
        windows["windows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|window| window["isActive"] == false),
        "new window stole focus: {windows}"
    );
    assert!(
        outcome
            .pages
            .iter()
            .any(|page| page.created && !page.closed),
        "{outcome:?}"
    );
    tab.close().await;
}

/// Waits for both initial documents to load before the driver adds the barrier's button.
async fn two_new_tabs(tab: &Tab, initial_ids: &[Value]) -> Vec<Value> {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let tabs = tab
            .cdp
            .send("Browser.getTabs", json!({}), None)
            .await
            .unwrap_or_else(|error| panic!("list acceptance tabs: {error}"));
        let fresh: Vec<_> = tabs["tabs"]
            .as_array()
            .unwrap_or_else(|| panic!("missing tabs in {tabs}"))
            .iter()
            .filter(|page| !initial_ids.contains(&page["targetId"]))
            .cloned()
            .collect();
        if fresh.len() == 2 && fresh.iter().all(|page| page["isLoading"] == false) {
            return fresh;
        }
        assert!(
            Instant::now() < until,
            "two SDK tabs never appeared: {tabs}"
        );
        time::sleep(Duration::from_millis(10)).await;
    }
}

fn assert_sdk_routes(outcome: &RunOutcome, first: &str, second: &str) {
    let routed: Vec<_> = outcome
        .calls
        .iter()
        .filter(|call| {
            call.params["selector"] == "html"
                || call.method == "bringToFront"
                || call.method == "close"
        })
        .map(|call| (call.method.clone(), call.guid.clone()))
        .collect();
    assert_eq!(
        routed,
        vec![
            ("click".to_string(), format!("frame@{second}")),
            ("bringToFront".to_string(), format!("page@{second}")),
            ("close".to_string(), format!("page@{first}")),
        ],
        "SDK objects must route their calls to the returned page's target"
    );
}
