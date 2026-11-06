//! Navigation, titles and document freshness on an isolated real BrowserOS neo profile.

#[path = "support/browser.rs"]
mod browser;

use std::collections::BTreeSet;
use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::time;

const ONE: &str = r#"<!doctype html><title>First</title><h1>First page</h1>
<a id="next" href="/two">Next</a><a id="slow" href="/slow">Slow</a>"#;
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
            "const result = await page.goto({:?}, {{waitUntil: {:?}}}); return {{isNull: result === null, title: await page.title(), url: page.url()}}",
            format!("{}/two", tab.origin), wait,
        )).await);
        assert_eq!(
            result,
            json!({"isNull":true, "title":"Second", "url":format!("{}/two", tab.origin)})
        );
    }
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_slow_navigation_has_playwrights_timeout_and_single_log() {
    let Some(tab) = open().await else { return };
    let url = format!("{}/slow", tab.origin);
    let outcome = run(&tab, &format!("try {{ await page.goto({url:?}, {{timeout: 500}}); }} catch (e) {{ return {{name:e.name, message:e.message}}; }}")).await;
    assert_eq!(
        value(outcome),
        json!({"name":"TimeoutError", "message":format!(
            "page.goto: Timeout 500ms exceeded.\nCall log:\n  - navigating to \"{url}\", waiting until \"load\"\n"
        )})
    );
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_unresolvable_hosts_keep_chromiums_error() {
    let Some(tab) = open().await else { return };
    let result = value(run(&tab, "try { await page.goto('http://missing.neo.invalid/'); } catch(e) {return {name:e.name, message:e.message}} ").await);
    assert_eq!(result["name"], "Error");
    assert!(
        result["message"]
            .as_str()
            .expect("message")
            .starts_with("page.goto: net::ERR_NAME_NOT_RESOLVED at http://missing.neo.invalid/"),
        "{result}"
    );
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_invalid_urls_keep_playwrights_protocol_error() {
    let Some(tab) = open().await else { return };
    let result = value(
        run(
            &tab,
            "try { await page.goto('not a url'); } catch(e) {return e.message}",
        )
        .await,
    );
    assert!(
        result.as_str().expect("message").starts_with(
            "page.goto: Protocol error (Page.navigate): Cannot navigate to invalid URL"
        ),
        "{result}"
    );
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_unsupported_and_invalid_wait_until_do_not_navigate() {
    let Some(tab) = open().await else { return };
    let result = value(run(&tab, "const errors=[]; for (const waitUntil of ['networkidle','bogus']) { try { await page.goto('https://example.test/', {waitUntil}); } catch(e) { errors.push(e.message); } } return {errors,url:page.url()}").await);
    assert_eq!(
        result,
        json!({"errors":["page.goto: Neo does not support waitUntil: \"networkidle\" yet", "waitUntil: expected one of (load|domcontentloaded|networkidle|commit)"], "url":format!("{}/one", tab.origin)})
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
    let mut events = tab.cdp.events();
    let result = value(run(&tab, &format!(
        "await page.goto({:?}); await page.goto({:?}); const before=await page.title(); await page.locator('#back').click(); return {{before,after:await page.title()}}",
        format!("{}/one",tab.origin), format!("{}/two",tab.origin),
    )).await);
    assert_eq!(result, json!({"before":"Second","after":"First"}));
    let mut restored = false;
    while let Ok(event) = events.try_recv() {
        restored |= event.method == "Page.frameNavigated"
            && event.params["type"] == "BackForwardCacheRestore";
    }
    assert!(
        restored,
        "the history test must exercise an actual bfcache restore"
    );
    tab.close().await;
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
async fn tests_that_browseros_replays_lifecycle_for_a_loaded_document() {
    let Some(tab) = open().await else { return };
    // The fixture server has no subresources: after title is readable, wait for complete.
    for _ in 0..100 {
        if tab.evaluate("document.readyState").await == "complete" {
            break;
        }
        time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(tab.evaluate("document.readyState").await, "complete");
    let mut events = tab.cdp.events();
    tab.cdp
        .send("Page.enable", json!({}), Some(&tab.driver))
        .await
        .expect("enable Page");
    tab.cdp
        .send(
            "Page.setLifecycleEventsEnabled",
            json!({"enabled":true}),
            Some(&tab.driver),
        )
        .await
        .expect("enable lifecycle");
    let observed = time::timeout(Duration::from_secs(3), async {
        let mut names = BTreeSet::new();
        loop {
            let event = events.recv().await.expect("CDP event");
            if event.session_id.as_ref() == Some(&tab.driver)
                && event.method == "Page.lifecycleEvent"
                && event.params["frameId"] == tab.target_id
            {
                names.insert(
                    event.params["name"]
                        .as_str()
                        .expect("event name")
                        .to_string(),
                );
                if names.contains("DOMContentLoaded") && names.contains("load") {
                    return names;
                }
            }
        }
    })
    .await
    .expect("already-loaded documents replay both lifecycle events");
    let _ = observed;
    tab.close().await;
}
