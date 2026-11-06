//! `textContent` and `ariaSnapshot` end to end on a real BrowserOS neo, through neo-sdk's
//! `locator.textContent`, `page.textContent` and `page.ariaSnapshot`, and `aria-ref=eN` refs from
//! an `ai` snapshot driving the existing `locator.click` and `locator.textContent`: in the run
//! that took the snapshot, in a later run on the same document, and not after the document
//! navigated away. `crates/neo-sdk/tests/parity/textContent.test.mjs` and
//! `ariaSnapshot.test.mjs` run the same reads through live Playwright beside Neo.
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary (`support/browser.rs`); otherwise
//! each test passes as a skip.

#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{CallResult, RunOutcome, RunStatus};
use serde_json::{Value, json};

/// Every read below runs against this page. `#go` counts its clicks into `#status`.
const READS: &str = r#"<!doctype html><html><head><title>Reads</title></head><body>
<h1>Reads</h1>
<ul><li>Apples</li><li>Pears</li></ul>
<p id="greet">Hello <b>world</b></p>
<p id="empty"></p>
<button id="go" onclick="document.getElementById('status').textContent = 'Clicked ' + (++window.__clicks)">Go</button>
<p id="status">Waiting</p>
<a href="/other">Other page</a>
<script>window.__clicks = 0</script>
</body></html>"#;

/// The page `READS` links to, with a button of the same name and role.
const OTHER: &str = r#"<!doctype html><html><head><title>Other</title></head><body>
<h1>Other</h1>
<button id="go" onclick="document.getElementById('status').textContent = 'Other clicked'">Go</button>
<p id="status">Untouched</p>
</body></html>"#;

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let fixture = Fixture {
        pages: vec![
            ("/reads".to_string(), READS.to_string()),
            ("/other".to_string(), OTHER.to_string()),
        ],
        ..Fixture::default()
    };
    Some(Tab::open(binary, fixture, "/reads").await)
}

async fn run(tab: &Tab, code: &str) -> RunOutcome {
    tab.start_run(code.to_string())
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"))
}

/// The script's return value, after checking the run itself went well.
fn value(outcome: &RunOutcome) -> Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    outcome
        .value
        .clone()
        .unwrap_or_else(|| panic!("the script returns a value: {outcome:?}"))
}

/// What a script sees of an error it caught: `{name, message}`.
const CAUGHT: &str = "(e) => ({ name: e.name, message: e.message })";

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_text_content_reads_the_first_match_with_playwrights_strictness() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        &format!(
            "const caught = {CAUGHT};
             return {{
               greet: await page.locator('#greet').textContent(),
               empty: await page.locator('#empty').textContent(),
               firstOfTwo: await page.textContent('li'),
               viaFrame: await page.mainFrame().textContent('li >> nth=1'),
               byRole: await page.getByRole('heading').textContent(),
               strict: await page.locator('li').textContent().catch(caught),
             }};"
        ),
    )
    .await;
    let result = value(&outcome);
    assert_eq!(result["greet"], "Hello world");
    assert_eq!(
        result["empty"], "",
        "an empty element reads as '', not null"
    );
    assert_eq!(
        result["firstOfTwo"], "Apples",
        "page.textContent is not strict"
    );
    assert_eq!(result["viaFrame"], "Pears");
    assert_eq!(result["byRole"], "Reads");
    assert_eq!(
        result["strict"],
        json!({
            "name": "Error",
            "message": "locator.textContent: Error: strict mode violation: locator('li') resolved to 2 elements:\n    1) <li>Apples</li> aka getByText('Apples')\n    2) <li>Pears</li> aka getByText('Pears')\n\nCall log:\n  - waiting for locator('li')\n",
        })
    );

    let greet = &outcome.calls[0];
    assert_eq!(
        (greet.method.as_str(), greet.title.as_deref(), &greet.result),
        ("textContent", Some("Get text content"), &CallResult::Ok)
    );
    assert_eq!(
        greet.log,
        [
            "  - waiting for locator('#greet')",
            "    - locator resolved to <p id=\"greet\">\u{2026}</p>",
        ],
        "Playwright's two lines, the preview folded for a child element"
    );
    assert_eq!(greet.delivery, None, "a read writes no input");
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_text_content_waits_for_an_element_appended_later() {
    let Some(tab) = open().await else { return };
    tab.evaluate(
        "setTimeout(() => { const p = document.createElement('p'); p.id = 'late'; \
         p.textContent = 'Arrived'; document.body.appendChild(p); }, 600)",
    )
    .await;
    let outcome = run(
        &tab,
        "return await page.locator('#late').textContent({ timeout: 10000 })",
    )
    .await;
    assert_eq!(value(&outcome), "Arrived");
    let call = &outcome.calls[0];
    assert!(
        call.duration >= Duration::from_millis(300),
        "the read polled until the element existed: {:?}",
        call.duration
    );
    assert_eq!(
        call.log,
        [
            "  - waiting for locator('#late')",
            "    - locator resolved to <p id=\"late\">Arrived</p>",
        ]
    );
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_text_content_times_out_and_fails_fast_on_a_bad_selector() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        &format!(
            "const caught = {CAUGHT};
             const started = Date.now();
             const missing = await page.locator('#missing').textContent({{ timeout: 500 }}).catch(caught);
             const waited = Date.now() - started;
             const invalid = await page.locator('li >>').textContent({{ timeout: 5000 }}).catch(caught);
             return {{ missing, waited, invalid, quick: Date.now() - started - waited < 2000 }};"
        ),
    )
    .await;
    let result = value(&outcome);
    assert_eq!(
        result["missing"],
        json!({
            "name": "TimeoutError",
            "message": "locator.textContent: Timeout 500ms exceeded.\nCall log:\n  - waiting for locator('#missing')\n",
        })
    );
    let waited = result["waited"].as_f64().unwrap_or_default();
    assert!((450.0..3000.0).contains(&waited), "waited {waited} ms");
    assert_eq!(result["invalid"]["name"], "Error");
    assert_eq!(
        result["invalid"]["message"],
        "locator.textContent: Unexpected token \"\" while parsing css selector \"\". Did you mean \
         to CSS.escape it?\nCall log:\n  - waiting for li >>\n",
        "the page's parse error, and the raw selector as the locator"
    );
    assert_eq!(result["quick"], true, "an invalid selector fails at once");
    tab.close().await;
}

