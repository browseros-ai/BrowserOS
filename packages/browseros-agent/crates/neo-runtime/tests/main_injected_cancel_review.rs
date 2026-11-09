//! Main-world selector polling ends with an explicit run cancellation. The predicate must run
//! first, then remain stopped after cancellation; a later run can still use the same tab.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "the shared fixture covers methods this check does not use"
)]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{PageId, RunRequest, RunStatus, Runtime};
use serde_json::json;
use tokio::time;
use tokio_util::sync::CancellationToken;

const PAGE: &str = r#"<!doctype html><button id="cancel-review-target" onclick="window.__clicks++">button</button>
<script>window.__selectorPolls=0;window.__clicks=0</script>"#;
const START_LIMIT: Duration = Duration::from_secs(30);
const SETTLE: Duration = Duration::from_millis(200);
const CHECK: Duration = Duration::from_millis(20);
const SOURCE: &str = r"const frame=page.mainFrame();await frame._connection.sendEval(
  'locator.waitForFunction',frame._guid,'waitForFunction',{
    selector:'#cancel-review-target',strict:true,
    expression:'element => { window.__selectorPolls++; return false }',isFunction:true,
    arg:{value:{v:'undefined'},handles:[]}
  },0);return 'unreachable'";

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_selector_polling_stops_when_its_run_is_cancelled() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![("/cancel-review".to_string(), PAGE.to_string())],
            ..Fixture::default()
        },
        "/cancel-review",
    )
    .await;
    let runtime = Runtime::connect(tab.cdp.clone()).await.expect("runtime");
    let cancel = CancellationToken::new();
    let request = RunRequest {
        code: SOURCE.to_string(),
        page: PageId::new(tab.target_id.clone()),
        timeout: Duration::from_mins(1),
        cancel: cancel.clone(),
        artifacts: None,
        hooks: None,
    };
    let running = tokio::spawn(async move {
        let outcome = runtime.run(request).await;
        runtime.drain().await;
        outcome
    });
    let started = time::timeout(START_LIMIT, async {
        loop {
            if tab.evaluate("window.__selectorPolls > 0").await == json!(true) {
                break;
            }
            time::sleep(CHECK).await;
        }
    })
    .await;
    assert!(started.is_ok(), "the predicate ran before cancellation");
    cancel.cancel();
    let outcome = running.await.expect("run task");
    assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:?}");
    let saved = tab.evaluate("window.__selectorPolls").await;
    assert!(saved.as_u64().is_some_and(|n| n > 0));
    time::sleep(SETTLE).await;
    assert_eq!(
        tab.evaluate("window.__selectorPolls").await,
        saved,
        "no polling after cancellation"
    );
    let next = tab.start_run("await (await page.$('#cancel-review-target')).dispatchEvent('click');return await page.evaluate(()=>window.__clicks)".to_string()).await.await.expect("next run");
    assert_eq!(next.status, RunStatus::Ok, "{next:?}");
    assert_eq!(next.value, Some(json!(1)));
    tab.close().await;
}
