//! `press` on a fresh headless BrowserOS neo, against what Playwright did on the same pages
//! (`neo-sdk/tests/goldens/form-goldens.json`, recorded with Playwright 26137face). Every press
//! case runs its recorded `source` as a Neo run on its recorded fixture, and the test compares
//! what a script and its page can observe: the outcome, the return value, the error's name and
//! category, the fields' values and selections, the focused element, the URL, and every key
//! and input event the page saw (`key`, `code`, `keyCode`, modifiers, `inputType`, `data`).
//! Messages, call logs and timing are not compared.
//!
//! Neo's one deliberate difference is checked as such: for a key with an unknown token Neo
//! sends no key at all, where Playwright pressed the tokens before it and left them held.
//!
//! The replay's forms submit to a path the fixture server holds for [`SUBMISSION_IN_FLIGHT`]
//! (the harness answers `/slow…` paths late); paths are compared as recorded. With the
//! capture's prompt `/submitted`, what `page.url()` reads right after a `noWaitAfter` press
//! was a race, which a loaded machine lost: the submission committed first. Held in flight, the
//! old URL is what a press that does not wait must return, and a press that waits returns only
//! once the new one has committed.

#[path = "support/browser.rs"]
mod browser;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use browser::{Fixture, Tab};
use neo_runtime::{Delivery, RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::time;

/// The capture's `pageFacts` (`capture-forms.mjs`), read the same way after each Neo run.
const FACTS: &str = r"(() => {
  const name = t => !t ? null : t === document.body ? 'body' : t.id || t.tagName.toLowerCase();
  const controls = [...document.querySelectorAll('input, textarea, select, [contenteditable], [role=checkbox], [role=radio]')].map(el => {
    const control = { id: el.id || null, tag: el.tagName.toLowerCase() };
    if (el instanceof HTMLInputElement) {
      control.type = el.type;
      if (el.type === 'checkbox' || el.type === 'radio') control.checked = el.checked; else control.value = el.value;
      if (el.selectionStart !== null) control.selection = [el.selectionStart, el.selectionEnd];
    } else if (el instanceof HTMLTextAreaElement) {
      control.value = el.value;
      control.selection = [el.selectionStart, el.selectionEnd];
    }
    return control;
  });
  return { path: location.pathname + location.search, activeElement: name(document.activeElement), controls, events: window.__events ?? null };
})()";

fn goldens() -> Value {
    let crate_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR")
            .unwrap_or_else(|| panic!("cargo sets CARGO_MANIFEST_DIR")),
    );
    let path = crate_dir.join("../neo-sdk/tests/goldens/form-goldens.json");
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("the form goldens are committed: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("form-goldens.json parses: {error}"))
}

fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} is a string in {value}"))
}

/// Where the capture's forms submit, and where the replay's do instead.
const SUBMITTED: &str = "/submitted";
const SLOW_SUBMITTED: &str = "/slow-submitted";

/// How long the fixture server holds a submission: far longer than a press takes to answer and
/// `page.url()` to read, however loaded the machine.
const SUBMISSION_IN_FLIGHT: Duration = Duration::from_secs(2);

/// A recorded path as the replay serves it.
fn served(path: &str) -> String {
    path.strip_prefix(SUBMITTED).map_or_else(
        || path.to_string(),
        |rest| format!("{SLOW_SUBMITTED}{rest}"),
    )
}

/// A replayed path as the capture recorded it.
fn as_recorded(path: &str) -> String {
    path.strip_prefix(SLOW_SUBMITTED)
        .map_or_else(|| path.to_string(), |rest| format!("{SUBMITTED}{rest}"))
}

/// The page the capture's forms submit to: the fixtures' recorder on an empty body.
fn submitted_page(fixture_html: &str) -> String {
    let start = fixture_html
        .find("</title>")
        .unwrap_or_else(|| panic!("a title"))
        + "</title>".len();
    let end = fixture_html
        .find("</head>")
        .unwrap_or_else(|| panic!("a head"));
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>submitted</title>{}</head><body></body></html>",
        &fixture_html[start..end]
    )
}

/// The error categories of the capture (`capture-forms.mjs` `categorize`), from the message
/// without its call log.
fn category(name: &str, message: &str) -> &'static str {
    let message = message.split("\nCall log:").next().unwrap_or(message);
    if name == "TimeoutError" {
        "timeout"
    } else if message.contains("strict mode violation") {
        "strict"
    } else if message.contains("Unknown key: ") {
        "unknown-key"
    } else {
        "other"
    }
}