/// The default-mode snapshot of `READS`, as Playwright renders it.
const READS_SNAPSHOT: &str = r#"- heading "Reads" [level=1]
- list:
  - listitem: Apples
  - listitem: Pears
- paragraph: Hello world
- paragraph
- button "Go"
- paragraph: Waiting
- link "Other page":
  - /url: /other"#;

/// The `ai` snapshot of a fresh `READS`: the focused body is an `[active]` generic, every
/// visible element gets a ref in document order (the empty paragraph has no box, so none), and
/// only the link has a pointer cursor.
const READS_AI_SNAPSHOT: &str = r#"- generic [active] [ref=e1]:
  - heading "Reads" [level=1] [ref=e2]
  - list [ref=e3]:
    - listitem [ref=e4]: Apples
    - listitem [ref=e5]: Pears
  - paragraph [ref=e6]: Hello world
  - paragraph
  - button "Go" [ref=e7]
  - paragraph [ref=e8]: Waiting
  - link "Other page" [ref=e9] [cursor=pointer]:
    - /url: /other"#;

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_snapshots_render_the_document_and_ai_refs_drive_click_and_reads() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        "const plain = await page.ariaSnapshot();
         const ai = await page.ariaSnapshot({ mode: 'ai' });
         const ref = (label) =>
           ai.split('\\n').find((line) => line.includes(label))?.match(/\\[ref=(e\\d+)\\]/)?.[1];
         const go = ref('button \"Go\"');
         const status = ref('Waiting');
         await page.locator(`aria-ref=${go}`).click();
         const statusText = await page.locator('#status').textContent();
         const viaRef = await page.locator(`aria-ref=${go}`).textContent();
         return { plain, ai, go, status, statusText, viaRef, type: typeof ai };",
    )
    .await;
    let result = value(&outcome);
    assert_eq!(result["type"], "string");
    assert_eq!(result["plain"], READS_SNAPSHOT);
    assert_eq!(result["ai"], READS_AI_SNAPSHOT);
    assert_eq!(
        (&result["go"], &result["status"]),
        (&json!("e7"), &json!("e8"))
    );
    assert_eq!(
        result["statusText"], "Clicked 1",
        "the ref clicked the button"
    );
    assert_eq!(result["viaRef"], "Go", "and reads it");

    let snapshot = &outcome.calls[0];
    assert_eq!(
        (
            snapshot.method.as_str(),
            snapshot.title.as_deref(),
            &snapshot.result
        ),
        ("ariaSnapshot", Some("Aria snapshot"), &CallResult::Ok)
    );
    assert!(
        snapshot.log.is_empty(),
        "a snapshot logs nothing: {snapshot:?}"
    );
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_refs_resolve_in_a_later_run_on_the_same_document() {
    let Some(tab) = open().await else { return };
    let first = run(
        &tab,
        "const ai = await page.ariaSnapshot({ mode: 'ai' });
         return /button \"Go\" \\[ref=(e\\d+)\\]/.exec(ai)[1];",
    )
    .await;
    let go = value(&first);
    let go = go.as_str().unwrap_or_default();
    let second = run(
        &tab,
        &format!(
            "await page.locator('aria-ref={go}').click();
             await page.locator('aria-ref={go}').click();
             return await page.locator('#status').textContent();"
        ),
    )
    .await;
    assert_eq!(value(&second), "Clicked 2");
    assert_eq!(tab.evaluate("window.__clicks").await, 2);
    tab.close().await;
}

