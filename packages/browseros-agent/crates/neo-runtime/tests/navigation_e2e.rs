//! Navigation, titles and document freshness on an isolated real BrowserOS neo profile.

#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::time;

const ONE: &str = r#"<!doctype html><title>First</title><h1>First page</h1>
<a id="next" href="/two">Next</a><a id="slow" href="/slow">Slow</a>
<script>
document.documentElement.dataset.historyDocument = crypto.randomUUID();
window.__restored = false; addEventListener('pageshow', e => window.__restored = e.persisted)
</script>"#;
const TWO: &str = r#"<!doctype html><title>Second</title><h1>Second page</h1>
<button id="go" onclick="document.title='Clicked'">Go</button>
<button id="back" onclick="history.back()">Back</button>"#;

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                slow_response: Duration::from_secs(5),
                pages: vec![
                    ("/one".to_string(), ONE.to_string()),
                    ("/two".to_string(), TWO.to_string()),
                    ("/wait-commit".to_string(), "<title>Commit</title><img src='/slow-image-commit'>".to_string()),
                    ("/wait-domcontentloaded".to_string(), "<title>DOM</title><img src='/slow-image-dom'>".to_string()),
                    ("/wait-load".to_string(), "<title>Load</title><img src='/slow-image-load'>".to_string()),
                    ("/stopped".to_string(), "<title>Stopped</title><script>window.__loadEvents=[];addEventListener('DOMContentLoaded',()=>window.__loadEvents.push('domcontentloaded'));addEventListener('load',()=>window.__loadEvents.push('load'));setTimeout(()=>window.stop(),200)</script><img src='/slow-image-stop'>".to_string()),
                ],
            },
            "/one",
        )
        .await,
    )
}

async fn run(tab: &Tab, code: &str) -> RunOutcome {
    tab.start_run(code.to_string())
        .await
        .await
        .unwrap_or_else(|error| panic!("run completes: {error}"))
}