/// The key and input events of a recorded event list, with the fields a key's effect shows.
fn key_events(events: &Value) -> Vec<Value> {
    let fields = [
        "type",
        "target",
        "key",
        "code",
        "keyCode",
        "charCode",
        "location",
        "repeat",
        "modifiers",
        "inputType",
        "data",
        "value",
    ];
    events
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|event| {
            matches!(
                event["type"].as_str(),
                Some("keydown" | "keypress" | "keyup" | "beforeinput" | "input" | "submit")
            )
        })
        .map(|event| {
            Value::Object(
                fields
                    .iter()
                    .filter_map(|field| Some((field.to_string(), event.get(*field)?.clone())))
                    .collect(),
            )
        })
        .collect()
}

/// The facts both sides are compared on: fields, focus, the URL's path, and key events.
fn observed(facts: &Value) -> Value {
    json!({
        "path": as_recorded(facts["path"].as_str().unwrap_or_default()),
        "activeElement": facts["activeElement"],
        "controls": facts["controls"],
        "keyEvents": key_events(&facts["events"]),
    })
}

/// What the golden's page showed, as [`observed`] reads Neo's.
fn recorded(case: &Value) -> Value {
    let page = &case["page"];
    let controls: Vec<Value> = page["controls"]
        .as_array()
        .unwrap_or_else(|| panic!("controls in {case}"))
        .iter()
        .map(|control| {
            let mut control = control.clone();
            if let Some(fields) = control.as_object_mut() {
                fields.retain(|key, _| {
                    ["id", "tag", "type", "value", "checked", "selection"].contains(&key.as_str())
                });
            }
            control
        })
        .collect();
    let mut key_events = key_events(&page["events"]);
    if case["outcome"] == "threw" && case["error"]["category"] == "unknown-key" {
        // Neo plans the whole key first: Playwright's keyDowns before the unknown token are
        // the one difference, and Neo sends none.
        key_events.retain(|event| event["type"] != "keydown");
    }
    json!({
        "path": page["urlRelative"],
        "activeElement": page["activeElement"],
        "controls": controls,
        "keyEvents": key_events,
    })
}

/// Reads the page facts once the page has settled on `path` (a submission may still be
/// committing), retrying reads that race a navigation.
async fn facts_at(tab: &Tab, path: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let reply = tab
            .cdp
            .send(
                "Runtime.evaluate",
                json!({ "expression": FACTS, "returnByValue": true }),
                Some(&tab.driver),
            )
            .await;
        if let Ok(reply) = reply
            && reply.get("exceptionDetails").is_none()
            && reply["result"]["value"]["path"] == path
        {
            return reply["result"]["value"].clone();
        }
        assert!(
            Instant::now() < deadline,
            "the page never settled on {path}"
        );
        time::sleep(Duration::from_millis(20)).await;
    }
}

async fn run(tab: &Tab, source: &str) -> RunOutcome {
    tab.start_run(source.to_string())
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"))
}

/// How the run's outcome differs from the recorded one: status, return value (a URL compared
/// from its path), error name and category, and every call's delivery.
fn outcome_mismatches(name: &str, case: &Value, outcome: &RunOutcome, origin: &str) -> Vec<String> {
    let mut mismatches = Vec::new();
    let deliveries = |expected| {
        outcome
            .calls
            .iter()
            .all(|call| call.delivery == Some(expected))
    };
    if case["outcome"] == "resolved" {
        if outcome.status != RunStatus::Ok {
            mismatches.push(format!("{name}: {outcome:?}"));
        }
        let value = outcome.value.clone().unwrap_or(Value::Null);
        let value = match value.as_str() {
            Some(url) => json!(as_recorded(url.strip_prefix(origin).unwrap_or(url))),
            None => value,
        };
        let recorded = case
            .get("returnValueRelative")
            .cloned()
            .unwrap_or(Value::Null);
        if value != recorded {
            mismatches.push(format!("{name}: returned {value}, Playwright {recorded}"));
        }
        if !deliveries(Delivery::Done) {
            mismatches.push(format!("{name}: deliveries {:?}", outcome.calls));
        }
        return mismatches;
    }
    let Some(error) = &outcome.error else {
        return vec![format!("{name}: resolved, Playwright threw: {outcome:?}")];
    };
    let recorded = (
        text(&case["error"], "name"),
        text(&case["error"], "category"),
    );
    if (error.name.as_str(), category(&error.name, &error.message)) != recorded {
        mismatches.push(format!("{name}: {error:?}, Playwright {recorded:?}"));
    }
    if !deliveries(Delivery::NotSent) {
        mismatches.push(format!("{name}: deliveries {:?}", outcome.calls));
    }
    mismatches
}

