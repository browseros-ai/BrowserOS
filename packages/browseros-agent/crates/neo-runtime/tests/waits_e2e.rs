//! `locator.waitFor`, `page`/`frame.waitForSelector`, `waitForTimeout`, `page.waitForURL` and
//! `page.waitForLoadState` on a real BrowserOS neo, each test on a throwaway profile:
//! - waits the same run meets by clicking, and by typing, which would time out if a wait held the
//!   tab's input turn for its whole life;
//! - waits that resolve while a key press holds the input turn, which fail if any single check
//!   takes the turn;
//! - a long wait on one tab, resolved only by an action on that tab, while runs on another tab
//!   click and type at once;
//! - runs ended by their deadline or cancellation under pending waits: each returns promptly, its
//!   CDP session detaches (no task it left holds it), and the next run on the tab clicks at once;
//! - a tab closed under its waits;
//! - a URL wait met by a hash change and by a navigating click;
//! - `DOMContentLoaded` held back by a parser-blocking script, and long before load on a slow
//!   page;
//! - `page`/`frame.waitForSelector` in every state, pending until a click changes the page, then
//!   `null` (Neo has no `ElementHandle` yet); the first of two matches judged, and `strict: true`
//!   refused;
//! - `waitForTimeout` waiting out its delay, and a tab closed under one that was the run's first
//!   call.
//!
//! Behaviour only: no call logs, and time only as generous bounds.

#[expect(
    dead_code,
    reason = "shared fixture includes navigation helpers other suites use"
)]
#[path = "support/browser.rs"]
mod browser;

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use browser::{Fixture, Tab};
use browseros_cdp::{CdpEvent, SessionId};
use neo_runtime::{
    CallRecord, CallResult, PageId, RunEnd, RunOutcome, RunRequest, RunStatus, Runtime,
};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::time;
use tokio_util::sync::CancellationToken;

/// `#add` adds `#added`; typing `neo` into `#name` adds `#typed` and sets the hash `#typed`.
const PAGE: &str = r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>waits</title></head><body>
<button id="add" type="button">Add</button>
<input id="name">
<a id="to-done" href="/done">Done</a>
<a id="to-hash" href="#section">Section</a>
<p id="section">Section</p>
<script>
const add = (id) => {
  const element = document.createElement('p')
  element.id = id
  element.textContent = id
  document.body.append(element)
}
document.querySelector('#add').addEventListener('click', () => add('added'))
document.querySelector('#name').addEventListener('input', (event) => {
  if (event.target.value !== 'neo') return
  add('typed')
  location.hash = 'typed'
})
</script>
</body></html>"##;

/// Its image answers after the fixture's `slow_response` (paths starting `/slow`), so load
/// fires that long after `DOMContentLoaded`. Each handler names the event in the title.
const LAZY: &str = r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>committed</title></head><body>
<img src="/slow-image" alt="">
<script>
document.addEventListener('DOMContentLoaded', () => { document.title = 'domcontentloaded' })
addEventListener('load', () => { document.title = 'load' })
</script>
</body></html>"#;

/// Its parser stops at a script the fixture answers after `slow_response` (paths starting
/// `/slow`), so `DOMContentLoaded`, whose handler renames the page, waits that long.
const HELD: &str = r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>committed</title>
<script>document.addEventListener('DOMContentLoaded', () => { document.title = 'domcontentloaded' })</script>
</head><body>
<script src="/slow-script.js"></script>
<p id="after">After the script</p>
</body></html>"#;

/// `#change` adds `#late`, removes `#doomed`, hides `#fade` and shows `#reveal`, all in one task.
/// `.pair` matches twice, the first hidden.
const SELECTORS: &str = r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>selectors</title></head><body>
<button id="change" type="button">Change</button>
<p id="doomed">Doomed</p>
<p id="fade">Fade</p>
<p id="reveal" style="display: none">Reveal</p>
<p class="pair" style="display: none">First</p>
<p class="pair">Second</p>
<script>
document.querySelector('#change').addEventListener('click', () => {
  const late = document.createElement('p')
  late.id = 'late'
  late.textContent = 'Late'
  document.body.append(late)
  document.querySelector('#doomed').remove()
  document.querySelector('#fade').style.display = 'none'
  document.querySelector('#reveal').style.display = ''
})
</script>
</body></html>"#;

