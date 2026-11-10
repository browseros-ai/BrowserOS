//! Neo: A real page proves fill/click records preserve raw code, params and input delivery.
//! The shared harness uses a fresh headless profile and closes only its owned browser.

#[path = "../../neo-runtime/tests/support/browser.rs"]
#[expect(
    dead_code,
    reason = "persistence needs only basic native fixture operations"
)]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{PageId, RunRequest, RunStatus, StartPage};
use neo_trace::TraceStore;
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// Neo: A deterministic page records normal form input and click effects independently of traces.
const FORM: &str = "<input id='field'><button id='go' onclick='window.clicked=true'>Go</button>";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fill_and_click_persist_code_params_and_delivery_from_a_real_page() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![("/form.html".to_owned(), FORM.to_owned())],
            ..Fixture::default()
        },
        "/form.html",
    )
    .await;
    let temporary = TempDir::new().expect("temporary state");
    let path = temporary.path().join("traces.db");
    let store = TraceStore::open(&path).await.expect("open");
    let code = "await page.locator('#field').fill('raw value');\nawait page.click('#go');";
    let target_id = tab.target_id.clone();
    let runtime = tab.runtime().await;
    let (id, outcome) = neo_trace::run(
        &runtime,
        RunRequest {
            code: code.to_owned(),
            page: StartPage::Page(PageId::new(tab.target_id.clone())),
            timeout: Duration::from_secs(30),
            cancel: CancellationToken::new(),
            artifacts: None,
        },
        Some(&store),
    )
    .await;
    let effects = tab
        .evaluate("({value:document.querySelector('#field').value, clicked:window.clicked})")
        .await;
    runtime.drain().await;
    store.close().await;
    tab.close().await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(effects, json!({"value":"raw value", "clicked":true}));
    let reopened = TraceStore::open(&path)
        .await
        .expect("read after committed close");
    let detail = reopened.run(id).await.expect("read").expect("run exists");
    assert_eq!(detail.code, code);
    assert_eq!(detail.run.id, id.to_string());
    assert_eq!(detail.run.status, "ok");
    assert_eq!(detail.run.page.as_deref(), Some(target_id.as_str()));
    assert_eq!(detail.run.call_count, 2);
    assert_eq!(detail.calls.len(), 2);
    for (index, (row, call)) in detail.calls.iter().zip(&outcome.calls).enumerate() {
        assert_eq!(row.seq, i64::try_from(index + 1).expect("small sequence"));
        assert_eq!(row.method, call.method);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&row.params).expect("JSON params"),
            call.params
        );
        assert_eq!(row.result.as_deref(), Some("ok"));
        assert_eq!(row.delivery.as_deref(), Some("done"));
        assert!(row.duration_ms.is_some());
    }
    assert_eq!(detail.calls[0].method, "fill");
    assert_eq!(detail.calls[1].method, "click");
    reopened.close().await;
}
