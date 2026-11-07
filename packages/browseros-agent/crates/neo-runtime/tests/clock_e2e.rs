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
use browseros_cdp::SessionId;
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

/// A page with a same-origin frame and a cross-site one (`localhost`, an out-of-process frame),
/// so run end reaches more than the main frame.
const FRAMES: &str = "<!doctype html><title>Clock frames</title><iframe src=/clock/child></iframe>
<script>const remote = document.createElement('iframe');
remote.src = 'http://localhost:' + location.port + '/clock/remote'; document.body.append(remote)</script>";
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

/// What of the clock's replacements is not the browser's own in a frame no other realm can read,
/// by name: its globals, `performance`'s accessor, `AbortSignal.timeout` and `Event`'s
/// `timeStamp` getter; `clock` while `__pwClock` is there.
const NOT_NATIVE: &str = r"(() => {
  const native = (fn) => typeof fn === 'function' && /\{\s*\[native code\]\s*\}$/.test(Function.prototype.toString.call(fn));
  const own = (object, key) => Object.getOwnPropertyDescriptor(object, key);
  const checks = {
    clock: globalThis.__pwClock === undefined,
    timers: [setTimeout, clearTimeout, setInterval, clearInterval, requestAnimationFrame,
      cancelAnimationFrame, requestIdleCallback, cancelIdleCallback].every(native),
    Date: native(Date) && native(Date.now),
    performance: native(own(window, 'performance')?.get),
    Intl: native(Intl.DateTimeFormat),
    'AbortSignal.timeout': native(AbortSignal) && native(own(AbortSignal, 'timeout')?.value),
    'Event.prototype.timeStamp': native(own(Event.prototype, 'timeStamp')?.get),
    Temporal: typeof Temporal === 'undefined' || native(Temporal.Now.instant),
  };
  return Object.keys(checks).filter((name) => !checks[name]);
})()";

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

/// Sends `method` on `session`.
async fn send(tab: &Tab, session: &SessionId, method: &str, params: Value) -> Value {
    let reply = tab
        .cdp
        .send(method, params, Some(session))
        .await
        .unwrap_or_else(|error| panic!("{method}: {error}"));
    assert!(reply.get("exceptionDetails").is_none(), "{method}: {reply}");
    reply
}

/// A session of the driver's own on the cross-site frame, attached once the run reached it.
async fn remote_frame(tab: &Tab) -> SessionId {
    let reply = tab
        .cdp
        .send(
            "Target.getTargets",
            json!({ "filter": [{ "type": "iframe" }] }),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("Target.getTargets: {error}"));
    let target = reply["targetInfos"]
        .as_array()
        .and_then(|targets| {
            targets.iter().find(|target| {
                target["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with("/clock/remote"))
            })
        })
        .unwrap_or_else(|| panic!("an out-of-process frame: {reply}"));
    let attached = tab
        .cdp
        .send(
            "Target.attachToTarget",
            json!({ "targetId": target["targetId"], "flatten": true }),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("attach to the cross-site frame: {error}"));
    SessionId::from(
        attached["sessionId"]
            .as_str()
            .unwrap_or_else(|| panic!("Target.attachToTarget answered {attached}")),
    )
}

/// Evaluates `expression` on `session`, by value.
async fn value_in(tab: &Tab, session: &SessionId, expression: &str) -> Value {
    let reply = send(
        tab,
        session,
        "Runtime.evaluate",
        json!({ "expression": expression, "returnByValue": true }),
    )
    .await;
    reply["result"]["value"].clone()
}

/// The run: it resumes the paused clock and leaves a `runFor` running, which resumes it again
/// on its way out, then waits until the test cancels it.
const RUN_FOR_LEFT_RUNNING: &str = r"await page.waitForLoadState('load')
await page.clock.install({ time: 0 })
await page.clock.pauseAt(1000)
await page.evaluate(() => {
  const probe = document.documentElement.probe
  probe.kept = setTimeout
  setInterval(() => probe.ticks++, 1)
})
await page.clock.resume()
page.clock.runFor(10000000).catch(() => {})
await new Promise(() => {})";

/// Every name [`NOT_NATIVE`] reports while the run's clock is on a frame.
const CLOCKED: [&str; 8] = [
    "clock",
    "timers",
    "Date",
    "performance",
    "Intl",
    "AbortSignal.timeout",
    "Event.prototype.timeStamp",
    "Temporal",
];

/// The page with its frames loaded and the probe set, and what [`RECORD`] kept, by CDP id.
async fn open_frames() -> Option<(Tab, Value)> {
    let binary = browser::e2e_browser()?;
    let fixture = Fixture {
        pages: vec![
            ("/clock/frames".to_string(), FRAMES.to_string()),
            ("/clock/child".to_string(), CHILD.to_string()),
            ("/clock/remote".to_string(), CHILD.to_string()),
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
    let recorded = send(
        &tab,
        &tab.driver,
        "Runtime.evaluate",
        json!({ "expression": RECORD }),
    )
    .await;
    let recorded = recorded["result"]["objectId"].clone();
    Some((tab, recorded))
}

/// Starts `code` against the tab on a runtime of its own, which the test drains, with a
/// cancellation token the test holds.
fn start(tab: &Tab, runtime: &Runtime, code: &str) -> (CancellationToken, JoinHandle<RunOutcome>) {
    let cancel = CancellationToken::new();
    let request = RunRequest {
        code: code.to_string(),
        page: PageId::new(tab.target_id.clone()),
        timeout: Duration::from_secs(30),
        cancel: cancel.clone(),
    };
    let runtime = runtime.clone();
    (
        cancel,
        tokio::spawn(async move { runtime.run(request).await }),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_nothing_of_a_runs_clock_outlives_it() {
    let Some((tab, recorded)) = open_frames().await else {
        return;
    };
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    let (cancel, run) = start(&tab, &runtime, RUN_FOR_LEFT_RUNNING);
    wait_for(&tab, RUNNING_FOR).await;
    let remote = remote_frame(&tab).await;
    let remote_during = value_in(&tab, &remote, NOT_NATIVE).await;
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
    let remote_after = value_in(&tab, &remote, NOT_NATIVE).await;
    let changed = send(
        &tab,
        &tab.driver,
        "Runtime.callFunctionOn",
        json!({ "objectId": recorded, "functionDeclaration": CHANGED, "returnByValue": true }),
    )
    .await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:?}");
    assert_eq!(
        remote_during,
        json!(CLOCKED),
        "the run's clock reached the cross-site frame"
    );
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
    assert_eq!(
        remote_after,
        json!([]),
        "the cross-site frame has the browser's own timing APIs back"
    );
}