/// Every press case of the goldens, through Neo, on one browser: each case on a fresh document
/// of its own fixture, with the recorder's session storage cleared first.
#[tokio::test(flavor = "multi_thread")]
async fn every_recorded_press_case_has_playwrights_outcome_and_page_effect() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let goldens = goldens();
    let cases: Vec<&Value> = goldens["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .filter(|case| case["method"] == "press")
        .collect();
    assert_eq!(cases.len(), 10, "the recorded press cases");
    let mut pages = Vec::new();
    for case in &cases {
        let html = text(case, "fixtureHtml");
        let action = format!("action=\"{SUBMITTED}\"");
        let replayed = html.replace(&action, &format!("action=\"{SLOW_SUBMITTED}\""));
        pages.push((format!("/{}.html", text(case, "name")), replayed));
        pages.push((served("/submitted?q=neo"), submitted_page(html)));
    }
    let tab = Tab::open(
        binary,
        Fixture {
            pages,
            slow_response: SUBMISSION_IN_FLIGHT,
        },
        "/blank",
    )
    .await;
    let mut mismatches = Vec::new();
    for case in &cases {
        let name = text(case, "name");
        tab.evaluate("sessionStorage.clear()").await;
        tab.navigate(&format!("/{name}.html")).await;
        tab.evaluate("window.__record({ type: 'call' })").await;
        let outcome = run(&tab, text(case, "source")).await;
        let expected = recorded(case);
        let facts = facts_at(&tab, &served(expected["path"].as_str().expect("a path"))).await;
        let actual = observed(&facts);
        if actual != expected {
            mismatches.push(format!(
                "{name}: page\n  neo:        {actual}\n  playwright: {expected}"
            ));
        }
        mismatches.extend(outcome_mismatches(name, case, &outcome, &tab.origin));
    }
    tab.close().await;
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// A press whose run ends in the middle of its `delay` still releases its keys: the page sees a
/// keyup for every keydown, the next press types an unshifted `a`, and the record's delivery is
/// the writer's (every key answered: `Done`).
#[tokio::test(flavor = "multi_thread")]
async fn a_run_that_ends_mid_delay_leaves_no_key_down() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let html = r"<!doctype html><input id='name'><script>
window.__keys = [];
for (const type of ['keydown', 'keyup'])
  addEventListener(type, e => __keys.push([e.type, e.code, e.shiftKey]), true);
</script>";
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![("/keys.html".into(), html.into())],
            ..Fixture::default()
        },
        "/keys.html",
    )
    .await;
    let cut = tab
        .start_run_within(
            "await page.locator('#name').press('Shift+a', { delay: 5000 })".to_string(),
            Duration::from_millis(1500),
        )
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"));
    let next = run(&tab, "await page.locator('#name').press('a')").await;
    let keys = tab.evaluate("window.__keys").await;
    let value = tab.evaluate("document.querySelector('#name').value").await;
    tab.close().await;

    assert_eq!(cut.status, RunStatus::Timeout, "{cut:?}");
    let [press] = cut.calls.as_slice() else {
        panic!("one press: {cut:#?}");
    };
    assert_eq!(press.delivery, Some(Delivery::Done), "{press:?}");
    assert_eq!(next.status, RunStatus::Ok, "{next:?}");
    assert_eq!(
        value,
        json!("aa"),
        "a typed while Shift was held, then a without it"
    );
    assert_eq!(
        keys,
        json!([
            ["keydown", "ShiftLeft", true],
            ["keydown", "KeyA", true],
            ["keyup", "KeyA", true],
            ["keyup", "ShiftLeft", false],
            ["keydown", "KeyA", false],
            ["keyup", "KeyA", false],
        ])
    );
}
