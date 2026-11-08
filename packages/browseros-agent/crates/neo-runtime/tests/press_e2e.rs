//! `press` on a fresh headless BrowserOS neo, through Neo alone: the run's outcome (its value,
//! or its error's kind), the field's value or the URL, every key event the page saw (`key`,
//! `code`, `keyCode` and the modifier flags), and each call's delivery. Wording, call logs, CDP
//! traffic and timing are never compared; `neo-quickjs-injected/tests/parity/press.test.mjs`
//! compares the same methods with Playwright on one browser.

#[path = "support/browser.rs"]
mod browser;

use std::time::{Duration, Instant};

use browser::{Fixture, Tab};
use neo_runtime::{CallResult, Delivery, RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::time;

/// The form's results page answers after this long, so its submission stays in flight well past
/// a press and a `page.url()` read, however loaded the machine (the harness holds `/slow…`).
const SUBMISSION_IN_FLIGHT: Duration = Duration::from_secs(1);

/// What the page records of each event; kept in session storage, so a submission's events
/// survive its navigation.
const RECORDER: &str = r"<script>
window.__events = JSON.parse(sessionStorage.getItem('events') || '[]');
for (const type of ['keydown', 'keyup', 'input', 'submit'])
  addEventListener(type, e => {
    __events.push({ type: e.type, target: e.target.id || e.target.tagName.toLowerCase(), key: e.key ?? null, code: e.code ?? null, keyCode: e.keyCode ?? null, shift: e.shiftKey ?? null, meta: e.metaKey ?? null, control: e.ctrlKey ?? null, inputType: e.inputType ?? null, data: e.data ?? null, at: e.timeStamp });
    sessionStorage.setItem('events', JSON.stringify(__events));
  }, true);
</script>";

fn page() -> String {
    format!(
        "<!doctype html><html><head>{RECORDER}</head><body>\
<form action='/slow-results' method='get'><input id='q' name='q' value='neo'></form>\
<input id='name'></body></html>"
    )
}

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let results =
        format!("<!doctype html><html><head>{RECORDER}</head><body>Results</body></html>");
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: vec![
                    ("/press.html".into(), page()),
                    ("/slow-results?q=neo".into(), results),
                ],
                slow_response: SUBMISSION_IN_FLIGHT,
            },
            "/press.html",
        )
        .await,
    )
}

async fn run(tab: &Tab, code: &str) -> RunOutcome {
    tab.start_run(code.to_string())
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"))
}

/// The page's key events as `[type, target, key, code, keyCode, shift, meta, control]`.
fn keys(events: &Value) -> Vec<Value> {
    events
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|event| event["type"] == "keydown" || event["type"] == "keyup")
        .map(|event| {
            json!([
                event["type"],
                event["target"],
                event["key"],
                event["code"],
                event["keyCode"],
                event["shift"],
                event["meta"],
                event["control"]
            ])
        })
        .collect()
}

/// The deliveries of the run's press calls, in order.
fn press_deliveries(outcome: &RunOutcome) -> Vec<Option<Delivery>> {
    outcome
        .calls
        .iter()
        .filter(|call| call.method == "press")
        .map(|call| call.delivery)
        .collect()
}

/// Reads `expression` once the tab has settled on `path`, retrying reads that race a commit.
async fn settled(tab: &Tab, path: &str, expression: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    let probe = format!("[location.pathname + location.search, {expression}]");
    loop {
        let reply = tab
            .cdp
            .send(
                "Runtime.evaluate",
                json!({ "expression": probe, "returnByValue": true }),
                Some(&tab.driver),
            )
            .await;
        if let Ok(reply) = reply
            && reply.get("exceptionDetails").is_none()
            && reply["result"]["value"][0] == path
        {
            return reply["result"]["value"][1].clone();
        }
        assert!(Instant::now() < deadline, "the tab never settled on {path}");
        time::sleep(Duration::from_millis(20)).await;
    }
}

