//! Regression for context bindings owned by concurrent runs in different browser contexts.
//! Matches the Playwright control in binding-two-context-runs.test.mjs: disposing A leaves
//! A's name undefined while B's independent same-named binding still answers B.

#[path = "support/browser.rs"]
#[expect(dead_code, reason = "only the browser fixture is used")]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use browseros_cdp::{CdpClient, SessionId};
use neo_runtime::{PageId, RunRequest, Runtime};
use serde_json::{Value, json};
use tokio::time;
use tokio_util::sync::CancellationToken;

async fn evaluate(cdp: &CdpClient, session: &SessionId, expression: &str) -> Value {
    let result = cdp
        .send(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
            Some(session),
        )
        .await
        .unwrap_or_else(|error| panic!("driver evaluates the page: {error}"));
    assert!(result.get("exceptionDetails").is_none(), "{result}");
    result["result"]["value"].clone()
}

async fn ready(cdp: &CdpClient, session: &SessionId) {
    time::timeout(Duration::from_secs(15), async {
        while evaluate(cdp, session, "window.ready === true").await != json!(true) {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|error| panic!("the run registers its binding: {error}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_binding_in_another_browser_context_does_not_keep_an_ended_runs_name() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/bindings").await;
    let created = tab
        .cdp
        .send("Target.createBrowserContext", json!({}), None)
        .await
        .expect("context B");
    let context = created["browserContextId"].as_str().expect("context id");
    let target = tab
        .cdp
        .send(
            "Target.createTarget",
            json!({
                "url": format!("{}/bindings", tab.origin), "browserContextId": context,
            }),
            None,
        )
        .await
        .expect("page B");
    let target = target["targetId"].as_str().expect("target id");
    let attached = tab
        .cdp
        .send(
            "Target.attachToTarget",
            json!({"targetId": target, "flatten": true}),
            None,
        )
        .await
        .expect("driver B");
    let driver = SessionId::from(attached["sessionId"].as_str().expect("session id"));
    let runtime = Runtime::connect(tab.cdp.clone()).await.expect("runtime");
    let cancel_a = CancellationToken::new();
    let cancel_b = CancellationToken::new();
    let start = |page, label, cancel| {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime.run(RunRequest {
                code: format!("await context.exposeFunction('shared', () => '{label}'); await page.evaluate(() => window.ready = true); await new Promise(() => {{}})"),
                page,
                timeout: Duration::from_mins(1),
                cancel,
                artifacts: None,
                hooks: None,
            }).await
        })
    };
    let a = start(PageId::new(tab.target_id.clone()), "A", cancel_a.clone());
    ready(&tab.cdp, &tab.driver).await;
    let b = start(PageId::new(target.to_string()), "B", cancel_b.clone());
    ready(&tab.cdp, &driver).await;
    let before = [
        evaluate(&tab.cdp, &tab.driver, "shared()").await,
        evaluate(&tab.cdp, &driver, "shared()").await,
    ];
    cancel_a.cancel();
    a.await.expect("run A ends");
    let after = [
        evaluate(&tab.cdp, &tab.driver, "typeof shared").await,
        evaluate(&tab.cdp, &driver, "shared()").await,
    ];
    cancel_b.cancel();
    b.await.expect("run B ends");
    runtime.drain().await;
    tab.cdp
        .send(
            "Target.disposeBrowserContext",
            json!({"browserContextId": context}),
            None,
        )
        .await
        .expect("close context B");
    tab.close().await;
    assert_eq!(
        before,
        [json!("A"), json!("B")],
        "each live run answers its own binding"
    );
    assert_eq!(
        after,
        [json!("undefined"), json!("B")],
        "another context is not a remaining owner"
    );
}
