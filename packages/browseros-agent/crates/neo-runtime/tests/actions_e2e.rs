//! Neo: action labels survive the full SDK, browser and call-record path.

#[expect(
    dead_code,
    reason = "the shared fixture also provides navigation helpers"
)]
#[path = "support/browser.rs"]
mod browser;

use browser::{Fixture, Tab};
use neo_runtime::RunStatus;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_browser_calls_record_their_action_kind_and_group() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/").await;
    let outcome = tab
        .start_run(format!(
            r#"
        await page.goto({url});
        await page.setContent('<input id="name"><button id="go">Go</button><p id="text">Hello</p>');
        await page.locator('#name').fill('Neo');
        await page.locator('#name').click();
        await page.keyboard.type('!');
        const value = await page.evaluate(() => document.querySelector('#name').value);
        const text = await page.locator('#text').textContent();
        await page.locator('#go').dispatchEvent('click');
        return {{ value, text }};
    "#,
            url = json!(format!("{}/?actions", tab.origin))
        ))
        .await
        .await
        .unwrap_or_else(|error| panic!("run task: {error}"));
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    assert_eq!(
        outcome.value,
        Some(json!({"value": "Neo!", "text": "Hello"})),
        "browser effects"
    );
    let actual: Vec<Value> = outcome
        .calls
        .iter()
        .map(|call| {
            let record =
                serde_json::to_value(call).unwrap_or_else(|error| panic!("record JSON: {error}"));
            record["action"].clone()
        })
        .collect();
    assert_eq!(
        actual,
        vec![
            json!({"kind": "goto", "group": "navigate"}),
            json!({"kind": "setContent", "group": "pageScript"}),
            json!({"kind": "fill", "group": "type"}),
            json!({"kind": "click", "group": "pointer"}),
            json!({"kind": "keyboard", "group": "type"}),
            json!({"kind": "evaluate", "group": "pageScript"}),
            json!({"kind": "textContent", "group": "read"}),
            json!({"kind": "dispatchEvent", "group": "pageScript"}),
        ],
        "every wire call has its fixed action label"
    );
}
