//! Read views over the same persisted trace, including values in non-argument fields.

use crate::support::{Op, Start, TestResult, at, end_run, harness, op_started, start_run};
use axum::http::StatusCode;
use browseros_core::TabId;
use neo_trace::{
    Delivery, DownloadStatus, Effect, ErrorOwner, ErrorRecord, OperationEnded, OperationId,
    OperationKind, OperationSite, OperationStarted, Outcome, PageState, RunDownload, RunEnded,
    RunId, RunStarted, RunStatus, TaskId, TraceStore,
};
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[tokio::test]
async fn audit_summary_does_not_load_or_decode_unused_payload_columns() -> TestResult {
    let h = harness().await?;
    let run = start_run(
        &h.sink,
        Start {
            task: Some("metadata-only"),
            ..Start::default()
        },
    );
    op_started(&h.sink, OperationId::new(run, 1), Op::write(3, 1));
    end_run(&h.sink, run, None);
    h.flushed().await?;
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(h.dir.path().join("trace.db")))
        .await?;
    sqlx::query("UPDATE operations SET args = 'invalid-json', result = 'invalid-json', notes = 'invalid-json' WHERE run_id = ?")
        .bind(run.to_string()).execute(&pool).await?;
    sqlx::query("UPDATE runs SET host_data = 'invalid-json', result = 'invalid-json' WHERE id = ?")
        .bind(run.to_string())
        .execute(&pool)
        .await?;
    pool.close().await;

    let summary = h
        .get("/v1/runs?taskId=metadata-only&fields=summary&internal=false")
        .await?;
    assert_eq!(summary.status, StatusCode::OK, "{}", summary.body);
    assert_eq!(
        summary.body["items"][0]["operations"][0]["name"],
        "locator.click"
    );
    h.get(&format!("/v1/runs/{run}"))
        .await?
        .assert_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error");
    Ok(())
}

