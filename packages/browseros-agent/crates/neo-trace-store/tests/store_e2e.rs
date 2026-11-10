//! Neo: BrowserOS operations persisted through a real Runtime. Raw SQLite and WAL bytes are
//! checked before checkpointing and after reopening, so query-time scrubbing cannot pass.

#![cfg(test)]

#[path = "../../neo-runtime/tests/support/browser.rs"]
#[expect(
    dead_code,
    reason = "the store test uses only the isolated browser and page evaluator"
)]
mod browser;
mod support;

use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use neo_runtime::{Delivery, GetterField, PageId, RunOrigin, RunRequest, RunStatus, Runtime};
use neo_trace_store::{GetterOutcome, StoreConfig, TraceStore};
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn assert_no_secret(path: &Path, secret: &str) {
    for file in [path.to_path_buf(), path.with_file_name("traces.sqlite-wal")] {
        let bytes = match fs::read(&file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound && file != path => continue,
            Err(error) => panic!("read {}: {error}", file.display()),
        };
        assert!(
            !bytes
                .windows(secret.len())
                .any(|part| part == secret.as_bytes()),
            "secret in {}",
            file.display()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn login_and_page_lifecycle_survive_reopen_without_password_bytes() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = browser::Tab::open(binary, browser::Fixture {
        pages: vec![("/login".to_string(), "<form onsubmit='window.clicked=true; return false'><input id='password' type='password'><button id='go'>Log in</button></form>".to_string())],
        ..browser::Fixture::default()
    }, "/start").await;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("traces.sqlite");
    let traces = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap()
        .with_sinks([traces.sink()]);
    let secret = "store-e2e-password'never-persist-9427";
    let literal = r"store-e2e-password\'never-persist-9427";
    let url = format!("{}/login", tab.origin);
    let outcome = runtime.run(RunRequest {
        code: format!("await page.goto({}); await page.fill('#password', '{literal}'); await page.click('#go'); const created = await context.newPage(); await created.close(); return page.url();", json!(url)),
        page: PageId::new(tab.target_id.clone()),
        timeout: Duration::from_secs(30),
        cancel: CancellationToken::new(),
        origin: RunOrigin::default(),
        artifacts: None,
    }).await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!(url)));
    assert_eq!(tab.evaluate("window.clicked").await, json!(true));
    assert_eq!(
        tab.evaluate("document.querySelector('#password').value")
            .await,
        json!(secret)
    );
    runtime.drain().await;
    traces.flushed().await;
    // Inspect the live WAL too: a close/checkpoint might otherwise conceal a transient leak.
    assert!(
        path.with_file_name("traces.sqlite-wal").exists(),
        "inspect the live WAL before checkpointing"
    );
    assert_no_secret(&path, secret);
    assert_no_secret(&path, literal);
    let reopened = support::reopen(&runtime, traces, &path).await;
    let (run, calls) = support::complete(&reopened, outcome.run_id).await;
    support::matches_outcome(&run, &calls, &outcome);
    assert_eq!(
        calls
            .iter()
            .map(|call| call.method.as_str())
            .collect::<Vec<_>>(),
        ["goto", "fill", "click", "newPage", "close"]
    );
    assert_eq!(calls[1].delivery, Some(Delivery::Done));
    assert_eq!(calls[2].delivery, Some(Delivery::Done));
    assert_ne!(calls[1].params["value"], json!(secret));
    assert_eq!(run.pages.len(), 2);
    assert!(run.pages.iter().all(|page| page.page.tab_id.is_some()));
    // Context.newPage has no target yet; the created page joins it through by_call.
    for call in calls.iter().filter(|call| call.method != "newPage") {
        let page = call.page.as_ref().unwrap();
        assert!(page.tab_id.is_some());
        assert!(run.pages.iter().any(|row| &row.page == page));
    }
    let created = run.pages.iter().find(|page| page.created).unwrap();
    assert_eq!(created.by_call, Some(calls[3].id.no));
    assert!(created.closed_offset_us.is_some());
    assert_eq!(calls[4].page.as_ref(), Some(&created.page));
    let getters = reopened.getters(&outcome.run_id).await.unwrap();
    let urls: Vec<_> = getters
        .iter()
        .filter(|getter| getter.field == GetterField::Url)
        .collect();
    assert_eq!(urls.len(), 1);
    assert_eq!(
        urls[0].outcome,
        GetterOutcome::Ok {
            summary: json!(url)
        }
    );
    assert_eq!(
        urls[0].page.as_ref().unwrap().target,
        PageId::new(tab.target_id.clone())
    );
    assert_no_secret(&path, secret);
    assert_no_secret(&path, literal);
    support::assert_clean(&reopened.close(support::BUDGET).await);
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_created_contexts_ready_page_uses_the_shared_registry_capture_point() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = browser::Tab::open(binary, browser::Fixture::default(), "/start").await;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("traces.sqlite");
    let traces = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap()
        .with_sinks([traces.sink()]);
    let outcome = runtime.run(RunRequest {
        code: "const isolated = await context.browser().newContext(); const child = await isolated.newPage(); const url = child.url(); await isolated.close(); return url;".to_string(),
        page: PageId::new(tab.target_id.clone()), timeout: Duration::from_secs(30),
        cancel: CancellationToken::new(), origin: RunOrigin::default(), artifacts: None,
    }).await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!("about:blank")));
    let reopened = support::reopen(&runtime, traces, &path).await;
    let (run, calls) = support::complete(&reopened, outcome.run_id).await;
    support::matches_outcome(&run, &calls, &outcome);
    let created: Vec<_> = run.pages.iter().filter(|page| page.created).collect();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].by_call, Some(2));
    assert!(created[0].closed_offset_us.is_some());
    support::assert_clean(&reopened.close(support::BUDGET).await);
    tab.close().await;
}