/// How long a run may take to return once its deadline passed or it was cancelled: well under
/// the 1 s its finish waits at most for the tasks it leaves (`dispatch.rs` `RECORDS_WAIT`).
const PROMPT_RETURN: Duration = Duration::from_millis(500);

/// Every wait a test expects to be met gets this, far longer than any of them takes, so a wait
/// that cannot be met (as when it held the tab's input) fails as a timeout instead of hanging.
const MET_WITHIN_MS: u64 = 10_000;

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                slow_response: Duration::from_secs(3),
                pages: vec![
                    ("/waits".to_string(), PAGE.to_string()),
                    ("/lazy".to_string(), LAZY.to_string()),
                    ("/held".to_string(), HELD.to_string()),
                    ("/selectors".to_string(), SELECTORS.to_string()),
                    (
                        "/slow-script.js".to_string(),
                        "document.documentElement.dataset.script = 'ran'".to_string(),
                    ),
                ],
            },
            "/waits",
        )
        .await,
    )
}

/// One `Runtime` for the tab, whose runs share its input turns as one server's runs do.
async fn runtime(tab: &Tab) -> Runtime {
    Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"))
}

fn request(
    target_id: &str,
    code: &str,
    timeout: Duration,
    cancel: CancellationToken,
) -> RunRequest {
    RunRequest {
        code: code.to_string(),
        page: PageId::new(target_id.to_string()),
        timeout,
        cancel,
    }
}

async fn run(tab: &Tab, code: &str) -> RunOutcome {
    tab.start_run(code.to_string())
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"))
}

fn value(outcome: &RunOutcome) -> Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    outcome.value.clone().unwrap_or(Value::Null)
}

/// The run's records of `method`, in arrival order.
fn records<'a>(outcome: &'a RunOutcome, method: &str) -> Vec<&'a CallRecord> {
    outcome
        .calls
        .iter()
        .filter(|call| call.method == method)
        .collect()
}

/// When the call replied (or the run ended under it), in Unix milliseconds.
fn ended_at_ms(call: &CallRecord) -> u128 {
    u128::from(call.started_at_ms) + call.duration.as_millis()
}

/// The CDP sessions attached to the tab since `events` subscribed, other than the test's own
/// driver, that are still attached after up to 3 s. A run detaches its session when it ends,
/// unless a task it left behind still holds the page session. Fails when no session attached:
/// then the check would prove nothing.
async fn sessions_left(events: &mut broadcast::Receiver<CdpEvent>, tab: &Tab) -> BTreeSet<String> {
    let mut seen = 0;
    let mut attached = BTreeSet::new();
    let _ = time::timeout(Duration::from_secs(3), async {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(RecvError::Lagged(skipped)) => panic!("missed {skipped} CDP events"),
                Err(RecvError::Closed) => return,
            };
            let session = event.params["sessionId"].as_str().unwrap_or_default();
            match event.method.as_str() {
                "Target.attachedToTarget"
                    if event.params["targetInfo"]["targetId"] == json!(tab.target_id)
                        && session != tab.driver.as_str() =>
                {
                    seen += 1;
                    attached.insert(session.to_string());
                }
                "Target.detachedFromTarget" => {
                    attached.remove(session);
                }
                _ => {}
            }
            if seen > 0 && attached.is_empty() {
                return;
            }
        }
    })
    .await;
    assert!(seen > 0, "no run session attached to the tab");
    attached
}

fn path(url: &Value, origin: &str) -> String {
    url.as_str()
        .unwrap_or_default()
        .strip_prefix(origin)
        .unwrap_or_default()
        .to_string()
}

