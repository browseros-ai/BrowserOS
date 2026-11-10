//! Form actions on a fresh headless BrowserOS neo. Public results, normal DOM effects,
//! retries and delivery are checked; diagnostic wording and retry timing are not contracts.

#[expect(
    dead_code,
    reason = "shared fixture includes navigation helpers other suites use"
)]
#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{CallResult, Delivery, RunOutcome, RunStatus};
use serde_json::json;
use tokio::time;

const FORM: &str = include_str!("../../neo-quickjs-injected/tests/pages/form-actions.html");

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: vec![("/forms.html".into(), FORM.into())],
                ..Fixture::default()
            },
            "/forms.html",
        )
        .await,
    )
}

async fn run(tab: &Tab, source: &str) -> RunOutcome {
    tab.start_run(source.into())
        .await
        .await
        .unwrap_or_else(|error| panic!("run task: {error}"))
}

fn assert_ok(outcome: &RunOutcome) {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_date_fill_and_clear_deliver_the_set_value_path() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        r"
      const date = page.locator('#date')
      const filled = await date.fill('2026-11-06') === undefined
      const value = await date.inputValue()
      const cleared = await date.clear() === undefined
      return { filled, value, cleared, empty: await date.inputValue() }
    ",
    )
    .await;
    let facts = tab.evaluate("window.__facts()").await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(
        outcome.value,
        Some(json!({"filled":true,"value":"2026-11-06","cleared":true,"empty":""}))
    );
    assert_eq!(facts["values"]["date"], "");
    assert!(
        outcome
            .calls
            .iter()
            .filter(|call| call.method == "fill")
            .all(|call| call.delivery == Some(Delivery::Done))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_fill_clear_refill_reads_the_last_value_and_uses_real_input() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        r"
      const field = page.getByLabel('Name')
      const voids = [await field.fill('first') === undefined,
        await field.clear() === undefined, await field.fill('last') === undefined]
      return { voids, value: await field.inputValue() }
    ",
    )
    .await;
    let facts = tab.evaluate("window.__facts()").await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(
        outcome.value,
        Some(json!({"voids":[true,true,true],"value":"last"}))
    );
    assert_eq!(facts["values"]["text"], "last");
    let events = facts["events"].as_array().expect("recorded events");
    assert!(events.iter().any(|event| event["type"] == "beforeinput"));
    assert!(
        events
            .iter()
            .any(|event| event["type"] == "keydown" && event["key"] == "Delete")
    );
    assert!(
        events
            .iter()
            .any(|event| event["type"] == "keyup" && event["key"] == "Delete")
    );
    assert!(
        outcome
            .calls
            .iter()
            .filter(|call| call.method == "fill")
            .all(|call| call.delivery == Some(Delivery::Done))
    );
    assert_eq!(outcome.calls.last().expect("read").delivery, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_select_returns_values_for_single_multiple_label_index_and_deselection() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        r"
      const single = page.locator('#single')
      return [await page.selectOption('#single', 'two'),
        await page.mainFrame().selectOption('#single', {label:'First'}),
        await single.selectOption({index:1}), await single.selectOption({value:'one'}),
        await single.selectOption('Second'),
        await page.locator('#multiple').selectOption(['three','one']),
        await single.selectOption(null)]
    ",
    )
    .await;
    let facts = tab.evaluate("window.__facts()").await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(
        outcome.value,
        Some(json!([
            ["two"],
            ["one"],
            ["two"],
            ["one"],
            ["two"],
            ["one", "three"],
            []
        ]))
    );
    assert_eq!(facts["values"]["single"], "");
    assert_eq!(facts["selected"], json!(["one", "three"]));
    assert!(
        outcome
            .calls
            .iter()
            .all(|call| call.delivery == Some(Delivery::Done))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_input_value_reads_live_values_and_follows_labels_without_delivery() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        r"
      return [await page.inputValue('#text'), await page.mainFrame().inputValue('#area'),
        await page.locator('#single').inputValue(), await page.locator('label').inputValue()]
    ",
    )
    .await;
    let events = tab.evaluate("window.__events").await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!(["seed", "seed", "one", "seed"])));
    assert!(outcome.calls.iter().all(|call| call.delivery.is_none()));
    assert_eq!(events, json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_validation_and_strict_failures_reject_without_delivery() {
    let Some(tab) = open().await else { return };
    let scripts = [
        "await page.locator('#number').fill('abc')",
        "await page.locator('#checkbox').fill('x')",
        "await page.locator('#plain').fill('x')",
        "await page.locator('#plain').selectOption('one')",
        "await page.locator('#plain').inputValue()",
        "await page.locator('.duplicate').fill('x')",
        "await page.locator('.duplicate').inputValue()",
    ];
    let mut outcomes = Vec::new();
    for source in scripts {
        outcomes.push(run(&tab, source).await);
    }
    let facts = tab.evaluate("window.__facts()").await;
    tab.close().await;
    for outcome in outcomes {
        assert_eq!(outcome.status, RunStatus::Error, "{outcome:#?}");
        assert_eq!(outcome.error.as_ref().expect("error").name, "Error");
        assert!(
            outcome
                .calls
                .iter()
                .all(|call| call.delivery.is_none() || call.delivery == Some(Delivery::NotSent))
        );
    }
    assert_eq!(facts["values"]["text"], "seed");
    assert_eq!(facts["values"]["number"], "");
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_unready_fields_and_options_timeout_without_delivery() {
    let Some(tab) = open().await else { return };
    let scripts = [
        "await page.locator('#disabled').fill('x', {timeout:500})",
        "await page.locator('#readonly').fill('x', {timeout:500})",
        "await page.locator('#missing').fill('x', {timeout:500})",
        "await page.locator('#single').selectOption('missing', {timeout:500})",
        "await page.locator('#disabled-option').selectOption('two', {timeout:500})",
        "await page.locator('#missing').inputValue({timeout:500})",
    ];
    let mut outcomes = Vec::new();
    for source in scripts {
        outcomes.push(run(&tab, source).await);
    }
    let facts = tab.evaluate("window.__facts()").await;
    tab.close().await;
    for outcome in outcomes {
        assert_eq!(outcome.status, RunStatus::Error, "{outcome:#?}");
        assert_eq!(outcome.error.as_ref().expect("error").name, "TimeoutError");
        assert!(
            outcome
                .calls
                .iter()
                .all(|call| call.delivery.is_none() || call.delivery == Some(Delivery::NotSent))
        );
    }
    assert_eq!(facts["values"]["disabled"], "seed");
    assert_eq!(facts["values"]["readonly"], "seed");
    assert_eq!(facts["values"]["disabled-option"], "one");
    assert_eq!(facts["events"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_fill_waits_for_editability_and_refinds_a_replaced_element() {
    let Some(tab) = open().await else { return };
    let running = tab.start_run("await page.locator('#disabled').fill('replacement', {timeout:5000}); return await page.locator('#disabled').inputValue()".into()).await;
    time::sleep(Duration::from_millis(100)).await;
    tab.evaluate("document.querySelector('#disabled').outerHTML = '<input id=disabled>'")
        .await;
    let replaced = running.await.expect("run");
    let running = tab.start_run("await page.locator('#readonly').fill('editable', {timeout:5000}); return await page.locator('#readonly').inputValue()".into()).await;
    time::sleep(Duration::from_millis(100)).await;
    tab.evaluate("document.querySelector('#readonly').readOnly = false")
        .await;
    let editable = running.await.expect("run");
    tab.close().await;
    assert_ok(&replaced);
    assert_eq!(replaced.value, Some(json!("replacement")));
    assert_ok(&editable);
    assert_eq!(editable.value, Some(json!("editable")));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_select_waits_for_an_option_to_appear_or_become_enabled() {
    let Some(tab) = open().await else { return };
    let running = tab
        .start_run(
            "return await page.locator('#single').selectOption('later', {timeout:5000})".into(),
        )
        .await;
    time::sleep(Duration::from_millis(100)).await;
    tab.evaluate("document.querySelector('#single').add(new Option('Later','later'))")
        .await;
    let appeared = running.await.expect("run");
    let running = tab
        .start_run(
            "return await page.locator('#disabled-option').selectOption('two', {timeout:5000})"
                .into(),
        )
        .await;
    time::sleep(Duration::from_millis(100)).await;
    tab.evaluate("document.querySelector('#disabled-option option[value=two]').disabled = false")
        .await;
    let enabled = running.await.expect("run");
    tab.close().await;
    assert_ok(&appeared);
    assert_eq!(appeared.value, Some(json!(["later"])));
    assert_ok(&enabled);
    assert_eq!(enabled.value, Some(json!(["two"])));
    assert!(
        appeared
            .calls
            .iter()
            .chain(&enabled.calls)
            .all(|call| call.result == CallResult::Ok && call.delivery == Some(Delivery::Done))
    );
}