/// Enter in a form's field submits it; a press that waits returns once the results page has
/// committed, and one with `noWaitAfter` returns on the old URL while the submission is held.
#[tokio::test(flavor = "multi_thread")]
async fn enter_submits_the_form_and_only_a_waiting_press_returns_on_the_new_url() {
    let Some(tab) = open().await else { return };
    let script = |options: &str| {
        format!(
            "const result = await page.locator('#q').press('Enter'{options})\n\
             return {{ undefined: result === undefined, url: page.url() }}"
        )
    };
    let waited = run(&tab, &script("")).await;
    let waited_events = settled(&tab, "/slow-results?q=neo", "window.__events").await;
    tab.evaluate("sessionStorage.clear()").await;
    tab.navigate("/press.html").await;
    let unwaited = run(&tab, &script(", { noWaitAfter: true }")).await;
    let unwaited_events = settled(&tab, "/slow-results?q=neo", "window.__events").await;
    let origin = tab.origin.clone();
    tab.close().await;

    assert_eq!(waited.status, RunStatus::Ok, "{waited:#?}");
    assert_eq!(
        waited.value,
        Some(json!({ "undefined": true, "url": format!("{origin}/slow-results?q=neo") }))
    );
    assert_eq!(unwaited.status, RunStatus::Ok, "{unwaited:#?}");
    assert_eq!(
        unwaited.value,
        Some(json!({ "undefined": true, "url": format!("{origin}/press.html") }))
    );
    for (events, outcome) in [(&waited_events, &waited), (&unwaited_events, &unwaited)] {
        assert_eq!(
            keys(events),
            [
                json!(["keydown", "q", "Enter", "Enter", 13, false, false, false]),
                json!(["keyup", "q", "Enter", "Enter", 13, false, false, false]),
            ]
        );
        let submits = events
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter(|event| event["type"] == "submit")
            .count();
        assert_eq!(submits, 1, "{events}");
        assert_eq!(press_deliveries(outcome), [Some(Delivery::Done)]);
    }
}

/// `Shift+A` types `A` with Shift held; `ControlOrMeta+a` selects the text (Meta on mac) and
/// `Backspace` deletes it.
#[tokio::test(flavor = "multi_thread")]
async fn shortcuts_type_and_edit_what_they_name() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        "const name = page.locator('#name')
await name.press('Shift+A')
const typed = await name.inputValue()
await name.press('ControlOrMeta+a')
await name.press('Backspace')
return { typed, cleared: await name.inputValue() }",
    )
    .await;
    let events = tab.evaluate("window.__events").await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    assert_eq!(outcome.value, Some(json!({ "typed": "A", "cleared": "" })));
    let (modifier, code, key_code, meta, control) = if cfg!(target_os = "macos") {
        ("Meta", "MetaLeft", 91, true, false)
    } else {
        ("Control", "ControlLeft", 17, false, true)
    };
    assert_eq!(
        keys(&events),
        [
            json!([
                "keydown",
                "name",
                "Shift",
                "ShiftLeft",
                16,
                true,
                false,
                false
            ]),
            json!(["keydown", "name", "A", "KeyA", 65, true, false, false]),
            json!(["keyup", "name", "A", "KeyA", 65, true, false, false]),
            json!([
                "keyup",
                "name",
                "Shift",
                "ShiftLeft",
                16,
                false,
                false,
                false
            ]),
            json!([
                "keydown", "name", modifier, code, key_code, false, meta, control
            ]),
            json!(["keydown", "name", "a", "KeyA", 65, false, meta, control]),
            json!(["keyup", "name", "a", "KeyA", 65, false, meta, control]),
            json!([
                "keyup", "name", modifier, code, key_code, false, false, false
            ]),
            json!([
                "keydown",
                "name",
                "Backspace",
                "Backspace",
                8,
                false,
                false,
                false
            ]),
            json!([
                "keyup",
                "name",
                "Backspace",
                "Backspace",
                8,
                false,
                false,
                false
            ]),
        ]
    );
    let inputs: Vec<Value> = events
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|event| event["type"] == "input")
        .map(|event| json!([event["inputType"], event["data"]]))
        .collect();
    assert_eq!(
        inputs,
        [
            json!(["insertText", "A"]),
            json!(["deleteContentBackward", null])
        ]
    );
    assert_eq!(press_deliveries(&outcome), [Some(Delivery::Done); 3]);
}