/// Evaluates `expression` on `session` until it returns `true`, for up to 15 s.
async fn eventually(tab: &Tab, session: &SessionId, expression: &str) {
    let started = Instant::now();
    loop {
        let reply = tab
            .cdp
            .send(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
                Some(session),
            )
            .await;
        if reply.is_ok_and(|reply| reply["result"]["value"] == json!(true)) {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "never true: {expression}"
        );
        time::sleep(Duration::from_millis(20)).await;
    }
}

/// A second tab on `path` of the fixture's origin, loaded, with a session the test reads it on.
async fn second_tab(tab: &Tab, path: &str) -> (String, SessionId) {
    let created = tab
        .cdp
        .send(
            "Target.createTarget",
            json!({ "url": format!("{}{path}", tab.origin) }),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("open a second tab: {error}"));
    let target_id = created["targetId"].as_str().unwrap_or_default().to_string();
    let attached = tab
        .cdp
        .send(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("attach the second tab: {error}"));
    let session = SessionId::from(attached["sessionId"].as_str().unwrap_or_default());
    eventually(
        tab,
        &session,
        &format!(
            "location.pathname === {} && document.readyState === 'complete'",
            json!(path)
        ),
    )
    .await;
    (target_id, session)
}

/// The owner's acceptance: a run starts two waits, then clicks what meets them. Had either wait
/// taken the tab's input, the clicks would wait their turn until the wait timed out.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_click_in_the_same_run_meets_pending_waits() {
    let Some(tab) = open().await else { return };
    let started = Instant::now();
    let outcome = run(
        &tab,
        &format!(
            "const appeared = page.locator('#added').waitFor({{ timeout: {MET_WITHIN_MS} }})
const hashed = page.waitForURL(/#section$/, {{ timeout: {MET_WITHIN_MS} }})
await page.click('#add')
await page.click('#to-hash')
await appeared
await hashed
return page.url()"
        ),
    )
    .await;
    let elapsed = started.elapsed();
    let added = tab
        .evaluate("document.querySelectorAll('#added').length")
        .await;
    let origin = tab.origin.clone();
    tab.close().await;

    assert_eq!(
        path(&value(&outcome), &origin),
        "/waits#section",
        "{outcome:#?}"
    );
    assert!(
        outcome
            .calls
            .iter()
            .all(|call| call.result == CallResult::Ok),
        "{:#?}",
        outcome.calls
    );
    assert_eq!(
        records(&outcome, "waitForSelector").len(),
        1,
        "{outcome:#?}"
    );
    assert_eq!(records(&outcome, "neo:waitForURL").len(), 1, "{outcome:#?}");
    assert_eq!(added, json!(1), "one click, one element");
    assert!(
        elapsed < Duration::from_millis(MET_WITHIN_MS),
        "the run took {elapsed:?}"
    );
}

/// The same with typing: three key presses into `#name` meet a selector wait and a URL wait the
/// run started first.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_typing_in_the_same_run_meets_pending_waits() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        &format!(
            "const appeared = page.locator('#typed').waitFor({{ state: 'attached', timeout: {MET_WITHIN_MS} }})
const hashed = page.waitForURL('**#typed', {{ timeout: {MET_WITHIN_MS} }})
for (const key of ['n', 'e', 'o']) await page.locator('#name').press(key)
await appeared
await hashed
return [page.url(), await page.locator('#name').inputValue()]"
        ),
    )
    .await;
    tab.close().await;

    let value = value(&outcome);
    assert!(
        value[0]
            .as_str()
            .is_some_and(|url| url.ends_with("/waits#typed")),
        "{value}"
    );
    assert_eq!(value[1], json!("neo"), "{value}");
    assert!(
        outcome
            .calls
            .iter()
            .all(|call| call.result == CallResult::Ok),
        "{:#?}",
        outcome.calls
    );
    assert_eq!(records(&outcome, "press").len(), 3, "{outcome:#?}");
}

