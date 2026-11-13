//! The host's native inventory over its authenticated routes, joined to real retained traces.

use crate::support::{self, Op, Start, TestResult, harness};
use axum::http::StatusCode;
use browseros_core::TabId;
use neo_trace::{
    Delivery, Effect, OperationEnded, OperationId, Outcome, PageState, RetentionPolicy,
};
use serde_json::json;

#[tokio::test]
async fn all_open_native_tabs_include_distinct_task_history_and_current_metadata() -> TestResult {
    let h = harness().await?;
    for tab in [3, 5, 88, 77] {
        h.browser.open_tab(tab);
    }
    // The host inventory includes native tabs the agent's page cache excludes.
    h.browser.set_url(77, "chrome-extension://test/panel.html");
    for (task, tab) in [
        (Some("zulu"), 3),
        (Some("alpha"), 3),
        (Some("alpha"), 3),
        (Some("closed-tab"), 9),
        (None, 88),
    ] {
        let run = support::start_run(
            &h.sink,
            Start {
                task,
                ..Start::default()
            },
        );
        support::op_started(&h.sink, OperationId::new(run, 1), Op::write(tab, 1));
        if task == Some("zulu") {
            support::op_started(&h.sink, OperationId::new(run, 2), Op::write(5, 2));
        }
    }
    support::start_run(
        &h.sink,
        Start {
            task: Some("no-tab"),
            ..Start::default()
        },
    );
    // A newPage operation gets its tab only at the end; it must associate that task too.
    let run = support::start_run(
        &h.sink,
        Start {
            task: Some("created"),
            ..Start::default()
        },
    );
    let new_page = OperationId::new(run, 1);
    support::op_started(
        &h.sink,
        new_page,
        Op {
            name: "context.newPage",
            effect: Effect::Write,
            tab: None,
            at: 2,
        },
    );
    h.sink.op_ended(OperationEnded {
        id: new_page,
        outcome: Outcome::Ok,
        delivery: Delivery::Done,
        sent_at: None,
        result: None,
        error: None,
        page_after: Some(PageState {
            tab_id: Some(TabId(5)),
            url: None,
            title: None,
        }),
        main_world: false,
        frames_incomplete: false,
        at: support::at(3),
    });
    h.flushed().await?;
    let reply = h.get("/v1/tabs").await?;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.request_id().is_some());
    assert_eq!(
        reply.body,
        json!({"items": [
            {"tabId":3,"targetId":"target-3","url":"https://example.test/3","title":"Tab 3","taskIds":["alpha","zulu"]},
            {"tabId":5,"targetId":"target-5","url":"https://example.test/5","title":"Tab 5","taskIds":["created","zulu"]},
            {"tabId":88,"targetId":"target-88","url":"https://example.test/88","title":"Tab 88","taskIds":[]},
            {"tabId":77,"targetId":"target-77","url":"chrome-extension://test/panel.html","title":"Tab 77","taskIds":[]}
        ]})
    );
    let _: neo_sdk_api::TabList = serde_json::from_value(reply.body)?;
    h.browser.close_tab(3);
    h.browser.set_url(5, "https://example.test/changed");
    let changed = h.get("/v1/tabs").await?;
    assert_eq!(changed.body["items"].as_array().map(Vec::len), Some(3));
    assert_eq!(changed.body["items"][0]["tabId"], 5);
    assert_eq!(
        changed.body["items"][0]["url"],
        "https://example.test/changed"
    );
    Ok(())
}

#[tokio::test]
async fn retention_removes_task_associations_without_removing_live_tabs() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(3);
    let run = support::start_run(
        &h.sink,
        Start {
            task: Some("old"),
            ..Start::default()
        },
    );
    support::op_started(&h.sink, OperationId::new(run, 1), Op::write(3, 1));
    support::op_ended(&h.sink, OperationId::new(run, 1), Delivery::Done, None);
    support::end_run(&h.sink, run, None);
    h.flushed().await?;
    assert_eq!(
        h.get("/v1/tabs").await?.body["items"][0]["taskIds"],
        json!(["old"])
    );
    let eight_days = 8 * 24 * 60 * 60 * 1000;
    h.sink
        .sweep(RetentionPolicy::default(), support::at(eight_days))
        .await?;
    let after = h.get("/v1/tabs").await?;
    assert_eq!(after.body["items"][0]["tabId"], 3);
    assert_eq!(after.body["items"][0]["taskIds"], json!([]));
    Ok(())
}

#[tokio::test]
async fn an_empty_browser_is_empty_and_an_unreachable_browser_is_a_503() -> TestResult {
    let h = harness().await?;
    let empty = h.get("/v1/tabs").await?;
    assert_eq!(empty.status, StatusCode::OK);
    assert_eq!(empty.body, json!({"items":[]}));
    h.browser.become_unreachable();
    h.get("/v1/tabs")
        .await?
        .assert_error(StatusCode::SERVICE_UNAVAILABLE, "browser_unavailable");
    Ok(())
}

#[tokio::test]
async fn a_failed_association_query_is_a_generic_500_not_an_empty_task_list() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(3);
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(h.dir.path().join("trace.db")),
    )
    .await?;
    sqlx::query("DROP TABLE operations").execute(&pool).await?;
    pool.close().await;
    let reply = h.get("/v1/tabs").await?;
    reply.assert_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error");
    assert_eq!(reply.body["message"], "internal error");
    Ok(())
}

/// Neo's blank tab keeps the browser open when every other tab closes. The host sees it, marked
/// `owner: neo`; every other tab has no `owner`.
#[tokio::test]
async fn the_tab_neo_keeps_for_itself_is_listed_with_its_owner() -> TestResult {
    let h = harness().await?;
    for tab in [3, 4] {
        h.browser.open_tab(tab);
    }
    *h.neo_tab.lock().map_err(|_| "poisoned")? = Some(4);
    let reply = h.get("/v1/tabs").await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let items = reply.body["items"].as_array().ok_or("items")?;
    let owner_of = |tab: i64| {
        items
            .iter()
            .find(|item| item["tabId"] == tab)
            .map(|item| item.get("owner").cloned())
    };
    assert_eq!(owner_of(4), Some(Some(json!("neo"))), "Neo's tab, marked");
    assert_eq!(
        owner_of(3),
        Some(None),
        "every other tab has no owner field"
    );
    Ok(())
}
