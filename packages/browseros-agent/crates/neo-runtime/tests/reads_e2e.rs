//! `textContent`, `innerText`, `getAttribute` and `ariaSnapshot` end to end on a real BrowserOS
//! neo, through neo-sdk's `locator.textContent`, `page.textContent`, the locator, page and frame
//! forms of `innerText` and `getAttribute` (on `crates/neo-sdk/tests/pages/element-text.html`)
//! and page/locator `ariaSnapshot`, and `aria-ref=eN` refs from an `ai` snapshot driving the existing
//! `locator.click` and `locator.textContent`: in the run that took the snapshot, in a later run
//! on the same document, and not after the document navigated away.
//! `crates/neo-sdk/tests/parity/textContent.test.mjs`, `innerText.test.mjs`,
//! `getAttribute.test.mjs` and `ariaSnapshot.test.mjs` run the same reads through live
//! Playwright beside Neo.
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary (`support/browser.rs`); otherwise
//! each test passes as a skip.

#[path = "support/browser.rs"]
mod browser;

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

/// What a script sees of an error it caught, as a kind: its name, and whether it is a strict
/// mode violation. The wording is never compared.
const CAUGHT: &str = "(e) => ({ name: e.name, strict: /strict mode violation/.test(e.message) })";

/// An error kind as `CAUGHT` returns it.
fn kind(name: &str, strict: bool) -> Value {
    json!({ "name": name, "strict": strict })
}

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
        kind("Error", true),
        "locator reads are strict"
    );

    let greet = &outcome.calls[0];
    assert_eq!(
        (greet.method.as_str(), &greet.result),
        ("textContent", &CallResult::Ok)
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
    assert_eq!(
        value(&outcome),
        "Arrived",
        "the read waited for the element"
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
             const missing = await page.locator('#missing').textContent({{ timeout: 500 }}).catch(caught);
             const started = Date.now();
             const invalid = await page.locator('li >>').textContent({{ timeout: 5000 }}).catch(caught);
             return {{ missing, invalid, quick: Date.now() - started < 2000 }};"
        ),
    )
    .await;
    let result = value(&outcome);
    assert_eq!(result["missing"], kind("TimeoutError", false));
    assert_eq!(result["invalid"], kind("Error", false));
    assert_eq!(
        result["quick"], true,
        "an invalid selector fails at once, not at its timeout"
    );
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
        (snapshot.method.as_str(), &snapshot.result),
        ("ariaSnapshot", &CallResult::Ok)
    );
    assert_eq!(snapshot.delivery, None, "a snapshot writes no input");
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

/// A document without a body is polled until it has one, and a snapshot that never finds one
/// times out.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_snapshot_waits_for_the_body_and_times_out_without_one() {
    let Some(tab) = open().await else { return };
    tab.evaluate(
        "window.__body = document.body; document.body.remove(); \
         setTimeout(() => document.documentElement.appendChild(window.__body), 1500)",
    )
    .await;
    let outcome = run(&tab, "return await page.ariaSnapshot({ timeout: 10000 })").await;
    assert_eq!(
        value(&outcome),
        READS_SNAPSHOT,
        "found once the body is back"
    );

    tab.evaluate("document.body.remove()").await;
    let outcome = run(
        &tab,
        &format!("return await page.ariaSnapshot({{ timeout: 1500 }}).catch({CAUGHT});"),
    )
    .await;
    assert_eq!(value(&outcome), kind("TimeoutError", false));
    tab.close().await;
}

/// Neo: a scoped snapshot crosses the real SDK, runtime and page glue, retaining usable refs.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_locator_snapshot_reads_only_its_subtree_with_usable_refs() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        r"const list = page.getByRole('list');
          const plain = await list.ariaSnapshot();
          const ai = await list.ariaSnapshot({ mode: 'ai' });
          const ref = ai.split('\n').find(line => line.includes('Apples')).match(/\[ref=(e\d+)\]/)[1];
          return { plain, refText: await page.locator('aria-ref=' + ref).textContent() };",
    )
    .await;
    assert_eq!(
        value(&outcome),
        json!({
            "plain": "- list:\n  - listitem: Apples\n  - listitem: Pears",
            "refText": "Apples",
        })
    );
    tab.close().await;
}

/// A lone surrogate (a truncated emoji) reads as U+FFFD. Rust strings are UTF-8, and
/// browseros-cdp drops a CDP reply that escapes one, so without the glue's replacement both reads
/// would wait out their timeout. Playwright returns the lone surrogate unchanged.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_lone_surrogate_reads_as_a_replacement_character() {
    let Some(tab) = open().await else { return };
    tab.evaluate(
        "document.body.innerHTML = ''; const b = document.createElement('button'); \
         b.textContent = 'Go \\ud83d'; const p = document.createElement('p'); \
         p.textContent = 'cut \\ude00 here'; document.body.append(b, p)",
    )
    .await;
    let outcome = run(
        &tab,
        "return {
           text: await page.locator('body > p').textContent({ timeout: 5000 }),
           snapshot: await page.ariaSnapshot({ timeout: 5000 }),
         };",
    )
    .await;
    assert_eq!(
        value(&outcome),
        json!({
            "text": "cut \u{fffd} here",
            "snapshot": "- button \"Go \u{fffd}\"\n- paragraph: cut \u{fffd} here",
        })
    );
    tab.close().await;
}

