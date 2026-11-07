//! Whether an agent-driven page can tell it is being automated, against a real browser.
//!
//! The page runs the checks sites use to spot an open DevTools and blank or close themselves:
//! timing `console.table` (whose arguments the inspector serializes only while `Runtime` is
//! enabled), getters on logged objects, and `debugger` pauses. The test opens it through the
//! same attach path agent tabs take, reads it, snapshots it, clicks it and collects its console
//! the way tools do, then requires that no check fired and that the click's error was still
//! collected.
//!
//! Needs a BrowserOS (or Chromium) instance with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p browseros-core --test devtools_detection_live -- --ignored`. The test opens
//! and closes its own tab.

use base64::Engine;
use browseros_cdp::{CdpClient, ConnectOptions};
use browseros_core::{
    PageJs, PageSignals,
    page_js::set_isolated_world_enabled,
    pages::{NewPageOptions, PageManager, PageManagerHooks},
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

const DETECTOR: &str = r#"<!doctype html><title>detector</title>
<p id="text">Resume</p><button id="save">Save</button>
<script>
(() => {
  const det = { hits: {}, rounds: 0 };
  window.__det = det;
  const hit = (name) => { det.hits[name] = (det.hits[name] || 0) + 1; };
  const big = Array.from({ length: 50 }, (_, i) => {
    const row = {};
    for (let k = 0; k < 400; k++) row['k' + k] = 'v' + i + '_' + k;
    return row;
  });
  setInterval(() => {
    det.rounds++;
    const t0 = performance.now();
    console.table(big);
    const tableMs = performance.now() - t0;
    const t1 = performance.now();
    console.log(big);
    const logMs = performance.now() - t1;
    if (tableMs > 5 || logMs > 5) hit('consoleTiming');
    const err = new Error('probe');
    Object.defineProperty(err, 'stack', { get() { hit('errorStackGetter'); return ''; } });
    console.debug(err);
    const div = document.createElement('div');
    Object.defineProperty(div, 'id', { get() { hit('elementIdGetter'); return 'x'; } });
    console.debug(div);
    const t2 = performance.now();
    debugger;
    if (performance.now() - t2 > 50) hit('debuggerPause');
  }, 100);
  document.getElementById('save').addEventListener('click', () => {
    console.error('save failed: validation');
  });
})();
</script>"#;

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_reading_clicking_and_console_collection_trip_no_devtools_check()
-> Result<(), Box<dyn std::error::Error>> {
    set_isolated_world_enabled(true);
    let port = std::env::var("BROWSEROS_CDP_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(9005);
    let client = Arc::new(CdpClient::connect(ConnectOptions::new(port)).await?);
    let pages = PageManager::new(client.clone(), PageManagerHooks::default());
    let page_id = pages
        .new_blank_page(NewPageOptions {
            background: Some(true),
            ..NewPageOptions::default()
        })
        .await?;
    let result = exercise(&client, &pages, page_id.clone()).await;
    let _ = pages.close(page_id).await;
    result
}

async fn exercise(
    client: &Arc<CdpClient>,
    pages: &PageManager,
    page_id: browseros_core::PageId,
) -> Result<(), Box<dyn std::error::Error>> {
    let page = pages.get_session(page_id.clone()).await?;
    let signals = PageSignals::new(client.clone());
    signals.attach_page(page_id.clone(), page.session_id.clone());

    let url = format!(
        "data:text/html;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(DETECTOR)
    );
    page.session
        .send_value("Page.navigate", json!({ "url": url }))
        .await?;
    let mut loaded = false;
    for _attempt in 0..50 {
        if PageJs::page_world(&page.session)
            .eval::<bool>("!!window.__det")
            .await
            .unwrap_or(false)
        {
            loaded = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(loaded, "detector page did not load");

    // Reads and snapshots alone first: a harvest disables `Runtime`, which would also switch
    // off reporting that attaching wrongly left on.
    for round in 0..30 {
        let text: String = PageJs::isolated(&page.session)
            .eval("document.getElementById('text').textContent")
            .await?;
        assert_eq!(text, "Resume");
        let _: Value = page
            .session
            .send_value("Accessibility.getFullAXTree", json!({}))
            .await?;
        if round == 20 {
            let _: bool = PageJs::isolated(&page.session)
                .eval("(document.getElementById('save').click(), true)")
                .await?;
        }
        if round >= 20 {
            signals.harvest_console(&page_id, &page.session).await?;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let detector: Value = PageJs::page_world(&page.session)
        .eval("JSON.parse(JSON.stringify(window.__det))")
        .await?;
    // Focus emulation keeps the background tab's timers at full speed, so the checks really ran.
    assert!(
        detector["rounds"].as_u64().unwrap_or(0) >= 20,
        "detector barely ran: {detector}"
    );
    assert_eq!(
        detector["hits"],
        json!({}),
        "devtools checks fired: {detector}"
    );
    let collected = signals.console_entries(&page_id);
    assert!(
        collected
            .iter()
            .any(|entry| entry.text == "save failed: validation"),
        "the click's console error was not collected: {collected:?}"
    );
    Ok(())
}
