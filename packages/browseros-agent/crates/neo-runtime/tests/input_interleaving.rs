//! Input from many calls and runs on one tab of a fresh headless BrowserOS neo: every input
//! call takes the tab's input turn (`page.rs` `TabInputs`, shared by the runs of one `Runtime`),
//! so fills, key presses, checks, selections and clicks never split one another's input, turns
//! go in arrival order, a call that stops waiting for its turn sends nothing, a press cut short
//! still releases its keys, and a fill cut short mid-insertText still inserts its whole value
//! before the next input. Behaviour only: page-seen events, values and delivery records.

#[expect(
    dead_code,
    reason = "shared fixture includes navigation helpers other suites use"
)]
#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{CallResult, Delivery, PageId, RunOutcome, RunRequest, RunStatus, Runtime};
use serde_json::{Value, json};
use tokio::time;
use tokio_util::sync::CancellationToken;

/// Every control the tests type into, and a recorder of the events input produces, by target.
/// No `change`: a text field fires it when it loses focus, during the next call's input.
/// `#hold` marks `#flag` when its press lands, so a script can see that click holding the turn.
const PAGE: &str = r#"<!doctype html><html><body>
<button id='hold' onmousedown="document.getElementById('flag').value = 'held'">Hold</button>
<button id='c'>Click</button>
<input id='a'><input id='b'><input id='f'><input id='p'><input id='late'><input id='flag'>
<input id='cb' type='checkbox'>
<select id='s'><option value='one'>One</option><option value='two'>Two</option></select>
<script>
window.__events = [];
for (const type of ['mousedown', 'mouseup', 'click', 'keydown', 'keypress', 'keyup', 'beforeinput', 'input'])
  addEventListener(type, e => __events.push({ type: e.type, target: e.target.id, key: e.key ?? null, data: e.data ?? null, shift: e.shiftKey ?? null }), true);
</script>
</body></html>"#;

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: vec![("/inputs.html".into(), PAGE.into())],
                ..Fixture::default()
            },
            "/inputs.html",
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

fn request(tab: &Tab, code: &str, timeout: Duration, cancel: CancellationToken) -> RunRequest {
    RunRequest {
        code: code.to_string(),
        page: PageId::new(tab.target_id.clone()),
        timeout,
        cancel,
    }
}

fn assert_ok(outcome: &RunOutcome) {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
}

/// The recorded events, as `(target, type, data)`.
fn events(recorded: &Value) -> Vec<(String, String, Value)> {
    recorded
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|event| {
            (
                event["target"].as_str().unwrap_or_default().to_string(),
                event["type"].as_str().unwrap_or_default().to_string(),
                event["data"].clone(),
            )
        })
        .collect()
}

/// Two runs started together on one tab, one filling `#a` and one pressing `Shift+B` into `#b`,
/// ten times each: every fill lands whole in `#a`, every `B` in `#b`, and neither field ever
/// sees the other's input.
#[tokio::test(flavor = "multi_thread")]
async fn two_runs_typing_into_two_fields_never_mix() {
    let Some(tab) = open().await else { return };
    let runtime = runtime(&tab).await;
    let filling = "for (let i = 0; i < 10; i++) await page.locator('#a').fill('alpha' + i)";
    let pressing = "for (let i = 0; i < 10; i++) await page.locator('#b').press('Shift+B')";
    let timeout = Duration::from_secs(30);
    let (filled, pressed) = tokio::join!(
        runtime.run(request(&tab, filling, timeout, CancellationToken::new())),
        runtime.run(request(&tab, pressing, timeout, CancellationToken::new())),
    );
    let values = tab
        .evaluate("[document.querySelector('#a').value, document.querySelector('#b').value]")
        .await;
    let recorded = events(&tab.evaluate("window.__events").await);
    tab.close().await;

    assert_ok(&filled);
    assert_ok(&pressed);
    assert_eq!(values, json!(["alpha9", "BBBBBBBBBB"]));
    for (target, kind, data) in &recorded {
        match target.as_str() {
            "a" => assert!(
                !kind.starts_with("key")
                    && (data.is_null()
                        || data.as_str().is_some_and(|data| data.starts_with("alpha"))),
                "{kind} {data} reached #a: {recorded:?}"
            ),
            "b" => assert!(
                data.is_null() || *data == json!("B"),
                "{kind} {data} reached #b: {recorded:?}"
            ),
            other => panic!("input reached #{other}: {recorded:?}"),
        }
    }
    let typed: Vec<_> = recorded
        .iter()
        .filter(|(target, kind, _)| target == "b" && kind == "input")
        .collect();
    assert_eq!(typed.len(), 10, "{recorded:?}");
    for outcome in [&filled, &pressed] {
        assert!(
            outcome
                .calls
                .iter()
                .all(|call| call.delivery == Some(Delivery::Done)),
            "{:#?}",
            outcome.calls
        );
    }
}

