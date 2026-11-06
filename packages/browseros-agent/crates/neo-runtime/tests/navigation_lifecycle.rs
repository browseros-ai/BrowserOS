//! CDP ordering and wire replies through the real runtime, with a scripted browser socket.

#[path = "support/fake_browser.rs"]
mod fake_browser;

use std::time::Duration;

use fake_browser::{FakeBrowser, NavigationScript, Replies};
use neo_runtime::RunStatus;
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_cdp_commits_before_and_after_the_reply_complete_navigation() {
    for (reply, commit, same_document, stop_loading) in [
        (50, 5, false, false),
        (5, 50, false, false),
        (50, 5, true, false),
        (5, 50, false, true),
    ] {
        let browser = FakeBrowser::start(Replies {
            navigation: Some(NavigationScript {
                reply_after: Duration::from_millis(reply),
                commit_after: Duration::from_millis(commit),
                same_document,
                stop_loading,
            }),
            ..Replies::default()
        })
        .await;
        let runtime = browser.runtime().await;
        let outcome = runtime.run(fake_browser::run_request(
            "const before=await page.title(); const result=await page.goto('https://example.test/next',{referer:'https://from.test/',timeout:2000}); return {before,isNull:result===null,title:await page.title()}",
            Duration::from_secs(10), CancellationToken::new(),
        )).await;
        assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
        assert_eq!(browser.page_clicks(), 0);
        assert_eq!(browser.armed_interceptor(), None);
        assert_eq!(
            outcome.value,
            Some(json!({"before":"Fake page","isNull":true,"title":"Fake page"}))
        );
        let exchanges = browser.exchanges();
        let navigation = exchanges
            .iter()
            .find(|exchange| exchange.method() == "Page.navigate")
            .expect("navigation sent");
        assert_eq!(
            navigation.command["params"],
            json!({"url":"https://example.test/next","referrer":"https://from.test/","referrerPolicy":"unsafeUrl","frameId":fake_browser::PAGE})
        );
        assert!(
            !exchanges
                .iter()
                .any(|exchange| exchange.method() == "Input.dispatchMouseEvent")
        );
        assert_eq!(
            exchanges
                .iter()
                .filter(|exchange| exchange.method() == "Page.createIsolatedWorld")
                .count(),
            if same_document { 1 } else { 2 },
            "cross-document commit must invalidate even a still-live old glue"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_dispatch_sends_generated_goto_and_title_results() {
    let browser = FakeBrowser::start(Replies {
        navigation: Some(NavigationScript {
            reply_after: Duration::from_millis(20),
            commit_after: Duration::from_millis(1),
            same_document: false,
            stop_loading: false,
        }),
        ..Replies::default()
    })
    .await;
    let runtime = browser.runtime().await;
    let outcome = runtime.run(fake_browser::run_request(
        "const goto=JSON.parse(await __neo_runtime.call(JSON.stringify({id:1,guid:'frame@fake-page',method:'goto',params:{url:'https://example.test/next'},metadata:{timeout:2000}}))); const title=JSON.parse(await __neo_runtime.call(JSON.stringify({id:2,guid:'frame@fake-page',method:'title',params:{},metadata:{timeout:0}}))); return {goto,title}",
        Duration::from_secs(10), CancellationToken::new(),
    )).await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value,
        Some(json!({"goto":{"id":1,"result":{}},"title":{"id":2,"result":{"value":"Fake page"}}}))
    );
}
