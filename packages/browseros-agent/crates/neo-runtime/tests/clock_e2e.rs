//! The fake clock end to end on a real BrowserOS neo, through neo-sdk's `page.clock`, for Neo's
//! one deviation from Playwright here: the clock lives as long as the run (`Deviation:` in
//! `neo-runtime-injected/src/clock-call.ts`). Once a run that paused the page's time has ended,
//! the page has its own `Date` and timers back, and a later document has no clock; nothing of the
//! clock outlives the run, even a `runFor` it left running.
//! `crates/neo-sdk/tests/parity/clock.test.mjs` compares the in-run behaviour with live
//! Playwright.
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary (`support/browser.rs`); otherwise
//! each test passes as a skip.

#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{PageId, RunOutcome, RunRequest, RunStatus, Runtime};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio::time;
use tokio_util::sync::CancellationToken;

const PAGE: &str = "<!doctype html><title>Clock</title><p id=p>text</p>";

/// Whether the page's time is its own (a 2020s epoch), its clock global, and a timer that
/// fires, which a paused fake clock never lets happen.
const OWN_TIME: &str = "(async () => [Date.now() > 1600000000000, typeof globalThis.__pwClock, await new Promise((resolve) => setTimeout(() => resolve('fired'), 10))])()";

/// How long a page's own timer may take.
const TIMER: Duration = Duration::from_secs(3);

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let fixture = Fixture {
        pages: vec![("/clock".to_string(), PAGE.to_string())],
        ..Fixture::default()
    };
    Some(Tab::open(binary, fixture, "/clock").await)
}

async fn finish(run: JoinHandle<RunOutcome>) -> RunOutcome {
    run.await
        .unwrap_or_else(|error| panic!("the run's task: {error}"))
}

fn value(outcome: &RunOutcome) -> Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    outcome
        .value
        .clone()
        .unwrap_or_else(|| panic!("the script returns a value: {outcome:?}"))
}

/// Evaluates `expression` in the tab's main world, awaiting a promise, outside Neo: `hung` when
/// it has not settled within [`TIMER`].
async fn settle(tab: &Tab, expression: &str) -> Value {
    let evaluated = tab.cdp.send(
        "Runtime.evaluate",
        json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
        Some(&tab.driver),
    );
    let Ok(reply) = time::timeout(TIMER, evaluated).await else {
        return json!("hung");
    };
    let reply = reply.unwrap_or_else(|error| panic!("{expression}: {error}"));
    assert!(
        reply.get("exceptionDetails").is_none(),
        "{expression} threw: {reply}"
    );
    reply["result"]["value"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_runs_clock_ends_with_the_run() {
    let Some(tab) = open().await else { return };
    let during = value(
        &finish(
            tab.start_run(
                r"await page.clock.install({ time: 0 })
                  await page.clock.pauseAt(1000)
                  return await page.evaluate(() => [Date.now(), typeof globalThis.__pwClock])"
                    .to_string(),
            )
            .await,
        )
        .await,
    );
    assert_eq!(
        during,
        json!([1000, "object"]),
        "the run's clock is paused at 1000"
    );

    assert_eq!(
        settle(&tab, OWN_TIME).await,
        json!([true, "undefined", "fired"]),
        "after the run, the page has its own time and timers"
    );
    tab.navigate("/clock").await;
    assert_eq!(
        settle(&tab, OWN_TIME).await,
        json!([true, "undefined", "fired"]),
        "a later document has no clock"
    );
    tab.close().await;
}

/// A page with a same-origin frame, so run end reaches more than the main frame.
const FRAMES: &str = "<!doctype html><title>Clock frames</title><iframe src=/clock/child></iframe>";
const CHILD: &str = "<!doctype html><title>Clock child</title>";

/// The run's probe, on an element so it adds no global: the page's own `Date.now`, kept before
/// the run, and the ticks of the fake interval the run sets.
const PROBE: &str = "void (document.documentElement.probe = { ticks: 0, realNow: Date.now.bind(Date), since: Date.now() })";

/// More ticks than real milliseconds since the probe: only a `runFor` runs the fake clock ahead
/// of real time, so one is running in the page.
const RUNNING_FOR: &str = "(() => { const probe = document.documentElement.probe; return probe.ticks - (probe.realNow() - probe.since) > 1000 })()";

/// Every own property of what the clock replaces or changes in place, in the page and its frame,
/// read now and kept as a CDP object the page cannot reach; [`CHANGED`] reads them again.
const RECORD: &str = r"(() => {
  const own = (object) => new Map(Reflect.ownKeys(object).map((key) => [key, Object.getOwnPropertyDescriptor(object, key)]));
  const read = (realm) => [['window', realm], ['AbortSignal', realm.AbortSignal],
    ['Event.prototype', realm.Event.prototype], ['Date', realm.Date], ['Date.prototype', realm.Date.prototype]]
    .map(([name, object]) => [name, own(object)]);
  const realms = [['page', window], ['frame', frames[0]]];
  return { read, realms, before: realms.map(([, realm]) => read(realm)) };
})()";