/// A key press with a `delay` holds the tab's input turn from keydown to keyup (`press.rs`).
/// Waits started inside that window must resolve within 1 s each, and before the press ends: a
/// wait with any single check that took the turn would sit behind the press for its last 2.5 s
/// and time out.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_waits_resolve_while_a_press_holds_the_input_turn() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        "const pressed = page.locator('#name').press('a', { delay: 3000 })
await new Promise((resolve) => setTimeout(resolve, 500))
const settled = (wait) => wait.then(() => 'resolved', (error) => error.name)
const results = {
  waitFor: await settled(page.locator('#add').waitFor({ timeout: 1000 })),
  waitForURL: await settled(page.waitForURL('**/waits', { timeout: 1000 })),
  waitForLoadState: await settled(page.waitForLoadState('load', { timeout: 1000 })),
}
await pressed
return results",
    )
    .await;
    tab.close().await;

    assert_eq!(
        value(&outcome),
        json!({ "waitFor": "resolved", "waitForURL": "resolved", "waitForLoadState": "resolved" }),
        "{outcome:#?}"
    );
    let [press] = records(&outcome, "press")[..] else {
        panic!("one press: {outcome:#?}");
    };
    for method in ["waitForSelector", "neo:waitForURL", "neo:waitForLoadState"] {
        let [wait] = records(&outcome, method)[..] else {
            panic!("one {method}: {outcome:#?}");
        };
        assert!(
            wait.started_at_ms > press.started_at_ms && ended_at_ms(wait) < ended_at_ms(press),
            "{method} did not run inside the press: {wait:#?} {press:#?}"
        );
    }
}

/// A wait on tab A holds nothing tab B needs: while it waits, a run on B clicks and types and
/// returns. A's wait resolves only after a later run's click on A started: B's own `#added`
/// never met it.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_long_wait_on_one_tab_leaves_another_tab_free() {
    let Some(tab) = open().await else { return };
    let runtime = runtime(&tab).await;
    let (other, other_session) = second_tab(&tab, "/waits").await;
    let long = Duration::from_secs(30);
    let waiting = runtime.run(request(
        &tab.target_id,
        "await page.locator('#added').waitFor({ timeout: 25000 }); return 'met'",
        long,
        CancellationToken::new(),
    ));
    let others = async {
        // Let A's wait start polling first.
        time::sleep(Duration::from_millis(500)).await;
        let started = Instant::now();
        let typed = runtime
            .run(request(
                &other,
                "await page.click('#add')
for (const key of ['n', 'e', 'o']) await page.locator('#name').press(key)
return await page.locator('#name').inputValue()",
                long,
                CancellationToken::new(),
            ))
            .await;
        let typed_after = started.elapsed();
        let pending_on_a = tab
            .evaluate("document.querySelectorAll('#added').length")
            .await;
        let met = runtime
            .run(request(
                &tab.target_id,
                "await page.click('#add')",
                long,
                CancellationToken::new(),
            ))
            .await;
        (typed, typed_after, pending_on_a, met)
    };
    let (waited, (typed, typed_after, pending_on_a, met)) = tokio::join!(waiting, others);
    eventually(
        &tab,
        &other_session,
        "document.querySelectorAll('#added').length === 1",
    )
    .await;
    tab.close().await;

    assert_eq!(value(&typed), json!("neo"), "{typed:#?}");
    assert!(
        typed_after < Duration::from_secs(10),
        "tab B's run took {typed_after:?} while tab A waited"
    );
    assert_eq!(pending_on_a, json!(0), "nothing reached tab A from tab B");
    assert_eq!(met.status, RunStatus::Ok, "{met:#?}");
    assert_eq!(value(&waited), json!("met"), "{waited:#?}");
    let [wait] = records(&waited, "waitForSelector")[..] else {
        panic!("one wait on A: {waited:#?}");
    };
    let [click] = records(&met, "click")[..] else {
        panic!("one click on A: {met:#?}");
    };
    assert!(
        ended_at_ms(wait) >= u128::from(click.started_at_ms),
        "A's wait resolved before the click on A started: {wait:#?} {click:#?}"
    );
}