#[tokio::test]
async fn summary_omits_values_everywhere_and_keeps_audit_metadata() -> TestResult {
    let h = harness().await?;
    let value = "trace-summary-canary-7qP";
    let run = RunId::new();
    let fill = OperationId::new(run, 1);
    let error = ErrorRecord {
        owner: ErrorOwner::Website,
        code: "timeout".into(),
        message: format!("Could not fill {value}"),
        host_code: Some(value.into()),
        retryable: false,
        operation_id: Some(fill),
    };
    h.sink.run_started(RunStarted {
        id: run,
        code: format!("await page.getByRole('textbox').fill('{value}')"),
        request_id: None,
        task_id: Some(TaskId::parse("summary-canary")?),
        host_data: Some(json!({"value": value})),
        at: at(0),
    });
    h.sink.op_started(OperationStarted {
        site: Some(OperationSite { line: 1, column: 7 }),
        id: fill,
        parent: None,
        name: "locator.fill".into(),
        kind: OperationKind::Builtin,
        effect: Effect::Write,
        tab: Some(TabId(3)),
        args: json!({"value": value}),
        at: at(1),
    });
    h.sink.op_notes(fill, vec![format!("filling with {value}")]);
    h.sink.op_ended(OperationEnded {
        id: fill,
        outcome: Outcome::Error,
        delivery: Delivery::Unknown,
        sent_at: Some(at(2)),
        result: Some(json!({"echo": value})),
        error: Some(error.clone()),
        page_after: Some(PageState {
            tab_id: Some(TabId(3)),
            url: Some("https://example.test/form".into()),
            title: Some("Form".into()),
        }),
        main_world: false,
        at: at(3),
    });
    h.sink.run_ended(RunEnded {
        id: run,
        status: RunStatus::Error,
        error: Some(error),
        result: Some(json!({"echo": value})),
        replay: None,
        at: at(4),
    });
    h.flushed().await?;

    let store = TraceStore::open(&h.dir.path().join("trace.db")).await?;
    store
        .outputs()
        .record_downloads(
            run,
            &[RunDownload {
                index: 0,
                url: format!("https://example.test/{value}"),
                suggested_filename: value.into(),
                path: Some(value.into()),
                size: None,
                state: DownloadStatus::Failed,
                failure: Some(value.into()),
            }],
        )
        .await?;

    for uri in [format!("/v1/runs/{run}"), "/v1/trace".into()] {
        let original = h.get(&uri).await?;
        assert_eq!(original.status, StatusCode::OK);
        assert!(original.body.to_string().contains(value));
        let summary = h.get(&format!("{uri}?fields=summary")).await?;
        assert_eq!(summary.status, StatusCode::OK, "{}", summary.body);
        assert!(
            !summary.body.to_string().contains(value),
            "{}",
            summary.body
        );
        let op = &summary.body["operations"][0];
        for key in [
            "args",
            "argsTruncated",
            "result",
            "resultTruncated",
            "notes",
        ] {
            assert!(op.get(key).is_none(), "{key}: {op}");
        }
        assert_eq!(op["name"], "locator.fill");
        assert_eq!(op["site"], json!({"line": 1, "column": 7}));
        assert_eq!(op["effect"], "write");
        assert_eq!(op["outcome"], "error");
        assert_eq!(op["delivery"], "unknown");
        assert_eq!(op["durationMs"], 2);
        assert_eq!(op["tabId"], 3);
        assert_eq!(op["page"]["url"], "https://example.test/form");
        assert_eq!(op["page"]["tabId"], 3);
        assert_eq!(op["page"]["title"], "Form");
        assert_eq!(op["error"]["owner"], "website");
        assert_eq!(op["error"]["code"], "timeout");
        assert_eq!(op["error"]["message"], "");
        assert!(op["error"].get("hostCode").is_none());
        assert_eq!(op["internal"], false);
        let full = h.get(&format!("{uri}?fields=full&internal=true")).await?;
        assert_eq!(full.body, original.body, "explicit defaults");
        let reread = h.get(&uri).await?;
        assert_eq!(reread.body, original.body, "a view never rewrites values");
    }
    let full = h.get(&format!("/v1/runs/{run}")).await?;
    for key in ["code", "hostData", "result", "downloads"] {
        assert!(full.body[key].to_string().contains(value), "{key}");
    }
    let audit = h
        .get("/v1/runs?taskId=summary-canary&fields=summary&internal=false")
        .await?;
    assert_eq!(audit.status, StatusCode::OK, "{}", audit.body);
    assert!(!audit.body.to_string().contains(value), "{}", audit.body);
    let audit_op = &audit.body["items"][0]["operations"][0];
    assert!(audit_op.get("site").is_none());
    assert!(audit_op.get("delivery").is_none());
    assert_eq!(audit_op["page"]["title"], "Form");
    Ok(())
}

#[tokio::test]
async fn audit_lists_paged_runs_with_steps_and_keeps_other_tasks_out() -> TestResult {
    let h = harness().await?;
    let first = start_run(
        &h.sink,
        Start {
            task: Some("audit"),
            ..Start::default()
        },
    );
    op_started(&h.sink, OperationId::new(first, 1), Op::write(3, 1));
    end_run(&h.sink, first, None);
    let second = start_run(
        &h.sink,
        Start {
            task: Some("audit"),
            at: 10,
            ..Start::default()
        },
    );
    op_started(&h.sink, OperationId::new(second, 1), Op::write(4, 11));
    op_started(
        &h.sink,
        OperationId::new(second, 2),
        Op {
            name: "events.listen",
            effect: Effect::Read,
            tab: Some(9),
            at: 12,
        },
    );
    let other = start_run(
        &h.sink,
        Start {
            task: Some("other"),
            at: 20,
            ..Start::default()
        },
    );
    op_started(&h.sink, OperationId::new(other, 1), Op::write(8, 21));
    h.flushed().await?;

    let prefix = "/v1/runs?taskId=audit&limit=1&fields=summary&internal=false";
    let latest = h.get(prefix).await?;
    assert_eq!(latest.status, StatusCode::OK);
    let item = &latest.body["items"][0];
    assert_eq!(item["id"], second.to_string());
    assert_eq!(item["status"], "running");
    assert_eq!(item["complete"], false);
    assert_eq!(
        item["operationCount"], 2,
        "the count describes execution, not the view"
    );
    let ops = item["operations"].as_array().ok_or("audit operations")?;
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["tabId"], 4);
    assert!(ops[0].get("outcome").is_none());
    let cursor = latest.body["nextCursor"]
        .as_str()
        .ok_or("older runs cursor")?;
    let older = h.get(&format!("{prefix}&cursor={cursor}")).await?;
    assert_eq!(older.body["items"][0]["id"], first.to_string());
    assert!(older.body.get("nextCursor").is_none());
    let default = h.get("/v1/runs?taskId=audit").await?;
    assert!(default.body["items"][0].get("operations").is_none());
    Ok(())
}

