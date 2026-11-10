//! The navigation and page-reading acceptance run, end to end on a real BrowserOS neo: one Neo
//! run (`neo-quickjs-injected/tests/scripts/navigation-acceptance.js`) opens a fixture page with
//! `page.goto`, reads its title, AI snapshot and text, clicks by a snapshot ref and by role with
//! the existing `locator.click`, reads the change, follows a link to a second document and reads
//! that one, then shows the old ref no longer resolves.
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary, for example
//! `/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo`; otherwise it passes as a skip.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "one run through the shared support needs only part of it"
)]
mod browser;

use browser::{Fixture, Tab};
use neo_runtime::RunStatus;
use serde_json::{Value, json};

const ACCEPTANCE: &str =
    include_str!("../../neo-quickjs-injected/tests/scripts/navigation-acceptance.js");

const START: &str = r#"<!doctype html>
<html><head><title>Neo acceptance</title></head>
<body>
<main>
  <h1>Counter</h1>
  <p id="status">Clicked 0 times</p>
  <button id="increment">Increment</button>
  <a href="/next">Next page</a>
</main>
<script>
  let clicks = 0;
  document.getElementById('increment').addEventListener('click', () => {
    clicks += 1;
    document.getElementById('status').textContent = `Clicked ${clicks} times`;
  });
</script>
</body></html>"#;

const NEXT: &str = r#"<!doctype html>
<html><head><title>Next</title></head>
<body><h1>Arrived</h1><p id="message">Second document</p></body></html>"#;

#[tokio::test(flavor = "multi_thread")]
async fn one_run_navigates_reads_clicks_and_reads_the_change() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let fixture = Fixture {
        pages: vec![
            ("/start".to_string(), START.to_string()),
            ("/next".to_string(), NEXT.to_string()),
        ],
        ..Fixture::default()
    };
    let tab = Tab::open(binary, fixture, "/blank").await;
    let outcome = tab.start_run(ACCEPTANCE.to_string()).await.await.unwrap();
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    let value = outcome.value.clone().unwrap_or(Value::Null);
    let field = |name: &str| value.get(name).cloned().unwrap_or(Value::Null);
    let text = |name: &str| field(name).as_str().unwrap_or_default().to_string();

    assert_eq!(
        field("navigated"),
        json!({"url":"/start","status":200,"ok":true,
               "contentType":"text/html; charset=utf-8","bodyHasTitle":true}),
        "goto resolves the document's response: {value:#}"
    );
    assert_eq!(text("title"), "Neo acceptance", "{value:#}");
    let reference = text("ref");
    let snapshot = text("snapshot");
    assert!(
        reference.starts_with('e') && reference[1..].chars().all(|c| c.is_ascii_digit()),
        "a main-frame ref has no frame prefix: {reference}"
    );
    for line in [
        format!("button \"Increment\" [ref={reference}]"),
        "heading \"Counter\" [level=1]".to_string(),
        "Clicked 0 times".to_string(),
        "link \"Next page\"".to_string(),
        "/url: /next".to_string(),
    ] {
        assert!(
            snapshot.contains(&line),
            "{line:?} missing from:\n{snapshot}"
        );
    }
    assert_eq!(
        [
            text("before"),
            text("afterRefClick"),
            text("afterRoleClick")
        ],
        ["Clicked 0 times", "Clicked 1 times", "Clicked 2 times"],
        "{value:#}"
    );
    assert_eq!(
        [text("nextUrl"), text("nextTitle"), text("nextText")],
        ["/next", "Next", "Second document"],
        "reads after a click that navigates land in the new document: {value:#}"
    );
    assert_eq!(
        text("nextSnapshot"),
        "- heading \"Arrived\" [level=1]\n- paragraph: Second document",
        "the default-mode snapshot of the new document"
    );
    assert_eq!(
        field("staleRef"),
        json!("TimeoutError"),
        "a ref from the previous document resolves nothing in the new one"
    );
}