/// A run ends, by its deadline or by cancellation, while two of its waits have no timeout of
/// their own. It returns promptly ([`PROMPT_RETURN`]), both records are `Unfinished` with that
/// reason, and its CDP session detaches: no wait it started still holds the page. The next run on
/// the tab clicks and waits at once.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_run_ended_under_its_waits_leaves_the_tab_free() {
    let Some(tab) = open().await else { return };
    let runtime = runtime(&tab).await;
    let pending = "await Promise.all([
  page.locator('#never').waitFor({ state: 'attached', timeout: 0 }),
  page.waitForURL('**/never', { timeout: 0 }),
])";
    let deadline = Duration::from_millis(1500);
    let mut events = tab.cdp.events();
    let started = Instant::now();
    let timed_out = runtime
        .run(request(
            &tab.target_id,
            pending,
            deadline,
            CancellationToken::new(),
        ))
        .await;
    let timed_out_after = started.elapsed().saturating_sub(deadline);
    let timed_out_left = sessions_left(&mut events, &tab).await;
    let mut events = tab.cdp.events();
    let cancel = CancellationToken::new();
    let cancel_later = async {
        time::sleep(Duration::from_millis(1500)).await;
        cancel.cancel();
        Instant::now()
    };
    let (cancelled, cancelled_at) = tokio::join!(
        runtime.run(request(
            &tab.target_id,
            pending,
            Duration::from_secs(30),
            cancel.clone(),
        )),
        cancel_later,
    );
    let cancelled_after = cancelled_at.elapsed();
    let cancelled_left = sessions_left(&mut events, &tab).await;
    let started = Instant::now();
    let next = runtime
        .run(request(
            &tab.target_id,
            "await page.click('#add')
await page.locator('#added').waitFor({ timeout: 5000 })
await page.waitForURL('**/waits', { timeout: 5000 })
await page.waitForLoadState('load', { timeout: 5000 })
return 'free'",
            Duration::from_secs(30),
            CancellationToken::new(),
        ))
        .await;
    let next_after = started.elapsed();
    let added = tab
        .evaluate("document.querySelectorAll('#added').length")
        .await;
    tab.close().await;

    for (outcome, status, reason, returned_after, left) in [
        (
            &timed_out,
            RunStatus::Timeout,
            RunEnd::Timeout,
            timed_out_after,
            timed_out_left,
        ),
        (
            &cancelled,
            RunStatus::Cancelled,
            RunEnd::Cancelled,
            cancelled_after,
            cancelled_left,
        ),
    ] {
        assert_eq!(outcome.status, status, "{outcome:#?}");
        assert!(
            returned_after < PROMPT_RETURN,
            "{status:?}: returned {returned_after:?} after its end"
        );
        assert!(
            left.is_empty(),
            "{status:?}: sessions still attached: {left:?}"
        );
        let results: Vec<_> = outcome.calls.iter().map(|call| &call.result).collect();
        assert_eq!(
            results,
            [
                &CallResult::Unfinished { reason },
                &CallResult::Unfinished { reason }
            ],
            "{outcome:#?}"
        );
    }
    assert_eq!(value(&next), json!("free"), "{next:#?}");
    assert!(
        next_after < Duration::from_secs(10),
        "the next run took {next_after:?}"
    );
    assert_eq!(added, json!(1), "one click, one element");
}