/// A ref names an element of the document whose snapshot minted it. After a cross-document
/// navigation the new document's injected script has no snapshot, so the ref matches nothing
/// and the call times out without touching the new page. A new snapshot there mints refs from
/// `e1` again (Neo keeps `frameSeq` 0), which is why Playwright renumbers the frame instead.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_refs_do_not_resolve_after_the_document_navigated_away() {
    let Some(tab) = open().await else { return };
    let first = run(
        &tab,
        "const ai = await page.ariaSnapshot({ mode: 'ai' });
         return /button \"Go\" \\[ref=(e\\d+)\\]/.exec(ai)[1];",
    )
    .await;
    let go = value(&first);
    let go = go.as_str().unwrap_or_default();
    tab.navigate("/other").await;
    let outcome = run(
        &tab,
        &format!(
            "const caught = {CAUGHT};
             return {{
               click: await page.locator('aria-ref={go}').click({{ timeout: 800 }}).catch(caught),
               read: await page.locator('aria-ref={go}').textContent({{ timeout: 800 }}).catch(caught),
               status: await page.locator('#status').textContent(),
             }};"
        ),
    )
    .await;
    let result = value(&outcome);
    assert_eq!(result["click"]["name"], "TimeoutError", "{result}");
    assert_eq!(result["read"]["name"], "TimeoutError", "{result}");
    assert_eq!(
        result["status"], "Untouched",
        "the stale ref clicked nothing on the new page"
    );
    tab.close().await;
}

/// A document without a body is polled for one at 0, 1000 and 3000 ms (Playwright's
/// `[1000, 2000, 4000, 8000]`), and a snapshot that never finds one times out without a log.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_snapshot_waits_for_the_body_and_times_out_without_one() {
    let Some(tab) = open().await else { return };
    tab.evaluate(
        "window.__body = document.body; document.body.remove(); \
         setTimeout(() => document.documentElement.appendChild(window.__body), 1500)",
    )
    .await;
    let outcome = run(
        &tab,
        "const started = Date.now();
         const snapshot = await page.ariaSnapshot({ timeout: 10000 });
         return { snapshot, waited: Date.now() - started };",
    )
    .await;
    let result = value(&outcome);
    assert_eq!(result["snapshot"], READS_SNAPSHOT);
    let waited = result["waited"].as_f64().unwrap_or_default();
    assert!(
        (2500.0..6000.0).contains(&waited),
        "found at the 3000 ms poll, not before: {waited} ms"
    );

    tab.evaluate("document.body.remove()").await;
    let outcome = run(
        &tab,
        &format!("return await page.ariaSnapshot({{ timeout: 1500 }}).catch({CAUGHT});"),
    )
    .await;
    assert_eq!(
        value(&outcome),
        json!({
            "name": "TimeoutError",
            "message": "page.ariaSnapshot: Timeout 1500ms exceeded.",
        })
    );
    tab.close().await;
}

/// `locator.ariaSnapshot` sends a selector, which Neo refuses before any work; neo-sdk exposes
/// no such method, so the script sends Playwright's message itself.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_snapshot_with_a_selector_is_refused() {
    let Some(tab) = open().await else { return };
    let frame = format!("frame@{}", tab.target_id);
    let outcome = run(
        &tab,
        &format!(
            "const message = {{ id: 1, guid: {frame:?}, method: 'ariaSnapshot',
                params: {{ selector: 'ul', mode: 'ai' }}, metadata: {{ timeout: 5000 }} }};
             return JSON.parse(await __neo_runtime.call(JSON.stringify(message)));"
        ),
    )
    .await;
    assert_eq!(
        value(&outcome),
        json!({
            "id": 1,
            "error": { "error": {
                "message": "Neo does not serve locator.ariaSnapshot yet",
                "name": "Error",
            } },
        })
    );
    tab.close().await;
}
