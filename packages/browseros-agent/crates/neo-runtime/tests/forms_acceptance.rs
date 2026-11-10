//! The forms acceptance flow on a fresh headless BrowserOS neo, through Neo's public methods
//! only: fill a search box, clear and refill it, read its value, press Enter to submit the form
//! (the run sees the new URL), then on a second page toggle a checkbox both ways and select
//! single and multiple options. The checkbox's state is read by the test over its own CDP
//! session (`isChecked` is not served).

#[expect(
    dead_code,
    reason = "shared browser harness also serves protocol_e2e.rs, which reads the browser's port"
)]
#[path = "support/browser.rs"]
mod browser;

use browser::{Fixture, Tab};
use neo_runtime::{Delivery, RunOutcome, RunStatus};
use serde_json::json;

const SEARCH: &str = r"<!doctype html><html><body>
<form action='/results' method='get'>
  <label for='q'>Search</label><input id='q' name='q'><button>Search</button>
</form>
</body></html>";

const OPTIONS: &str = r"<!doctype html><html><body>
<label><input id='agree' type='checkbox'> I agree</label>
<select id='size'><option value='s'>Small</option><option value='m'>Medium</option><option value='l'>Large</option></select>
<select id='toppings' multiple><option value='cheese'>Cheese</option><option value='olives'>Olives</option><option value='basil'>Basil</option></select>
</body></html>";

async fn run(tab: &Tab, code: &str) -> RunOutcome {
    let outcome = tab
        .start_run(code.to_string())
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"));
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    for call in &outcome.calls {
        let expected = (call.method != "inputValue").then_some(Delivery::Done);
        assert_eq!(call.delivery, expected, "{call:#?}");
    }
    outcome
}

#[tokio::test(flavor = "multi_thread")]
async fn a_search_form_and_its_options_work_end_to_end() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![
                ("/search.html".into(), SEARCH.into()),
                ("/options.html".into(), OPTIONS.into()),
            ],
            ..Fixture::default()
        },
        "/search.html",
    )
    .await;

    let searched = run(
        &tab,
        r"const query = page.getByLabel('Search')
await query.fill('first query')
await query.clear()
const cleared = await query.inputValue()
await query.fill('neo browser')
const value = await query.inputValue()
await query.press('Enter')
return { cleared, value, url: page.url() }",
    )
    .await;
    let landed = tab.evaluate("location.pathname + location.search").await;

    tab.navigate("/options.html").await;
    run(&tab, "await page.locator('#agree').check()").await;
    let checked = tab
        .evaluate("document.querySelector('#agree').checked")
        .await;
    run(&tab, "await page.locator('#agree').uncheck()").await;
    let unchecked = tab
        .evaluate("document.querySelector('#agree').checked")
        .await;
    let selected = run(
        &tab,
        r"const single = await page.locator('#size').selectOption('m')
const multiple = await page.locator('#toppings').selectOption(['cheese', 'basil'])
return { single, multiple }",
    )
    .await;
    let shown = tab
        .evaluate(
            "[document.querySelector('#size').value, [...document.querySelector('#toppings').selectedOptions].map(o => o.value)]",
        )
        .await;
    let origin = tab.origin.clone();
    tab.close().await;

    assert_eq!(
        searched.value,
        Some(json!({
            "cleared": "",
            "value": "neo browser",
            "url": format!("{origin}/results?q=neo+browser"),
        }))
    );
    assert_eq!(landed, json!("/results?q=neo+browser"));
    assert_eq!((checked, unchecked), (json!(true), json!(false)));
    assert_eq!(
        selected.value,
        Some(json!({ "single": ["m"], "multiple": ["cheese", "basil"] }))
    );
    assert_eq!(shown, json!(["m", ["cheese", "basil"]]));
}
