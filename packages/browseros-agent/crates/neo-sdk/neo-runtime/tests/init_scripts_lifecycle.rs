//! A run removes every init script it added, from every session that shows it, before it
//! returns, however it ends: the run's tab session also takes its scripts with it when it
//! detaches, but that detach goes out in the background after the run returned, where the next
//! run may already be navigating (`run_sessions.rs`). Against the fake browser in
//! `support/fake_browser.rs`, which records each command and its answer. No browser needed;
//! `init_scripts_e2e.rs` shows the same on a real BrowserOS neo.

#[path = "support/fake_browser.rs"]
#[expect(
    dead_code,
    reason = "the page's interceptor and click readers serve click_lifecycle.rs"
)]
mod fake_browser;

use std::time::Duration;

use fake_browser::{Exchange, FakeBrowser, Replies};
use neo_runtime::RunStatus;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

/// A page script and a context script, both on the run's tab.
const ADD: &str = "await page.addInitScript(() => { window.__page = 1 })
await page.context().addInitScript('window.__context = 1')";

/// Each `(session, identifier)` the fake answered `method` for, or was asked `method` of.
fn scripts(exchanges: &[Exchange], method: &str) -> Vec<(String, String)> {
    let mut scripts: Vec<(String, String)> = exchanges
        .iter()
        .filter(|exchange| exchange.method() == method)
        .filter_map(|exchange| {
            let identifier = exchange.command["params"]["identifier"]
                .as_str()
                .or_else(|| exchange.reply.as_ref()?["result"]["identifier"].as_str())?;
            Some((
                exchange.command["sessionId"].as_str()?.to_string(),
                identifier.to_string(),
            ))
        })
        .collect();
    scripts.sort();
    scripts
}

/// The sessions the fake was asked to turn auto-attach `on` or off for, in order.
fn auto_attach(exchanges: &[Exchange], on: bool) -> Vec<String> {
    exchanges
        .iter()
        .filter(|exchange| {
            exchange.method() == "Target.setAutoAttach"
                && exchange.command["params"]["autoAttach"] == Value::Bool(on)
        })
        .filter_map(|exchange| exchange.command["sessionId"].as_str().map(str::to_string))
        .collect()
}

/// Runs `code` against the fake and checks what the fake had been asked when the run returned.
async fn removed_on_return(code: &str, timeout: Duration, status: RunStatus) {
    let browser = FakeBrowser::start(Replies::default()).await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            code,
            timeout,
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, status, "{outcome:?}");

    let exchanges = browser.exchanges();
    let added = scripts(&exchanges, "Page.addScriptToEvaluateOnNewDocument");
    assert_eq!(
        added.len(),
        2,
        "both scripts reached the tab: {exchanges:?}"
    );
    assert_eq!(
        scripts(&exchanges, "Page.removeScriptToEvaluateOnNewDocument"),
        added,
        "each script was removed before the run returned: {exchanges:?}"
    );
    let tab = vec![added[0].0.clone()];
    assert_eq!(auto_attach(&exchanges, true), tab, "{exchanges:?}");
    assert_eq!(
        auto_attach(&exchanges, false),
        tab,
        "no iframe is held for the run's scripts after it: {exchanges:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_that_returns_removes_its_init_scripts_first() {
    removed_on_return(
        &format!("{ADD}\nreturn 1"),
        Duration::from_secs(5),
        RunStatus::Ok,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_ended_by_its_deadline_removes_its_init_scripts_first() {
    removed_on_return(
        &format!("{ADD}\nawait new Promise(() => {{}})"),
        Duration::from_secs(1),
        RunStatus::Timeout,
    )
    .await;
}