/// The tab closes while a run waits on it, for an element, a URL and a load: every wait rejects
/// as closed long before its own timeout, and the run itself still returns.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_closing_the_tab_rejects_its_waits() {
    let Some(tab) = open().await else { return };
    let lazy = format!("{}/lazy", tab.origin);
    let run = tab
        .start_run(format!(
            "await page.goto({}, {{ waitUntil: 'commit' }})
const closed = (wait) => wait.then(
  () => 'resolved',
  (error) => ({{ name: error.name, closed: /has been closed/.test(error.message) }}),
)
return await Promise.all([
  closed(page.locator('#never').waitFor({{ timeout: 20000 }})),
  closed(page.waitForURL('**/never', {{ timeout: 20000 }})),
  closed(page.waitForLoadState('load', {{ timeout: 20000 }})),
])",
            json!(lazy)
        ))
        .await;
    // The image of /lazy answers after 3 s: close the tab while its load is still pending.
    eventually(
        &tab,
        &tab.driver,
        &format!("location.href === {}", json!(lazy)),
    )
    .await;
    time::sleep(Duration::from_millis(500)).await;
    let closed_at = Instant::now();
    tab.cdp
        .send(
            "Target.closeTarget",
            json!({ "targetId": tab.target_id }),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("close the run's tab: {error}"));
    let outcome = run
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"));
    let returned_after = closed_at.elapsed();
    tab.close().await;

    let closed = json!({ "name": "Error", "closed": true });
    assert_eq!(
        value(&outcome),
        json!([closed, closed, closed]),
        "{outcome:#?}"
    );
    for method in ["waitForSelector", "neo:waitForURL", "neo:waitForLoadState"] {
        let [record] = records(&outcome, method)[..] else {
            panic!("one {method}: {outcome:#?}");
        };
        assert!(
            matches!(&record.result, CallResult::Error { name, .. } if name == "TargetClosedError"),
            "{record:#?}"
        );
    }
    assert!(
        returned_after < Duration::from_secs(10),
        "the run returned {returned_after:?} after the close"
    );
}

/// A URL wait follows the tab's own navigations: a hash change within the document, then a
/// click that loads another document.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_url_wait_follows_a_hash_change_and_a_navigating_click() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        &format!(
            "const hashed = page.waitForURL(/#section$/, {{ timeout: {MET_WITHIN_MS} }})
await page.click('#to-hash')
await hashed
const hash = page.url()
const done = page.waitForURL('**/done', {{ timeout: {MET_WITHIN_MS} }})
await page.click('#to-done')
await done
return {{ hash, done: page.url(), title: await page.title() }}"
        ),
    )
    .await;
    let origin = tab.origin.clone();
    tab.close().await;

    let value = value(&outcome);
    assert_eq!(path(&value["hash"], &origin), "/waits#section", "{value}");
    assert_eq!(path(&value["done"], &origin), "/done", "{value}");
    assert_eq!(value["title"], json!("/done"), "{value}");
    assert_eq!(records(&outcome, "neo:waitForURL").len(), 2, "{outcome:#?}");
}

/// A parser-blocking script the fixture answers 3 s late holds `DOMContentLoaded` back: a
/// `domcontentloaded` wait, by load state or by URL, times out meanwhile, and one without a short
/// timeout resolves only once the page's own `DOMContentLoaded` handler ran.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_domcontentloaded_wait_waits_for_a_parser_blocking_script() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        &format!(
            "await page.goto({}, {{ waitUntil: 'commit' }})
const settled = (wait) => wait.then(() => 'resolved', (error) => error.name)
const early = await settled(page.waitForLoadState('domcontentloaded', {{ timeout: 300 }}))
const earlyUrl = await settled(
  page.waitForURL('**/held', {{ waitUntil: 'domcontentloaded', timeout: 300 }}),
)
const before = await page.title()
await page.waitForLoadState('domcontentloaded')
return {{ early, earlyUrl, before, at: await page.title() }}",
            json!(format!("{}/held", tab.origin))
        ),
    )
    .await;
    tab.close().await;

    assert_eq!(
        value(&outcome),
        json!({
            "early": "TimeoutError",
            "earlyUrl": "TimeoutError",
            "before": "committed",
            "at": "domcontentloaded",
        }),
        "{outcome:#?}"
    );
}

/// On a page whose image answers 3 s late, `waitForLoadState('domcontentloaded')` returns while
/// load is still pending, and `waitForLoadState('load')` only once it fired.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_domcontentloaded_comes_long_before_load_on_a_slow_page() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        &format!(
            "await page.goto({}, {{ waitUntil: 'commit' }})