/// `delay` holds the key down between its keydown and keyup.
#[tokio::test(flavor = "multi_thread")]
async fn a_delayed_press_holds_its_key_for_the_delay() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        "await page.locator('#name').press('a', { delay: 300 })
return await page.locator('#name').inputValue()",
    )
    .await;
    let events = tab.evaluate("window.__events").await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    assert_eq!(outcome.value, Some(json!("a")));
    let keyed: Vec<&Value> = events
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|event| event["type"] == "keydown" || event["type"] == "keyup")
        .collect();
    let [down, up] = keyed.as_slice() else {
        panic!("one keydown and one keyup: {events}");
    };
    let held = up["at"].as_f64().unwrap_or_default() - down["at"].as_f64().unwrap_or_default();
    assert!(held >= 290.0, "a held for {held} ms: {events}");
    assert_eq!(press_deliveries(&outcome), [Some(Delivery::Done)]);
}

/// A key with an unknown token rejects with an `Error` after the focus, and no key ever goes
/// down: not even the Shift before it, which Playwright presses and leaves held.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_key_rejects_without_a_key_event() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        "try {
  await page.locator('#name').press('Shift+Foo')
  return 'resolved'
} catch (error) {
  return error.name
}",
    )
    .await;
    let facts = tab
        .evaluate(
            "[window.__events, document.activeElement.id, document.querySelector('#name').value]",
        )
        .await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    assert_eq!(outcome.value, Some(json!("Error")));
    assert_eq!(facts, json!([[], "name", ""]), "focused, nothing pressed");
    let [press] = outcome.calls.as_slice() else {
        panic!("one press: {outcome:#?}");
    };
    assert!(
        matches!(&press.result, CallResult::Error { name, .. } if name == "Error"),
        "{press:?}"
    );
    assert_eq!(press.delivery, Some(Delivery::NotSent));
}

/// A press whose run ends in the middle of its `delay` still releases its keys: the page sees a
/// keyup for every keydown, the next press types an unshifted `a`, and the record's delivery is
/// the writer's (every key answered: `Done`).
#[tokio::test(flavor = "multi_thread")]
async fn a_run_that_ends_mid_delay_leaves_no_key_down() {
    let Some(tab) = open().await else { return };
    let cut = tab
        .start_run_within(
            "await page.locator('#name').press('Shift+a', { delay: 5000 })".to_string(),
            Duration::from_millis(1500),
        )
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"));
    let next = run(&tab, "await page.locator('#name').press('a')").await;
    let events = tab.evaluate("window.__events").await;
    let value = tab.evaluate("document.querySelector('#name').value").await;
    tab.close().await;

    assert_eq!(cut.status, RunStatus::Timeout, "{cut:?}");
    assert_eq!(press_deliveries(&cut), [Some(Delivery::Done)], "{cut:#?}");
    assert_eq!(next.status, RunStatus::Ok, "{next:?}");
    assert_eq!(
        value,
        json!("aa"),
        "a typed while Shift was held, then a without it"
    );
    assert_eq!(
        keys(&events),
        [
            json!([
                "keydown",
                "name",
                "Shift",
                "ShiftLeft",
                16,
                true,
                false,
                false
            ]),
            json!(["keydown", "name", "a", "KeyA", 65, true, false, false]),
            json!(["keyup", "name", "a", "KeyA", 65, true, false, false]),
            json!([
                "keyup",
                "name",
                "Shift",
                "ShiftLeft",
                16,
                false,
                false,
                false
            ]),
            json!(["keydown", "name", "a", "KeyA", 65, false, false, false]),
            json!(["keyup", "name", "a", "KeyA", 65, false, false, false]),
        ]
    );
}
