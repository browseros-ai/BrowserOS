//! Checked-state actions on a fresh headless BrowserOS neo. These tests assert checked state,
//! normal page events, error categories and delivery, independent of diagnostic wording.

#[expect(
    dead_code,
    reason = "shared fixture includes navigation helpers other suites use"
)]
#[path = "support/browser.rs"]
mod browser;

use browser::{Fixture, Tab};
use neo_runtime::{Delivery, RunOutcome, RunStatus};
use serde_json::json;

const CHECKS: &str = include_str!("../../neo-sdk/tests/pages/check-actions.html");

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: vec![("/checks.html".into(), CHECKS.into())],
                ..Fixture::default()
            },
            "/checks.html",
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

fn assert_error(outcome: &RunOutcome, name: &str, category: &str, delivery: Delivery) {
    assert_eq!(outcome.status, RunStatus::Error, "{outcome:#?}");
    let error = outcome
        .error
        .as_ref()
        .unwrap_or_else(|| panic!("missing error: {outcome:#?}"));
    assert_eq!(error.name, name, "the public error category");
    assert!(error.message.contains(category), "{error:?}");
    assert_eq!(outcome.calls.len(), 1, "one public action");
    assert_eq!(
        outcome.calls[0].delivery,
        Some(delivery),
        "the action's delivery"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_check_and_uncheck_toggle_a_checkbox_and_return_undefined() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, "const box=page.locator('#unchecked'); return [await box.check()===undefined, await box.uncheck()===undefined]").await;
    let facts = tab.evaluate("window.__facts()").await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!([true, true])));
    assert_eq!(facts["checked"]["unchecked"], false);
    let events = facts["events"].as_array().expect("events");
    assert_eq!(
        events
            .iter()
            .map(|e| e["type"].as_str().expect("type"))
            .collect::<Vec<_>>(),
        ["click", "input", "change", "click", "input", "change"]
    );
    assert!(
        outcome
            .calls
            .iter()
            .all(|call| call.delivery == Some(Delivery::Done))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_an_already_checked_box_sends_no_click() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        "return await page.locator('#checked').check() === undefined",
    )
    .await;
    let facts = tab.evaluate("window.__facts()").await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!(true)));
    assert_eq!(facts["checked"]["checked"], true);
    assert_eq!(facts["events"], json!([]));
    assert_eq!(outcome.calls[0].delivery, Some(Delivery::NotSent));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_check_selects_a_radio_in_its_group() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        "return await page.locator('#radio').check() === undefined",
    )
    .await;
    let facts = tab.evaluate("window.__facts()").await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!(true)));
    assert_eq!(facts["checked"]["radio"], true);
    assert_eq!(facts["checked"]["radio-checked"], false);
    assert_eq!(outcome.calls[0].delivery, Some(Delivery::Done));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_uncheck_rejects_a_checked_radio_without_input() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, "await page.locator('#radio-checked').uncheck()").await;
    let facts = tab.evaluate("window.__facts()").await;
    tab.close().await;
    assert_error(&outcome, "Error", "radio", Delivery::NotSent);
    assert_eq!(facts["checked"]["radio-checked"], true);
    assert_eq!(facts["events"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_role_checkbox_handlers_can_change_checked_state_both_ways() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, "const box=page.locator('#aria'); return [await box.check()===undefined, await box.uncheck()===undefined]").await;
    let facts = tab.evaluate("window.__facts()").await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!([true, true])));
    assert_eq!(facts["aria"], "false");
    assert_eq!(
        facts["events"].as_array().expect("events").len(),
        2,
        "both clicks reached the handler"
    );
    assert!(
        outcome
            .calls
            .iter()
            .all(|call| call.delivery == Some(Delivery::Done))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_prevented_checkbox_click_is_delivered_but_rejected() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, "await page.locator('#prevent').check()").await;
    let facts = tab.evaluate("window.__facts()").await;
    tab.close().await;
    assert_error(&outcome, "Error", "did not change", Delivery::Done);
    assert_eq!(facts["checked"]["prevent"], false);
    assert_eq!(
        facts["events"].as_array().expect("events").len(),
        1,
        "one prevented click, without retry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_plain_div_is_rejected_without_input() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, "await page.locator('#plain').check()").await;
    let events = tab.evaluate("window.__events").await;
    tab.close().await;
    assert_error(&outcome, "Error", "checkbox", Delivery::NotSent);
    assert_eq!(events, json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_covered_checkbox_waits_then_times_out_without_delivery() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, "await page.locator('#covered').check({timeout:1000})").await;
    let facts = tab.evaluate("window.__facts()").await;
    tab.close().await;
    assert_error(&outcome, "TimeoutError", "", Delivery::NotSent);
    assert_eq!(facts["checked"]["covered"], false);
    assert_eq!(facts["events"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_trial_leaves_the_checkbox_unchanged_without_page_clicks() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        "return await page.locator('#trial').check({trial:true}) === undefined",
    )
    .await;
    let facts = tab.evaluate("window.__facts()").await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!(true)));
    assert_eq!(facts["checked"]["trial"], false);
    assert_eq!(facts["events"], json!([]));
    assert_eq!(outcome.calls[0].delivery, Some(Delivery::NotSent));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_set_checked_delegates_both_states_for_every_receiver() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, "return [await page.setChecked('#unchecked',true)===undefined, await page.mainFrame().setChecked('#unchecked',false)===undefined, await page.locator('#unchecked').setChecked(true)===undefined, await page.locator('#unchecked').setChecked(false)===undefined]").await;
    let facts = tab.evaluate("window.__facts()").await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!([true, true, true, true])));
    assert_eq!(facts["checked"]["unchecked"], false);
    assert!(
        outcome
            .calls
            .iter()
            .all(|call| call.delivery == Some(Delivery::Done))
    );
}
