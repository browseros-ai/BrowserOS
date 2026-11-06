//! Public state reads and checkbox changes in one real Neo run on a throwaway browser profile.

#[expect(
    dead_code,
    reason = "the shared fixture also provides navigation helpers"
)]
#[path = "support/browser.rs"]
mod browser;

use browser::{Fixture, Tab};
use neo_runtime::{Delivery, RunStatus};
use serde_json::json;

const STATES: &str = r"<!doctype html><html><body>
<button id='visible'>Visible</button>
<div id='display-hidden' style='display:none'>Hidden</div>
<div id='visibility-hidden' style='visibility:hidden'>Hidden</div>
<input id='unchecked' type='checkbox'>
<input id='checked' type='checkbox' checked><label id='label' for='checked'>Checked</label>
<div id='aria' role='checkbox' aria-checked='true'>ARIA checkbox</div>
<div id='plain'>Plain element</div>
</body></html>";

const FLOW: &str = r"const receivers = [
  {
    isVisible: (selector, options) => page.locator(selector).isVisible(options),
    isChecked: (selector, options) => page.locator(selector).isChecked(options),
  },
  page,
  page.mainFrame(),
]
const typed = value => [value, typeof value]
const visible = []
const before = []
for (const receiver of receivers) {
  const visibility = []
  for (const selector of ['#visible', '#display-hidden', '#visibility-hidden', '#missing'])
    visibility.push(typed(await receiver.isVisible(selector, { timeout: -1 })))
  visible.push(visibility)
  const checked = []
  for (const selector of ['#unchecked', '#checked', '#label', '#aria'])
    checked.push(typed(await receiver.isChecked(selector)))
  before.push(checked)
}
await page.locator('#unchecked').check()
await page.locator('#checked').uncheck()
const after = []
for (const receiver of receivers)
  after.push([
    typed(await receiver.isChecked('#unchecked')),
    typed(await receiver.isChecked('#checked')),
  ])
async function rejection(action) {
  try { await action(); return { name: 'resolved', category: false } }
  catch (error) { return { name: error.name, message: error.message } }
}
const wrong = await rejection(() => page.locator('#plain').isChecked())
const missing = await rejection(() => page.locator('#missing').isChecked({ timeout: 500 }))
const strict = await rejection(() => page.locator('#visible, #plain').isVisible())
const strictChecked = await rejection(() => page.locator('#unchecked, #checked').isChecked())
const invalidVisible = await rejection(() => page.locator('[').isVisible())
const invalidChecked = await rejection(() => page.locator('[').isChecked())
return {
  visible, before, after,
  errors: [
    wrong.name,
    [missing.name, true],
    [strict.name, strict.message.includes('strict mode violation')],
    [strictChecked.name, strictChecked.message.includes('strict mode violation')],
    [invalidVisible.name, true],
    [invalidChecked.name, true],
  ],
}";

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_state_reads_and_checkbox_changes_work_through_every_receiver() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![("/states.html".into(), STATES.into())],
            ..Fixture::default()
        },
        "/states.html",
    )
    .await;
    let outcome = tab
        .start_run(FLOW.to_string())
        .await
        .await
        .unwrap_or_else(|error| panic!("run task: {error}"));
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    let visibility = json!([
        [true, "boolean"],
        [false, "boolean"],
        [false, "boolean"],
        [false, "boolean"]
    ]);
    let before = json!([
        [false, "boolean"],
        [true, "boolean"],
        [true, "boolean"],
        [true, "boolean"]
    ]);
    let after = json!([[true, "boolean"], [false, "boolean"]]);
    assert_eq!(
        outcome.value,
        Some(json!({
            "visible": [visibility, visibility, visibility],
            "before": [before, before, before],
            "after": [after, after, after],
            "errors": [
                "Error",
                ["TimeoutError", true],
                ["Error", true],
                ["Error", true],
                ["Error", true],
                ["Error", true]
            ]
        })),
        "values, boolean types and public error categories"
    );
    for call in &outcome.calls {
        let expected = match call.method.as_str() {
            "isVisible" | "isChecked" => None,
            "check" | "uncheck" => Some(Delivery::Done),
            method => panic!("unexpected call: {method}"),
        };
        assert_eq!(call.delivery, expected, "{call:#?}");
    }
}