#[tokio::test]
async fn internal_filter_preserves_script_listeners_and_counts_visible_tail_rows() -> TestResult {
    let h = harness().await?;
    let run = start_run(&h.sink, Start::default());
    for (seq, name, site) in [
        (1, "locator.fill", None),
        (
            2,
            "events.listen",
            Some(OperationSite { line: 2, column: 1 }),
        ),
        (3, "events.listen", None),
        (4, "events.next", None),
        (5, "events.interceptFileChooser", None),
    ] {
        h.sink.op_started(OperationStarted {
            site,
            id: OperationId::new(run, seq),
            parent: None,
            name: name.into(),
            kind: OperationKind::Builtin,
            effect: Effect::Read,
            tab: Some(TabId(3)),
            args: json!({"value": "kept-in-full"}),
            at: at(i64::from(seq)),
        });
    }
    end_run(&h.sink, run, None);
    h.flushed().await?;
    for uri in [format!("/v1/runs/{run}"), "/v1/trace".into()] {
        let original = h.get(&uri).await?;
        assert_eq!(
            original.body["operations"].as_array().map(Vec::len),
            Some(5)
        );
        let internal: Vec<_> = original.body["operations"]
            .as_array()
            .ok_or("operations")?
            .iter()
            .map(|op| op["internal"].clone())
            .collect();
        assert_eq!(
            internal,
            vec![
                json!(false),
                json!(false),
                json!(true),
                json!(true),
                json!(true)
            ]
        );
        for options in ["internal=false", "fields=summary&internal=false"] {
            let filtered = h.get(&format!("{uri}?{options}")).await?;
            assert_eq!(filtered.status, StatusCode::OK, "{}", filtered.body);
            let ops = filtered.body["operations"].as_array().ok_or("operations")?;
            assert_eq!(ops.len(), 2);
            assert_eq!(ops[1]["name"], "events.listen");
            assert_eq!(ops[1]["site"]["line"], 2);
            assert!(ops.iter().all(|op| op["internal"] == false));
        }
    }
    for (tail, truncated, count) in [(1, true, 1), (2, false, 2)] {
        let filtered = h
            .get(&format!("/v1/trace?tail={tail}&internal=false"))
            .await?;
        assert_eq!(filtered.body["truncated"], truncated);
        assert_eq!(
            filtered.body["operations"].as_array().map(Vec::len),
            Some(count)
        );
        assert_eq!(
            filtered.body["operations"][count - 1]["name"],
            "events.listen"
        );
    }
    Ok(())
}

#[tokio::test]
async fn trace_view_options_reject_bad_values_and_duplicates() -> TestResult {
    let h = harness().await?;
    let run = start_run(&h.sink, Start::default());
    op_started(&h.sink, OperationId::new(run, 1), Op::write(3, 1));
    h.flushed().await?;
    for uri in [format!("/v1/runs/{run}"), "/v1/trace".into()] {
        for options in [
            "fields=small",
            "fields=",
            "fields=full&fields=summary",
            "internal=0",
            "internal=",
            "internal=true&internal=false",
        ] {
            h.get(&format!("{uri}?{options}"))
                .await?
                .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
        }
    }
    Ok(())
}
