//! Locator handlers and their subscriptions belong to one run of a shared Runtime.
#[expect(dead_code, reason = "shared browser helpers serve other suites")]
#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{PageId, RunRequest, RunStatus, Runtime};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn request(tab: &Tab, code: &str) -> RunRequest {
    RunRequest {
        page: PageId::new(tab.target_id.clone()),
        code: code.to_string(),
        timeout: Duration::from_secs(8),
        cancel: CancellationToken::new(),
        session: None,
        artifacts: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn handlers_end_with_a_run_including_a_throwing_handler() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture {
        pages: vec![("/handlers.html".into(),
            "<div id=banner>Banner</div><button id=target onclick='window.clicked++'>Target</button><script>window.clicked=0;window.handled=0</script>".into())],
        ..Fixture::default()
    }, "/handlers.html").await;
    let runtime = Runtime::connect(tab.cdp.clone()).await.expect("runtime");
    let registered = runtime.run(request(&tab,
        "await page.addLocatorHandler(page.locator('#banner'), async()=>{await page.evaluate(()=>window.handled++)}, {noWaitAfter:true}); return 'registered'"
    )).await;
    let next = runtime.run(request(&tab,
        "await page.locator('#target').click(); return await page.evaluate(()=>({clicked,handled}))"
    )).await;
    let failed = runtime.run(request(&tab,
        "await page.addLocatorHandler(page.locator('#banner'), ()=>{throw new Error('review-handler-failed')}, {noWaitAfter:true}); await page.locator('#target').click()"
    )).await;
    let before = tab
        .evaluate("window.clicked")
        .await
        .as_i64()
        .expect("click count");
    let recovered = runtime.run(request(&tab,
        "await page.locator('#target').click(); return await page.evaluate(()=>({clicked,handled}))"
    )).await;
    tab.close().await;
    assert_eq!(registered.status, RunStatus::Ok, "{registered:?}");
    assert_eq!(next.status, RunStatus::Ok, "{next:?}");
    assert_eq!(next.value, Some(json!({"clicked":1,"handled":0})));
    assert_eq!(failed.status, RunStatus::Error, "{failed:?}");
    assert_eq!(
        failed.error.as_ref().map(|error| error.name.as_str()),
        Some("Error")
    );
    assert!(
        failed
            .error
            .as_ref()
            .is_some_and(|error| error.message.contains("review-handler-failed")),
        "{failed:?}"
    );
    assert_eq!(recovered.status, RunStatus::Ok, "{recovered:?}");
    assert_eq!(
        recovered.value,
        Some(json!({"clicked":before+1,"handled":0}))
    );
}