/// A lone surrogate inside a thrown page error or a click's element preview reaches Rust
/// well-formed too (`neo-runtime-injected/src/well-formed.ts`), so a strict mode violation that
/// lists the element fails at once and a click on it goes through.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_lone_surrogate_in_an_error_or_a_click_preview_does_not_hang() {
    let Some(tab) = open().await else { return };
    tab.evaluate(
        "document.body.innerHTML = '<ul><li>Apples</li><li id=\"b\">x</li></ul>'; \
         window.__clicks = 0; const b = document.getElementById('b'); \
         b.textContent = 'Pe\\ud800ars'; b.onclick = () => window.__clicks++",
    )
    .await;
    let outcome = run(
        &tab,
        &format!(
            "const caught = (e) => ({{ ...({CAUGHT})(e), wellFormed: e.message.includes('Pe\\ufffdars') }});
             const started = Date.now();
             const strict = await page.locator('li').textContent({{ timeout: 3000 }}).catch(caught);
             const strictMs = Date.now() - started;
             await page.locator('#b').click({{ timeout: 3000 }});
             return {{ strict, strictMs, text: await page.locator('#b').textContent() }};"
        ),
    )
    .await;
    let result = value(&outcome);
    assert_eq!(
        result["strict"],
        json!({ "name": "Error", "strict": true, "wellFormed": true }),
        "a strict mode violation whose message names the element, well-formed"
    );
    assert!(
        result["strictMs"].as_f64().is_some_and(|ms| ms < 2000.0),
        "at once, not at the timeout: {result}"
    );
    assert_eq!(result["text"], "Pe\u{fffd}ars");
    assert_eq!(
        tab.evaluate("window.__clicks").await,
        1,
        "the click reached #b"
    );
    tab.close().await;
}

/// `crates/neo-sdk/tests/pages/element-text.html`, which `innerText.test.mjs` and
/// `getAttribute.test.mjs` read through live Playwright too.
const ELEMENT_TEXT: &str = include_str!("../../neo-sdk/tests/pages/element-text.html");

/// A tab on `ELEMENT_TEXT`.
async fn open_element_text() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let fixture = Fixture {
        pages: vec![("/element-text".to_string(), ELEMENT_TEXT.to_string())],
        ..Fixture::default()
    };
    Some(Tab::open(binary, fixture, "/element-text").await)
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_inner_text_reads_the_rendered_text_where_text_content_reads_the_markup() {
    let Some(tab) = open_element_text().await else {
        return;
    };
    let outcome = run(
        &tab,
        &format!(
            "const caught = {CAUGHT};
             return {{
               inner: await page.locator('#mixed').innerText(),
               text: await page.locator('#mixed').textContent(),
               hidden: await page.locator('#hidden').innerText(),
               firstOfTwo: await page.innerText('#list > li'),
               viaFrame: await page.mainFrame().innerText('#list > li >> nth=1'),
               svg: await page.locator('#label').innerText({{ timeout: 5000 }}).catch(caught),
             }};"
        ),
    )
    .await;
    let result = value(&outcome);
    assert_eq!(
        result["inner"], "Hello LOUD\nnext line",
        "uppercased, the <br> a line break, the display:none span left out"
    );
    assert_eq!(result["text"], "Hello loudnext line secret");
    assert_eq!(
        result["hidden"], "Hidden text",
        "an element that is not rendered reads as its text content"
    );
    assert_eq!(
        result["firstOfTwo"], "First",
        "page.innerText is not strict"
    );
    assert_eq!(result["viaFrame"], "Second");
    assert_eq!(
        result["svg"],
        kind("Error", false),
        "an SVG element is not an HTMLElement: refused at once, not timed out"
    );

    let inner = &outcome.calls[0];
    assert_eq!(
        (inner.method.as_str(), &inner.result),
        ("innerText", &CallResult::Ok)
    );
    assert_eq!(inner.delivery, None, "a read writes no input");
    tab.close().await;
}

/// A missing attribute answers `null` at once: the call is not caught, so a read that waited
/// for the attribute would time out and fail the run.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_get_attribute_reads_present_empty_and_missing_attributes() {
    let Some(tab) = open_element_text().await else {
        return;
    };
    let outcome = run(
        &tab,
        "const missing = await page.locator('#mixed').getAttribute('data-missing', { timeout: 1000 });
         return {
           missing,
           missingType: typeof missing,
           present: await page.locator('#mixed').getAttribute('title'),
           empty: await page.getAttribute('#mixed', 'data-empty'),
           hidden: await page.mainFrame().getAttribute('#hidden', 'data-state'),
         };",
    )
    .await;
    let result = value(&outcome);
    assert_eq!(
        (result.get("missing"), &result["missingType"]),
        (Some(&Value::Null), &json!("object")),
        "a missing attribute reads as null, not undefined"
    );
    assert_eq!(result["present"], "Mixed text");
    assert_eq!(
        result["empty"], "",
        "an empty attribute reads as '', not null"
    );
    assert_eq!(
        result["hidden"], "hidden",
        "a hidden element's attribute is read"
    );

    let missing = &outcome.calls[0];
    assert_eq!(
        (missing.method.as_str(), &missing.result),
        ("getAttribute", &CallResult::Ok)
    );
    assert_eq!(missing.delivery, None, "a read writes no input");
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_get_attribute_and_inner_text_wait_for_elements_appended_later() {
    let Some(tab) = open_element_text().await else {
        return;
    };
    tab.evaluate(
        "for (const [id, ms] of [['late-attribute', 600], ['late-text', 1200]]) \
         setTimeout(() => { const p = document.createElement('p'); p.id = id; \
         p.dataset.state = 'arrived'; p.textContent = 'Arrived'; \
         document.body.appendChild(p); }, ms)",
    )
    .await;
    let outcome = run(
        &tab,
        "const state = await page.getAttribute('#late-attribute', 'data-state', { timeout: 10000 });
         const text = await page.locator('#late-text').innerText({ timeout: 10000 });
         return { state, text };",
    )
    .await;
    assert_eq!(
        value(&outcome),
        json!({ "state": "arrived", "text": "Arrived" }),
        "each read waited for its element"
    );
    tab.close().await;
}
