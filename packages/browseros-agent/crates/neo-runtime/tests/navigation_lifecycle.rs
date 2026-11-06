//! Navigation outcomes and page effects through the runtime, with a scripted browser socket.

#[path = "support/fake_browser.rs"]
#[expect(
    dead_code,
    reason = "Other suites use the shared fake's traffic inspection helpers"
)]
mod fake_browser;

use std::time::Duration;

use fake_browser::{CommitDuring, FakeBrowser, NavigationScript, Replies, Reply};
use neo_runtime::RunStatus;
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_reads_retry_when_a_commit_spans_the_reply_or_attachment() {
    for during in [CommitDuring::Title, CommitDuring::Attach] {
        let browser = FakeBrowser::start(Replies {
            commit_during: Some(during),
            title: Reply {
                delay: Duration::from_millis(50),
                ..Reply::default()
            },
            world: Reply {
                delay: Duration::from_millis(50),
                ..Reply::default()
            },
            ..Replies::default()
        })
        .await;
        let outcome = browser
            .runtime()
            .await
            .run(fake_browser::run_request(
                "return await page.title()",
                Duration::from_secs(10),
                CancellationToken::new(),
            ))
            .await;
        assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
        assert_eq!(outcome.value, Some(json!("After commit")));
    }
}

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
                restored_from_bfcache: stop_loading,
            }),
            ..Replies::default()
        })
        .await;
        let runtime = browser.runtime().await;
        let outcome = runtime.run(fake_browser::run_request(
            "const before=await page.title(); const result=await page.goto('https://example.test/next',{referer:'https://from.test/',timeout:2000}); return {before,isNull:result===null,title:await page.title(),url:page.url()}",
            Duration::from_secs(10), CancellationToken::new(),
        )).await;
        assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
        assert_eq!(browser.page_clicks(), 0);
        assert_eq!(browser.armed_interceptor(), None);
        assert_eq!(
            outcome.value,
            Some(
                json!({"before":"Fake page","isNull":true,"title":"Fake page","url":"https://example.test/next"})
            )
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
            restored_from_bfcache: false,
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
