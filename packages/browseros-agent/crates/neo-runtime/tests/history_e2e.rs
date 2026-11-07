//! `page.reload`, `goBack`, `goForward` and `waitForNavigation` on an isolated real BrowserOS
//! neo profile, with the back-forward cache on, as a user's browser has it (Playwright launches
//! its own browser without it; `neo-quickjs-injected/tests/parity/history.mjs` compares the two
//! there).

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "the host suite alone uses the shared harness CDP port accessor"
)]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{CallResult, RunOutcome, RunStatus};
use serde_json::{Value, json};

const ONE: &str = r#"<!doctype html><title>One</title>
<a id="next" href="/two">Two</a>
<a id="unsafe" href="http://127.0.0.1:1/unsafe">Unsafe port</a>
<button id="chain" onclick="history.pushState({}, '', '/not-yet'); setTimeout(() => { location.href = '/two?q=1' }, 50)">Chain</button>
<script>
window.__restored = false;
addEventListener('pageshow', (event) => { window.__restored = event.persisted });
</script>"#;
const TWO: &str = r"<!doctype html><title>Two</title>
<script>addEventListener('pageshow', (event) => { window.__restored = event.persisted })</script>";

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

/// A response as the scripts below describe it, or `null`.
const DESCRIBE: &str = "const describe = (r) => r === null ? null : \
    { status: r.status(), url: r.url(), method: r.request().method() };";

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_reload_answers_the_new_documents_response() {
    let Some(tab) = open().await else { return };
    tab.evaluate("window.__marker = 'before'").await;
    let result = value(
        run(
            &tab,
            &format!("{DESCRIBE} return describe(await page.reload({{ timeout: 5000 }}))"),
        )
        .await,
    );
    let marker = tab.evaluate("typeof window.__marker").await;
    let origin = tab.origin.clone();
    tab.close().await;
    assert_eq!(
        result,
        json!({"status": 200, "url": format!("{origin}/one"), "method": "GET"})
    );
    assert_eq!(marker, "undefined", "the reload made a new document");
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_history_steps_settle_on_cached_and_reloaded_documents_alike() {
    let Some(tab) = open().await else { return };
    // Three round trips: whether Chromium restores a document from its back-forward cache or
    // loads it again, each step must settle well inside its timeout, with `null` for a restored
    // document (no request) and its response otherwise, and leave the document readable.
    let result = value(
        run(
            &tab,
            &format!(
                "{DESCRIBE} const steps = [];
                for (let i = 0; i < 3; i++) {{
                  await page.goto({two:?});
                  const back = describe(await page.goBack({{ timeout: 5000 }}));
                  const backTitle = await page.title();
                  const forward = describe(await page.goForward({{ timeout: 5000 }}));
                  steps.push({{ back, backTitle, forward, forwardTitle: await page.title() }});
                }}
                await page.waitForLoadState('load', {{ timeout: 1000 }});
                return {{ steps, url: page.url() }}",
                two = format!("{}/two", tab.origin)
            ),
        )
        .await,
    );
    let restored = tab.evaluate("window.__restored").await;
    let origin = tab.origin.clone();
    tab.close().await;
    for step in result["steps"].as_array().expect("steps") {
        assert_eq!(
            (&step["backTitle"], &step["forwardTitle"]),
            (&json!("One"), &json!("Two"))
        );
        for (direction, path) in [("back", "/one"), ("forward", "/two")] {
            let answer = &step[direction];
            assert!(
                answer.is_null()
                    || *answer
                        == json!({"status": 200, "url": format!("{origin}{path}"), "method": "GET"}),
                "{direction}: {answer}"
            );
        }
    }
    assert_eq!(result["url"], format!("{origin}/two"));
    assert!(restored.is_boolean(), "{restored}");
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_restored_document_answers_null_and_counts_as_loaded() {
    let Some(tab) = open().await else { return };
    // Leave and come back until Chromium restores /one from its cache (it may decline).
    for _ in 0..5 {
        tab.navigate("/two").await;
        let back = value(
            run(
                &tab,
                &format!(
                    "{DESCRIBE} const back = describe(await page.goBack({{ timeout: 5000 }}));
                    await page.waitForLoadState('load', {{ timeout: 1000 }});
                    return {{ back, title: await page.title() }}"
                ),
            )
            .await,
        );
        if tab.evaluate("window.__restored").await == true {
            assert_eq!(back, json!({"back": null, "title": "One"}));
            tab.close().await;
            return;
        }
        assert_eq!(back["back"]["status"], 200, "{back}");
    }
    tab.close().await;
    panic!("Chromium restored no document from its back-forward cache in five tries");
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_no_history_entry_answers_null_where_the_page_is() {
    let Some(tab) = open().await else { return };
    let result = value(
        run(
            &tab,
            "return { back: await page.goBack(), forward: await page.goForward(), url: page.url() }",
        )
        .await,
    );
    let origin = tab.origin.clone();
    tab.close().await;
    assert_eq!(
        result,
        json!({"back": null, "forward": null, "url": format!("{origin}/one")})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_wait_for_navigation_answers_a_click_and_rejects_a_failed_one() {
    let Some(tab) = open().await else { return };
    let result = value(
        run(
            &tab,
            &format!(
                "{DESCRIBE} const failed = await Promise.all([
                  page.waitForNavigation({{ timeout: 5000 }}).then(() => null, (e) => ({{ name: e.name, network: /net::ERR_/.test(e.message) }})),
                  page.click('#unsafe'),
                ]).then(([failed]) => failed);
                await page.goto({one:?});
                const [clicked] = await Promise.all([page.waitForNavigation(), page.click('#next')]);
                return {{ failed, clicked: describe(clicked) }}",
                one = format!("{}/one", tab.origin)
            ),
        )
        .await,
    );
    let origin = tab.origin.clone();
    tab.close().await;
    assert_eq!(
        result,
        json!({
            "failed": {"name": "Error", "network": true},
            "clicked": {"status": 200, "url": format!("{origin}/two"), "method": "GET"},
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_slow_predicate_still_sees_each_navigation_in_order() {
    let Some(tab) = open().await else { return };
    // The page navigates twice while the predicate's first answer is still pending: the
    // cursor keeps the second navigation queued in Rust until the realm asks for it.
    let outcome = run(
        &tab,
        &format!(
            "{DESCRIBE} const seen = [];
            const [response] = await Promise.all([
              page.waitForNavigation({{ url: async (url) => {{
                seen.push(url.pathname + url.search);
                await new Promise((resolve) => setTimeout(resolve, 300));
                return url.searchParams.get('q') === '1';
              }} }}),
              page.click('#chain'),
            ]);
            return {{ seen, response: describe(response) }}"
        ),
    )
    .await;
    let calls: Vec<String> = outcome
        .calls
        .iter()
        .map(|call| call.method.clone())
        .collect();
    let replied = outcome
        .calls
        .iter()
        .all(|call| call.result == CallResult::Ok);
    let result = value(outcome);
    let origin = tab.origin.clone();
    tab.close().await;
    assert_eq!(
        result,
        json!({
            "seen": ["/not-yet", "/two?q=1"],
            "response": {"status": 200, "url": format!("{origin}/two?q=1"), "method": "GET"},
        })
    );
    assert_eq!(
        calls
            .iter()
            .filter(|method| method.starts_with("neo:"))
            .collect::<Vec<_>>(),
        [
            "neo:nextNavigation",
            "neo:nextNavigation",
            "neo:nextNavigation",
            "neo:finishNavigation"
        ],
        "one call per navigation the predicates tested, the one still waiting when the second \
         was accepted (its finish ended it), then the finish"
    );
    assert!(
        replied,
        "every call of the wait replied before the run ended, the ended next too"
    );
}