await page.waitForLoadState('domcontentloaded')
const atDomContentLoaded = await page.title()
await page.waitForLoadState()
return {{ atDomContentLoaded, atLoad: await page.title() }}",
            json!(format!("{}/lazy", tab.origin))
        ),
    )
    .await;
    tab.close().await;

    assert_eq!(
        value(&outcome),
        json!({ "atDomContentLoaded": "domcontentloaded", "atLoad": "load" }),
        "{outcome:#?}"
    );
    let waits = records(&outcome, "neo:waitForLoadState");
    assert_eq!(
        waits
            .iter()
            .map(|call| call.params["state"].clone())
            .collect::<Vec<_>>(),
        [json!("domcontentloaded"), json!("load")],
        "{outcome:#?}"
    );
}

/// Every state, in page and frame form, met by one click. Nothing on the page changes before the
/// click, so a wait that settled before it settled early; after it, each resolves `null`, where
/// Playwright resolves an `ElementHandle` for `attached` and `visible` (Neo has none yet).
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_page_and_frame_selector_waits_resolve_null_once_each_state_is_reached() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        &format!(
            "await page.goto({url})
const frame = page.mainFrame()
const waits = Promise.all([
  page.waitForSelector('#late', {{ state: 'attached', timeout: {MET_WITHIN_MS} }}),
  frame.waitForSelector('#late', {{ state: 'attached', timeout: {MET_WITHIN_MS} }}),
  page.waitForSelector('#doomed', {{ state: 'detached', timeout: {MET_WITHIN_MS} }}),
  frame.waitForSelector('#doomed', {{ state: 'detached', timeout: {MET_WITHIN_MS} }}),
  page.waitForSelector('#reveal', {{ timeout: {MET_WITHIN_MS} }}),
  frame.waitForSelector('#reveal', {{ state: 'visible', timeout: {MET_WITHIN_MS} }}),
  page.waitForSelector('#fade', {{ state: 'hidden', timeout: {MET_WITHIN_MS} }}),
  frame.waitForSelector('#fade', {{ state: 'hidden', timeout: {MET_WITHIN_MS} }}),
])
const before = await Promise.race([
  waits.then(() => 'settled', (error) => error.name),
  new Promise((resolve) => setTimeout(() => resolve('pending'), 500)),
])
await page.click('#change')
return {{ before, null: (await waits).map((value) => value === null) }}",
            url = json!(format!("{}/selectors", tab.origin))
        ),
    )
    .await;
    tab.close().await;

    assert_eq!(
        value(&outcome),
        json!({ "before": "pending", "null": [true, true, true, true, true, true, true, true] }),
        "{outcome:#?}"
    );
    let waits = records(&outcome, "waitForSelector");
    assert_eq!(waits.len(), 8, "{outcome:#?}");
    assert!(
        waits
            .iter()
            .all(|call| call.result == CallResult::Ok && call.delivery.is_none()),
        "{waits:#?}"
    );
}

/// Two matches, the first hidden. The page and frame forms judge the first, so `visible` times
/// out while `hidden` and `attached` resolve `null`, and `strict: true` refuses the pair.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_page_and_frame_selector_waits_judge_the_first_match_unless_strict() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        &format!(
            "await page.goto({url})
const settled = (wait) => wait.then(
  (value) => ({{ value }}),
  (error) => ({{ name: error.name, strict: /strict mode violation/.test(error.message) }}),
)
const frame = page.mainFrame()
return {{
  visible: await settled(page.waitForSelector('.pair', {{ timeout: 500 }})),
  hidden: await settled(frame.waitForSelector('.pair', {{ state: 'hidden', timeout: {MET_WITHIN_MS} }})),
  attached: await settled(page.waitForSelector('.pair', {{ state: 'attached', timeout: {MET_WITHIN_MS} }})),
  strict: await settled(page.waitForSelector('.pair', {{ strict: true, timeout: {MET_WITHIN_MS} }})),
  strictHidden: await settled(
    frame.waitForSelector('.pair', {{ strict: true, state: 'hidden', timeout: {MET_WITHIN_MS} }}),
  ),
}}",
            url = json!(format!("{}/selectors", tab.origin))
        ),
    )
    .await;
    tab.close().await;

    let strict = json!({ "name": "Error", "strict": true });
    assert_eq!(
        value(&outcome),
        json!({
            "visible": { "name": "TimeoutError", "strict": false },
            "hidden": { "value": null },
            "attached": { "value": null },
            "strict": strict,
            "strictHidden": strict,
        }),
        "{outcome:#?}"
    );
}