fn value(outcome: RunOutcome) -> Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    outcome
        .value
        .unwrap_or_else(|| panic!("script returns a value"))
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_goto_waits_for_each_supported_lifecycle_and_reads_the_new_title() {
    let Some(tab) = open().await else { return };
    for wait in ["commit", "domcontentloaded", "load"] {
        let result = value(run(&tab, &format!(
            "const result = await page.goto({:?}, {{waitUntil: {:?}}}); return {{status: result.status(), ok: result.ok(), responseUrl: result.url(), method: result.request().method(), contentType: (await result.allHeaders())['content-type'], title: await page.title(), url: page.url()}}",
            format!("{}/two", tab.origin), wait,
        )).await);
        assert_eq!(
            result,
            json!({"status":200, "ok":true, "responseUrl":format!("{}/two", tab.origin),
                   "method":"GET", "contentType":"text/html; charset=utf-8", "title":"Second",
                   "url":format!("{}/two", tab.origin)}),
            "{wait}"
        );
    }
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_goto_waits_for_load_but_not_slow_subresources_at_commit_or_domcontentloaded() {
    let Some(tab) = open().await else { return };
    for wait in ["commit", "domcontentloaded", "load"] {
        let outcome = run(
            &tab,
            &format!(
                "return await page.goto({:?}, {{waitUntil:{wait:?},timeout:10000}})",
                format!("{}/wait-{wait}", tab.origin),
            ),
        )
        .await;
        // A returned Response serializes as Playwright's channel objects do.
        assert_eq!(value(outcome)["_type"], "Response", "{wait}");
        let mut state = tab.evaluate("document.readyState").await;
        // Commit may precede HTML parsing. Wait only for the parser, while the image is held.
        for _ in 0..50 {
            if state != "loading" {
                break;
            }
            time::sleep(Duration::from_millis(20)).await;
            state = tab.evaluate("document.readyState").await;
        }
        assert_eq!(
            state,
            if wait == "load" {
                "complete"
            } else {
                "interactive"
            },
            "{wait}"
        );
    }
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_window_stop_without_load_leaves_goto_waiting() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, &format!(
        "try {{ await page.goto({:?}, {{waitUntil:'load',timeout:1000}}); return 'resolved'; }} catch(e) {{ return e.name; }}",
        format!("{}/stopped",tab.origin),
    )).await;
    let page_events = tab.evaluate("window.__loadEvents").await;
    let title = tab.evaluate("document.title").await;
    tab.close().await;
    assert_eq!(value(outcome), "TimeoutError");
    assert_eq!(title, "Stopped");
    assert_eq!(page_events, json!(["domcontentloaded"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_slow_navigation_times_out_without_committing() {
    let Some(tab) = open().await else { return };
    let url = format!("{}/slow", tab.origin);
    let outcome = run(
        &tab,
        &format!(
            "try {{ await page.goto({url:?}, {{timeout: 500}}); }} catch (e) {{ return {{name:e.name,url:page.url()}}; }}"
        ),
    )
    .await;
    assert_eq!(
        value(outcome),
        json!({"name":"TimeoutError", "url":format!("{}/one",tab.origin)})
    );
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_unresolvable_hosts_reject_with_a_dns_error() {
    let Some(tab) = open().await else { return };
    let result = value(run(&tab, "try { await page.goto('http://missing.neo.invalid/'); } catch(e) {return {name:e.name, dns:e.message.includes('ERR_NAME_NOT_RESOLVED')}}").await);
    assert_eq!(result, json!({"name":"Error","dns":true}));
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_invalid_urls_reject_with_an_invalid_url_error() {
    let Some(tab) = open().await else { return };
    let result = value(
        run(
            &tab,
            "try { await page.goto('not a url'); } catch(e) {return {name:e.name, invalid:e.message.includes('invalid URL')}}",
        )
        .await,
    );
    assert_eq!(result, json!({"name":"Error","invalid":true}));
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_unsupported_and_invalid_wait_until_do_not_navigate() {
    let Some(tab) = open().await else { return };
    let result = value(run(&tab, "const errors=[]; for (const waitUntil of ['networkidle','bogus']) { try { await page.goto('https://example.test/', {waitUntil}); } catch(e) { errors.push(e.name); } } return {errors,url:page.url()}").await);
    assert_eq!(
        result,
        json!({"errors":["Error", "Error"], "url":format!("{}/one", tab.origin)})
    );
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_hash_navigation_uses_an_already_loaded_documents_lifecycle() {
    let Some(tab) = open().await else { return };
    tab.evaluate("new Promise(resolve => document.readyState === 'complete' ? resolve(true) : addEventListener('load',()=>resolve(true)))").await;
    let result = value(run(&tab, &format!("const values=[]; for (const waitUntil of ['load','domcontentloaded','commit']) {{values.push(await page.goto({:?} + '#' + waitUntil, {{waitUntil,timeout:1500}}));}} return {{values,title:await page.title(),url:page.url()}}", format!("{}/one",tab.origin))).await);
    assert_eq!(
        result,
        json!({"values":[null,null,null],"title":"First","url":format!("{}/one#commit",tab.origin)})
    );
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_title_after_a_navigating_click_reads_the_new_world() {
    let Some(tab) = open().await else { return };
    let result = value(run(&tab, "const first=await page.title(); await page.locator('#next').click(); return {first,title:await page.title()} ").await);
    assert_eq!(result, json!({"first":"First","title":"Second"}));
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_title_replaces_a_lone_surrogate_without_hanging() {
    let Some(tab) = open().await else { return };
    tab.evaluate(r"document.title = 'a\uD800b'; true").await;
    let outcome = tab
        .start_run_within(
            "return await page.title()".to_string(),
            Duration::from_secs(3),
        )
        .await
        .await
        .expect("the bounded title run finishes");
    tab.close().await;
    assert_eq!(value(outcome), json!("a\u{fffd}b"));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_clicks_after_goto_keep_their_input_turns() {
    let Some(tab) = open().await else { return };
    let result = value(run(&tab, &format!("await page.goto({:?}); await page.locator('#go').click(); return await page.title()",format!("{}/two",tab.origin))).await);
    assert_eq!(result, "Clicked");
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_two_runs_on_the_same_tab_read_the_current_document() {
    let Some(tab) = open().await else { return };
    tab.navigate("/one").await;
    assert_eq!(
        value(
            run(
                &tab,
                &format!(
                    "await page.goto({:?}); return await page.title()",
                    format!("{}/two", tab.origin)
                )
            )
            .await
        ),
        "Second"
    );
    assert_eq!(value(run(&tab, &format!("const before=await page.title(); await page.goto({:?}); return {{before,after:await page.title()}}",format!("{}/one",tab.origin))).await), json!({"before":"Second","after":"First"}));
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_history_navigation_restores_a_readable_document() {
    let Some(tab) = open().await else { return };
    let original = history_document(&tab).await;
    let result = value(run(&tab, &format!(
        "await page.goto({:?}); const before=await page.title(); await page.locator('#back').click(); const after=await page.title(); const afterBackMarker=await page.locator('html').getAttribute('data-history-document'); await page.goto({:?}, {{timeout:1500}}); return {{titles:{{before,after,afterHash:await page.title()}},afterBackMarker,afterHashMarker:await page.locator('html').getAttribute('data-history-document')}}",
        format!("{}/two",tab.origin), format!("{}/one#restored",tab.origin),
    )).await);
    let current = history_document(&tab).await;
    tab.close().await;
    assert_eq!(
        result["titles"],
        json!({"before":"Second","after":"First","afterHash":"First"})
    );
    assert!(original["marker"].is_string(), "{original}");
    assert!(current["marker"].is_string(), "{current}");
    assert!(current["restored"].is_boolean(), "{current}");
    assert_eq!(
        result["afterBackMarker"], current["marker"],
        "Neo must read the driver's current document after history navigation"
    );
    assert_eq!(
        result["afterHashMarker"], current["marker"],
        "the fragment change must keep the current document readable"
    );
    // Chromium may flush an eligible cache entry. A cache hit retains this document;
    // a history reload creates a new one, which Neo must read just as faithfully.
    if current["restored"] == true {
        assert_eq!(current["marker"], original["marker"]);
    } else {
        assert_eq!(current["navigationType"], "back_forward");
        assert_ne!(current["marker"], original["marker"]);
    }
}

/// Reads the actual document independently of Neo after its fixture script has run.
async fn history_document(tab: &Tab) -> Value {
    let reply = tab.cdp.send("Runtime.evaluate", json!({
        "expression": r"(async () => {
            if (document.readyState === 'loading')
                await new Promise(resolve => addEventListener('DOMContentLoaded', resolve, {once:true}));
            return {marker:document.documentElement.dataset.historyDocument,
                restored:window.__restored,
                navigationType:performance.getEntriesByType('navigation')[0].type};
        })()",
        "awaitPromise":true,
        "returnByValue":true,
    }), Some(&tab.driver)).await.unwrap_or_else(|error| panic!("read history fixture document: {error}"));
    assert!(reply.get("exceptionDetails").is_none(), "{reply}");
    reply["result"]["value"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_title_returns_loading_while_a_click_navigation_is_pending() {
    let Some(tab) = open().await else { return };
    let result = value(
        run(
            &tab,
            "await page.locator('#slow').click({noWaitAfter:true}); return await page.title()",
        )
        .await,
    );
    assert_eq!(result, format!("Loading {}/slow", tab.origin));
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_later_runs_can_wait_on_an_already_loaded_document() {
    let Some(tab) = open().await else { return };
    // The fixture server has no subresources: after title is readable, wait for complete.
    for _ in 0..100 {
        if tab.evaluate("document.readyState").await == "complete" {
            break;
        }
        time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(tab.evaluate("document.readyState").await, "complete");
    for fragment in ["first-run", "second-run"] {
        let url = format!("{}/one#{fragment}", tab.origin);
        let outcome = run(&tab, &format!("await page.goto({url:?}, {{waitUntil:'load',timeout:1500}}); return {{url:page.url(),title:await page.title()}}" )).await;
        assert_eq!(value(outcome), json!({"url":url,"title":"First"}));
    }
    tab.close().await;
}