/// The properties [`RECORD`] kept that are not as they were, or were not there, as
/// `<realm> <object>.<key>`.
const CHANGED: &str = r"function () {
  const same = (a, b) => a !== undefined && b !== undefined &&
    ['value', 'get', 'set', 'writable', 'enumerable', 'configurable'].every((field) => Object.is(a[field], b[field]));
  const changed = [];
  this.realms.forEach(([where, realm], index) => {
    const now = this.read(realm);
    this.before[index].forEach(([name, was], part) => {
      const is = now[part][1];
      for (const key of new Set([...was.keys(), ...is.keys()]))
        if (!same(was.get(key), is.get(key))) changed.push(`${where} ${name}.${String(key)}`);
    });
  });
  return changed;
}";

/// Waits until `expression` is `true` in the page.
async fn wait_for(tab: &Tab, expression: &str) {
    time::timeout(Duration::from_secs(15), async {
        while tab.evaluate(expression).await != json!(true) {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|error| panic!("{expression}: {error}"));
}

/// Sends `method` on the tab's driver session.
async fn send(tab: &Tab, method: &str, params: Value) -> Value {
    let reply = tab
        .cdp
        .send(method, params, Some(&tab.driver))
        .await
        .unwrap_or_else(|error| panic!("{method}: {error}"));
    assert!(reply.get("exceptionDetails").is_none(), "{method}: {reply}");
    reply
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_nothing_of_a_runs_clock_outlives_it() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let fixture = Fixture {
        pages: vec![
            ("/clock/frames".to_string(), FRAMES.to_string()),
            ("/clock/child".to_string(), CHILD.to_string()),
        ],
        ..Fixture::default()
    };
    let tab = Tab::open(binary, fixture, "/clock/frames").await;
    wait_for(
        &tab,
        "frames[0]?.location.pathname === '/clock/child' && frames[0].document.readyState === 'complete'",
    )
    .await;
    tab.evaluate(PROBE).await;
    let recorded = send(&tab, "Runtime.evaluate", json!({ "expression": RECORD })).await;
    let recorded = recorded["result"]["objectId"].clone();

    // The run resumes the paused clock and leaves a `runFor` running, which resumes it again on
    // its way out; it ends by cancellation while that call runs.
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    let cancel = CancellationToken::new();
    let request = RunRequest {
        code: r"await page.clock.install({ time: 0 })
               await page.clock.pauseAt(1000)
               await page.evaluate(() => {
                 const probe = document.documentElement.probe
                 probe.kept = setTimeout
                 setInterval(() => probe.ticks++, 1)
               })
               await page.clock.resume()
               page.clock.runFor(10000000).catch(() => {})
               await new Promise(() => {})"
            .to_string(),
        page: PageId::new(tab.target_id.clone()),
        timeout: Duration::from_secs(60),
        cancel: cancel.clone(),
    };
    let run = tokio::spawn({
        let runtime = runtime.clone();
        async move { runtime.run(request).await }
    });
    wait_for(&tab, RUNNING_FOR).await;
    cancel.cancel();
    let outcome = finish(run).await;
    time::timeout(Duration::from_secs(10), runtime.drain())
        .await
        .unwrap_or_else(|error| panic!("the run's cleanup drains: {error}"));

    let ticks = tab.evaluate("document.documentElement.probe.ticks").await;
    tab.evaluate(
        "void document.documentElement.probe.kept(() => { document.documentElement.probe.late = true }, 0)",
    )
    .await;
    time::sleep(Duration::from_millis(500)).await;
    let later = tab
        .evaluate(
            "[document.documentElement.probe.ticks, document.documentElement.probe.late ?? false]",
        )
        .await;
    let changed = send(
        &tab,
        "Runtime.callFunctionOn",
        json!({ "objectId": recorded, "functionDeclaration": CHANGED, "returnByValue": true }),
    )
    .await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:?}");
    assert_eq!(
        later,
        json!([ticks, false]),
        "after the run, no fake timer fires: not the next one the `runFor` would reach, nor one \
         set through the fake `setTimeout` the page kept"
    );
    assert_eq!(
        changed["result"]["value"],
        json!([]),
        "every property the clock replaced, in the page and its frame, is as it was before"
    );
}