/// `waitForTimeout` waits at least its delay, in page and frame form, as a call that takes no
/// input turn.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_wait_for_timeout_waits_at_least_its_delay() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        "const started = Date.now()
const values = [await page.waitForTimeout(400), await page.mainFrame().waitForTimeout(200)]
return { undefined: values.map((value) => value === undefined), elapsed: Date.now() - started }",
    )
    .await;
    tab.close().await;

    let value = value(&outcome);
    assert_eq!(value["undefined"], json!([true, true]), "{value}");
    assert!(
        value["elapsed"]
            .as_u64()
            .is_some_and(|elapsed| elapsed >= 600),
        "{value}"
    );
    let waits = records(&outcome, "waitForTimeout");
    assert_eq!(waits.len(), 2, "{outcome:#?}");
    for (call, delay) in waits.into_iter().zip([400, 200]) {
        assert_eq!(call.result, CallResult::Ok, "{call:#?}");
        assert_eq!(call.delivery, None, "{call:#?}");
        assert!(
            call.duration >= Duration::from_millis(delay),
            "waited {:?} of {delay} ms",
            call.duration
        );
    }
}

/// The run's first call is `waitForTimeout(20000)`, so only its own `prepare` can have started
/// the event watcher that tells a run its tab closed. Once the run's session replays the
/// document's lifecycle, that watcher runs; closing the tab then rejects the wait as closed long
/// before its delay.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_closing_the_tab_ends_a_wait_for_timeout_that_was_the_runs_first_call() {
    let Some(tab) = open().await else { return };
    let mut events = tab.cdp.events();
    let run = tab
        .start_run(
            "return await page.waitForTimeout(20000).then(
  () => 'resolved',
  (error) => ({ name: error.name, closed: /has been closed/.test(error.message) }),
)"
            .to_string(),
        )
        .await;
    lifecycle_replayed(&mut events, &tab).await;
    // After the replay, `prepare` has one round trip left (focus emulation). A second later the
    // wait is on the rest of its timer, which is what the close must end.
    time::sleep(Duration::from_secs(1)).await;
    let closed_at = Instant::now();
    tab.cdp
        .send(
            "Target.closeTarget",
            json!({ "targetId": tab.target_id }),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("close the run's tab: {error}"));
    let outcome = run
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"));
    let returned_after = closed_at.elapsed();
    tab.close().await;

    assert_eq!(
        value(&outcome),
        json!({ "name": "Error", "closed": true }),
        "{outcome:#?}"
    );
    let [record] = records(&outcome, "waitForTimeout")[..] else {
        panic!("one waitForTimeout: {outcome:#?}");
    };
    assert!(
        matches!(&record.result, CallResult::Error { name, .. } if name == "TargetClosedError"),
        "{record:#?}"
    );
    assert!(
        returned_after < Duration::from_secs(10),
        "the run returned {returned_after:?} after the close"
    );
}

/// Waits up to 15 s for a `Page.lifecycleEvent` on a session other than the test's driver.
/// Chromium sends that replay to a run's session once its `prepare` enables lifecycle events.
async fn lifecycle_replayed(events: &mut broadcast::Receiver<CdpEvent>, tab: &Tab) {
    let replayed = time::timeout(Duration::from_secs(15), async {
        loop {
            match events.recv().await {
                Ok(event)
                    if event.method == "Page.lifecycleEvent"
                        && event
                            .session_id
                            .as_ref()
                            .is_some_and(|session| session.as_str() != tab.driver.as_str()) =>
                {
                    return;
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => panic!("the CDP connection closed"),
            }
        }
    })
    .await;
    assert!(
        replayed.is_ok(),
        "no run session replayed the document's lifecycle"
    );
}