/// A press cancelled in the middle of its 500 ms `delay` still sends both keyUps: the page sees
/// no key left down, the next press types an unshifted `a`, and the record's delivery is the
/// writer's (every key answered).
#[tokio::test(flavor = "multi_thread")]
async fn a_press_cancelled_mid_delay_still_releases_its_keys() {
    let Some(tab) = open().await else { return };
    let runtime = runtime(&tab).await;
    let cancel = CancellationToken::new();
    let stopper = {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            time::sleep(Duration::from_millis(300)).await;
            cancel.cancel();
        })
    };
    let cut = runtime
        .run(request(
            &tab,
            "await page.locator('#p').press('Shift+a', { delay: 500 })",
            Duration::from_secs(30),
            cancel,
        ))
        .await;
    stopper
        .await
        .unwrap_or_else(|error| panic!("the stopper: {error}"));
    let next = runtime
        .run(request(
            &tab,
            "await page.locator('#p').press('a')",
            Duration::from_secs(30),
            CancellationToken::new(),
        ))
        .await;
    let recorded = tab.evaluate("window.__events").await;
    let value = tab.evaluate("document.querySelector('#p').value").await;
    tab.close().await;

    assert_eq!(cut.status, RunStatus::Cancelled, "{cut:#?}");
    let [press] = cut.calls.as_slice() else {
        panic!("one press: {cut:#?}");
    };
    assert_eq!(
        press.delivery,
        Some(Delivery::Done),
        "both keyDowns and both keyUps were answered: {press:?}"
    );
    assert_ok(&next);
    let keys: Vec<(String, bool)> = recorded
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|event| event["type"] == "keydown" || event["type"] == "keyup")
        .map(|event| {
            (
                format!(
                    "{} {}",
                    event["type"].as_str().unwrap_or_default(),
                    event["key"].as_str().unwrap_or_default()
                ),
                event["shift"].as_bool().unwrap_or_default(),
            )
        })
        .collect();
    let downs = keys
        .iter()
        .filter(|(key, _)| key.starts_with("keydown"))
        .count();
    let ups = keys
        .iter()
        .filter(|(key, _)| key.starts_with("keyup"))
        .count();
    assert_eq!(downs, ups, "no key is left down: {keys:?}");
    assert_eq!(
        keys.last().cloned(),
        Some(("keyup a".to_string(), false)),
        "{keys:?}"
    );
    assert_eq!(
        value,
        json!("aa"),
        "a typed while Shift was held, then a without it"
    );
}

/// While a click holds the tab's input through its 2.5 s press, a click, a fill, a check, a
/// selection and a press queue in that order. Each one's input reaches the page whole and after
/// the one before it, and a press that gives up waiting for its turn sends nothing.
///
/// Arrival order is the order the calls ask for the turn, after their own element waits. The
/// queue starts only once the holding click is pressing (its `mousedown` marks `#flag`, read
/// with `inputValue`, which takes no turn), and each call starts 300 ms after the one before,
/// far longer than any of them takes to find its element.
#[tokio::test(flavor = "multi_thread")]
async fn queued_inputs_take_turns_in_arrival_order() {
    let Some(tab) = open().await else { return };
    let outcome = tab
        .start_run(
            r"const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const hold = page.locator('#hold').click({ delay: 2500 });
while (await page.locator('#flag').inputValue() !== 'held') await pause(10);
const click = page.locator('#c').click();
await pause(300);
const fill = page.locator('#f').fill('typed');
await pause(300);
const check = page.locator('#cb').check();
await pause(300);
const select = page.locator('#s').selectOption('two');
await pause(300);
const press = page.locator('#p').press('x');
await pause(300);
const late = page.locator('#late').press('y', { timeout: 100 }).then(() => 'resolved', error => error.name);
const results = await Promise.all([hold, click, fill, check, select, press, late]);
return results.map(result => result === undefined ? 'undefined' : result);"
                .to_string(),
        )
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"));
    let recorded = events(&tab.evaluate("window.__events").await);
    let values = tab
        .evaluate("['#f', '#p', '#cb', '#s', '#late'].map(id => document.querySelector(id)).map(e => e.type === 'checkbox' ? e.checked : e.value)")
        .await;
    tab.close().await;

    assert_ok(&outcome);
    assert_eq!(
        outcome.value,
        Some(json!([
            "undefined",
            "undefined",
            "undefined",
            "undefined",
            ["two"],
            "undefined",
            "TimeoutError"
        ]))
    );
    assert_eq!(values, json!(["typed", "x", true, "two", ""]));
    let mut groups: Vec<&str> = Vec::new();
    for (target, _, _) in &recorded {
        if groups.last() != Some(&target.as_str()) {
            groups.push(target);
        }
    }
    assert_eq!(
        groups,
        ["hold", "c", "f", "cb", "s", "p"],
        "each call's input whole, in arrival order: {recorded:?}"
    );
    let late = outcome
        .calls
        .iter()
        .find(|call| call.params["selector"] == "#late")
        .unwrap_or_else(|| panic!("the late press: {:#?}", outcome.calls));
    assert_eq!(late.delivery, Some(Delivery::NotSent), "{late:?}");
    assert!(
        matches!(&late.result, CallResult::Error { name, .. } if name == "TimeoutError"),
        "{late:?}"
    );
    let inputs: Vec<_> = outcome
        .calls
        .iter()
        .filter(|call| call.method != "inputValue" && call.params["selector"] != "#late")
        .collect();
    assert_eq!(inputs.len(), 6, "{:#?}", outcome.calls);
    assert!(
        inputs
            .iter()
            .all(|call| call.delivery == Some(Delivery::Done)),
        "{inputs:#?}"
    );
}

/// A slow page: `#a`'s handler keeps the renderer busy for 3 s inside `Input.insertText`, so a
/// fill of `#a` with a 1.5 s timeout is still waiting for its insertText's answer when the
/// timeout passes, however long (under 1.5 s) the fill took to reach its insertText.
const SLOW_FILL_PAGE: &str = r"<!doctype html><html><body>
<input id='a'><input id='b'>
<script>
window.__events = [];
for (const type of ['beforeinput', 'input'])
  addEventListener(type, e => __events.push([e.type, e.target.id, (e.data ?? '').length]), true);
document.getElementById('a').addEventListener('beforeinput', () => {
  const end = performance.now() + 3000;
  while (performance.now() < end);
});
</script>
</body></html>";

/// A fill of a large value whose call times out in the middle of its `Input.insertText` still
/// inserts the whole value, and the next fill, issued once that call has stopped and while its
/// writer still waits, lands whole on another field after it. Each record's delivery is what
/// its writer saw: the stopped fill's text was answered, so `Done`.
#[tokio::test(flavor = "multi_thread")]
async fn a_fill_stopped_mid_insert_text_lands_whole_before_the_next() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![("/slow-fill.html".into(), SLOW_FILL_PAGE.into())],
            ..Fixture::default()
        },
        "/slow-fill.html",
    )
    .await;
    let outcome = tab
        .start_run(
            r"const large = 'x'.repeat(50000);
const first = await page.locator('#a').fill(large, { timeout: 1500 }).then(() => 'resolved', error => error.name);
const next = await page.locator('#b').fill('second');
return [first, next === undefined];"
                .to_string(),
        )
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"));
    let values = tab
        .evaluate("[document.querySelector('#a').value.length, document.querySelector('#b').value]")
        .await;
    let recorded = tab.evaluate("window.__events").await;
    tab.close().await;

    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!(["TimeoutError", true])));
    assert_eq!(values, json!([50000, "second"]));
    assert_eq!(
        recorded,
        json!([
            ["beforeinput", "a", 50000],
            ["input", "a", 50000],
            ["beforeinput", "b", 6],
            ["input", "b", 6],
        ]),
        "the next fill lands after the whole first value"
    );
    let [stopped, next] = outcome.calls.as_slice() else {
        panic!("two fills: {:#?}", outcome.calls);
    };
    assert!(
        matches!(&stopped.result, CallResult::Error { name, .. } if name == "TimeoutError"),
        "{stopped:?}"
    );
    assert_eq!(stopped.delivery, Some(Delivery::Done), "{stopped:?}");
    assert_eq!(
        (&next.result, next.delivery),
        (&CallResult::Ok, Some(Delivery::Done)),
        "{next:?}"
    );
}
