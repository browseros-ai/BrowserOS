//! The record routes over a real store: runs listed and filtered, one run with its operations,
//! one operation, and the trace tail.

use crate::support::{
    BoxError, EventStream, Harness, Op, Start, TestResult, at, end_run, harness, op_ended,
    op_started, start_run,
};
use axum::http::StatusCode;
use browseros_core::TabId;
use neo_trace::{
    Delivery, Effect, OperationEnded, OperationId, OperationKind, OperationSite, OperationStarted,
    Outcome, PageState, RunEnded, RunId, RunStarted, RunStatus, TaskId,
};
use serde_json::json;
use std::{fs, io};

#[tokio::test]
async fn runs_filter_by_request_task_tab_status_and_time() -> TestResult {
    let h = harness().await?;
    let checkout = start_run(
        &h.sink,
        Start {
            task: Some("checkout"),
            request: Some("call-1"),
            at: 0,
        },
    );
    op_started(&h.sink, OperationId::new(checkout, 1), Op::write(3, 1));
    end_run(&h.sink, checkout, None);
    let search = start_run(
        &h.sink,
        Start {
            task: Some("search"),
            request: Some("call-2"),
            at: 10,
        },
    );
    op_started(&h.sink, OperationId::new(search, 1), Op::write(4, 11));
    let untasked = start_run(
        &h.sink,
        Start {
            at: 20,
            ..Start::default()
        },
    );
    h.flushed().await?;

    let [checkout, search, untasked] = [checkout, search, untasked].map(|run| run.to_string());
    let since = at(10).millis();
    let cases = [
        ("".to_string(), vec![&untasked, &search, &checkout]),
        ("requestId=call-1".to_string(), vec![&checkout]),
        ("taskId=search".to_string(), vec![&search]),
        ("tab=3".to_string(), vec![&checkout]),
        ("status=running".to_string(), vec![&untasked, &search]),
        ("status=ok".to_string(), vec![&checkout]),
        (format!("since={since}"), vec![&untasked, &search]),
        (format!("until={since}"), vec![&search, &checkout]),
        ("taskId=checkout&status=running".to_string(), vec![]),
    ];
    for (query, expected) in cases {
        let reply = h.get(&format!("/v1/runs?{query}")).await?;
        assert_eq!(reply.status, StatusCode::OK, "{query}");
        assert_eq!(
            reply.run_ids().iter().collect::<Vec<_>>(),
            expected,
            "{query}"
        );
    }

    let listed = h.get("/v1/runs?taskId=checkout").await?;
    let run = &listed.body["items"][0];
    assert_eq!(run["requestId"], "call-1");
    assert_eq!(run["tabs"], json!([3]));
    assert_eq!(run["status"], "ok");
    assert_eq!(run["complete"], true, "an ended run is complete");
    let running = h.get("/v1/runs?taskId=search").await?;
    assert_eq!(running.body["items"][0]["complete"], false);
    Ok(())
}

#[tokio::test]
async fn runs_page_newest_first_with_the_cursor_neo_gave() -> TestResult {
    let h = harness().await?;
    let runs = (0..5)
        .map(|i| {
            start_run(
                &h.sink,
                Start {
                    at: i,
                    ..Start::default()
                },
            )
            .to_string()
        })
        .collect::<Vec<_>>();
    h.flushed().await?;
    let mut seen = Vec::new();
    let mut uri = "/v1/runs?limit=2".to_string();
    loop {
        let page = h.get(&uri).await?;
        assert_eq!(page.status, StatusCode::OK);
        seen.extend(page.run_ids());
        match page.body["nextCursor"].as_str() {
            Some(cursor) => uri = format!("/v1/runs?limit=2&cursor={cursor}"),
            None => break,
        }
    }
    assert_eq!(seen, runs.into_iter().rev().collect::<Vec<_>>());
    Ok(())
}

#[tokio::test]
async fn a_bad_cursor_or_limit_is_a_400_not_a_500() -> TestResult {
    let h = harness().await?;
    for query in [
        "cursor=12",
        "cursor=soon:0193f3c2-0000-7000-8000-000000000000",
        "cursor=12:not-a-run",
        "limit=0",
        "limit=101",
        "limit=ten",
    ] {
        h.get(&format!("/v1/runs?{query}"))
            .await?
            .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    Ok(())
}

#[tokio::test]
async fn a_run_reads_with_its_operations_and_the_writes_left_unknown() -> TestResult {
    let h = harness().await?;
    let run = start_run(
        &h.sink,
        Start {
            task: Some("checkout"),
            ..Start::default()
        },
    );
    let goto = OperationId::new(run, 1);
    let pay = OperationId::new(run, 2);
    let title = OperationId::new(run, 3);
    op_started(&h.sink, goto, Op::write(3, 1));
    op_ended(&h.sink, goto, Delivery::Done, None);
    op_started(&h.sink, pay, Op::write(3, 2));
    op_ended(&h.sink, pay, Delivery::Unknown, None);
    op_started(
        &h.sink,
        title,
        Op {
            name: "page.title",
            effect: Effect::Read,
            tab: Some(3),
            at: 3,
        },
    );
    h.flushed().await?;

    let running = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(running.status, StatusCode::OK);
    assert_eq!(running.body["status"], "running");
    assert_eq!(running.body["complete"], false);
    assert!(
        running.body.get("effects").is_none(),
        "no verdict while running"
    );
    assert_eq!(
        running.body["unknown"],
        json!([{"operationId": pay.to_string(), "name": "locator.click"}])
    );
    let operations = running.body["operations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let ids = operations
        .iter()
        .map(|op| op["id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(ids, [goto, pay, title].map(|id| json!(id.to_string())));
    assert!(operations[2].get("delivery").is_none(), "still running");

    end_run(&h.sink, run, Some(json!({"paid": true})));
    h.flushed().await?;
    let ended = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(ended.body["status"], "ok");
    assert_eq!(ended.body["complete"], true);
    assert_eq!(ended.body["effects"], "unknown");
    assert_eq!(ended.body["result"], json!({"paid": true}));

    let operation = h.get(&format!("/v1/operations/{pay}")).await?;
    assert_eq!(operation.status, StatusCode::OK);
    assert_eq!(operation.body["runId"], run.to_string());
    assert_eq!(operation.body["effect"], "write");
    assert_eq!(operation.body["delivery"], "unknown");
    assert_eq!(operation.body["tabId"], 3);
    assert_eq!(operation.body["hasScreenshot"], false);
    h.get(&format!("/v1/operations/{}", OperationId::new(run, 9)))
        .await?
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    Ok(())
}

#[tokio::test]
async fn the_trace_tail_keeps_the_latest_matching_operations_with_their_runs() -> TestResult {
    let h = harness().await?;
    let first = start_run(
        &h.sink,
        Start {
            task: Some("checkout"),
            ..Start::default()
        },
    );
    let second = start_run(
        &h.sink,
        Start {
            task: Some("search"),
            at: 30,
            ..Start::default()
        },
    );
    let third = start_run(
        &h.sink,
        Start {
            task: Some("checkout"),
            at: 50,
            ..Start::default()
        },
    );
    op_started(&h.sink, OperationId::new(first, 1), Op::write(3, 10));
    op_started(&h.sink, OperationId::new(first, 2), Op::write(4, 20));
    op_started(&h.sink, OperationId::new(second, 1), Op::write(3, 40));
    op_started(&h.sink, OperationId::new(third, 1), Op::write(3, 60));
    h.flushed().await?;

    let ids = |body: &serde_json::Value, key: &str| {
        body[key]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| item["id"].clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let op = |run, seq| json!(OperationId::new(run, seq).to_string());
    let tail = h.get("/v1/trace?taskId=checkout&tail=2").await?;
    assert_eq!(tail.status, StatusCode::OK);
    assert_eq!(
        ids(&tail.body, "operations"),
        vec![op(first, 2), op(third, 1)]
    );
    assert_eq!(
        ids(&tail.body, "runs"),
        [first, third].map(|run| json!(run.to_string()))
    );
    assert_eq!(tail.body["truncated"], true);

    let tab = h.get("/v1/trace?tab=3").await?;
    assert_eq!(
        ids(&tab.body, "operations"),
        vec![op(first, 1), op(second, 1), op(third, 1)]
    );
    assert_eq!(tab.body["truncated"], false);
    let both = h.get("/v1/trace?taskId=checkout&tab=3").await?;
    assert_eq!(
        ids(&both.body, "operations"),
        vec![op(first, 1), op(third, 1)]
    );
    h.get("/v1/trace?tail=1001")
        .await?
        .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    Ok(())
}

#[tokio::test]
async fn an_operations_notes_read_back_as_given_on_the_operation_and_its_run() -> TestResult {
    let h = harness().await?;
    let value = "hunter2-swordfish";
    let run = start_run(&h.sink, Start::default());
    let click = OperationId::new(run, 1);
    let title = OperationId::new(run, 2);
    op_started(&h.sink, click, Op::write(3, 1));
    h.sink.op_notes(
        click,
        vec![
            "waiting for getByRole('button')".to_string(),
            format!("  typed {value}"),
            "attempting click action".to_string(),
        ],
    );
    op_ended(&h.sink, click, Delivery::Done, None);
    op_started(
        &h.sink,
        title,
        Op {
            name: "page.title",
            effect: Effect::Read,
            tab: Some(3),
            at: 2,
        },
    );
    op_ended(&h.sink, title, Delivery::Done, None);
    h.flushed().await?;

    let notes = json!([
        "waiting for getByRole('button')",
        format!("  typed {value}"),
        "attempting click action"
    ]);
    let operation = h.get(&format!("/v1/operations/{click}")).await?;
    assert_eq!(operation.status, StatusCode::OK);
    assert_eq!(operation.body["notes"], notes);
    let detail = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(detail.status, StatusCode::OK);
    assert_eq!(detail.body["operations"][0]["notes"], notes);
    assert!(
        detail.body["operations"][1].get("notes").is_none(),
        "an operation that noted nothing has no notes: {}",
        detail.body["operations"][1]
    );
    let quiet = h.get(&format!("/v1/operations/{title}")).await?;
    assert_eq!(quiet.status, StatusCode::OK);
    assert!(quiet.body.get("notes").is_none(), "{}", quiet.body);
    Ok(())
}

#[tokio::test]
async fn record_values_read_back_as_given_in_every_channel() -> TestResult {
    let h = harness().await?;
    let mut live = EventStream::open(&h, "/v1/events", None).await?;
    let value = "held-7Qx-swordfish";
    let run = RunId::new();
    h.sink.run_started(RunStarted {
        id: run,
        code: format!("return '{value}'"),
        request_id: None,
        task_id: Some(TaskId::parse("checkout")?),
        host_data: Some(json!({"for": value})),
        at: at(0),
    });
    let read = OperationId::new(run, 1);
    h.sink.op_started(OperationStarted {
        site: None,
        id: read,
        parent: None,
        name: "page.title".to_string(),
        kind: OperationKind::Builtin,
        effect: Effect::Read,
        tab: Some(TabId(3)),
        args: json!({"expected": value}),
        at: at(1),
    });
    h.sink.op_notes(read, vec![format!("read {value}")]);
    h.sink.op_ended(OperationEnded {
        id: read,
        outcome: Outcome::Ok,
        delivery: Delivery::Done,
        sent_at: None,
        result: Some(json!({"title": format!("Welcome {value}")})),
        error: None,
        page_after: Some(PageState {
            tab_id: Some(TabId(3)),
            url: Some(format!("https://b.test/?name={value}")),
            title: Some(format!("Welcome {value}")),
        }),
        main_world: false,
        at: at(2),
    });
    h.sink.run_ended(RunEnded {
        id: run,
        status: RunStatus::Ok,
        error: None,
        result: Some(json!({"echo": value})),
        replay: None,
        at: at(3),
    });
    h.flushed().await?;

    let mut channels = Vec::new();
    for uri in [
        format!("/v1/runs/{run}"),
        "/v1/runs?taskId=checkout".to_string(),
        format!("/v1/operations/{read}"),
        "/v1/trace?taskId=checkout".to_string(),
    ] {
        let reply = h.get(&uri).await?;
        assert_eq!(reply.status, StatusCode::OK, "{uri}: {}", reply.body);
        channels.push((uri, reply.body));
    }
    let events = live.take(4).await?;
    // The event log replays what it published from memory, after the client's last event.
    let mut resumed = EventStream::open(&h, "/v1/events", Some(&events[0].id)).await?;
    let replayed = resumed.take(3).await?;
    for (label, list) in [("live", &events), ("replayed", &replayed)] {
        for event in list {
            channels.push((format!("{label} {}", event.kind), event.data.clone()));
        }
    }
    for (channel, body) in &channels {
        let text = body.to_string();
        assert!(
            !text.contains("pageId"),
            "{channel} exposes legacy identity: {text}"
        );
        assert!(
            !text.contains("[private]"),
            "{channel} unexpectedly masks the value: {text}"
        );
    }
    let keeps_value = |channel: &str| {
        channels
            .iter()
            .any(|(name, body)| name == channel && body.to_string().contains(value))
    };
    for channel in [
        format!("/v1/runs/{run}"),
        format!("/v1/operations/{read}"),
        "/v1/trace?taskId=checkout".to_string(),
        "live operation.ended".to_string(),
        "replayed operation.ended".to_string(),
    ] {
        assert!(keeps_value(&channel), "{channel} does not keep the value");
    }

    assert!(
        store_holds(&h, value)?,
        "trace.db stores the original value"
    );
    Ok(())
}

#[tokio::test]
async fn starts_defer_input_and_ends_publish_the_original_values() -> TestResult {
    let h = harness().await?;
    let mut live = EventStream::open(&h, "/v1/events", None).await?;
    let found = "deferred-input-9Qp";
    let run = RunId::new();
    h.sink.run_started(RunStarted {
        id: run,
        code: format!("return '{found}'"),
        request_id: None,
        task_id: Some(TaskId::parse("checkout")?),
        host_data: Some(json!({"for": found})),
        at: at(0),
    });
    let fill = OperationId::new(run, 1);
    h.sink.op_started(OperationStarted {
        site: None,
        id: fill,
        parent: None,
        name: "locator.fill".to_string(),
        kind: OperationKind::Builtin,
        effect: Effect::Write,
        tab: Some(TabId(3)),
        args: json!({"value": found}),
        at: at(1),
    });
    h.flushed().await?;
    let started = live.take(2).await?;
    let open = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(open.status, StatusCode::OK);
    for body in started.iter().map(|event| &event.data).chain([&open.body]) {
        let text = body.to_string();
        assert!(
            !text.contains(found),
            "an open record shows the value: {text}"
        );
    }

    assert!(!store_holds(&h, found)?, "start input stays off disk");
    h.sink.op_ended(OperationEnded {
        id: fill,
        outcome: Outcome::Ok,
        delivery: Delivery::Done,
        sent_at: None,
        result: None,
        error: None,
        page_after: None,
        main_world: false,
        at: at(2),
    });
    h.sink.run_ended(RunEnded {
        id: run,
        status: RunStatus::Ok,
        error: None,
        result: None,
        replay: None,
        at: at(3),
    });
    h.flushed().await?;
    let ended = live.take(2).await?;
    let detail = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(detail.body["hostData"], json!({"for": found}));
    assert_eq!(
        detail.body["operations"][0]["args"],
        json!({"value": found})
    );
    assert_eq!(detail.body["code"], format!("return '{found}'"));
    let operation_end = ended
        .iter()
        .find(|event| event.kind == "operation.ended")
        .ok_or("the operation's end event")?;
    assert!(operation_end.data.to_string().contains(found));
    let run_end = ended
        .iter()
        .find(|event| event.kind == "run.ended")
        .ok_or("the run's summary event")?;
    assert_eq!(run_end.data["run"]["id"], run.to_string());
    assert!(
        store_holds(&h, found)?,
        "trace.db stores the original value"
    );
    Ok(())
}

/// Whether the store file or its WAL holds `needle`. Both are read whole, and only the WAL may
/// be absent (after a checkpoint): a read that fails fails the test, or the scan would pass
/// without looking.
fn store_holds(h: &Harness, needle: &str) -> Result<bool, BoxError> {
    let wal = match fs::read(h.dir.path().join("trace.db-wal")) {
        Ok(wal) => wal,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let files = [fs::read(h.dir.path().join("trace.db"))?, wal];
    Ok(files.iter().any(|file| {
        file.windows(needle.len())
            .any(|window| window == needle.as_bytes())
    }))
}

/// The optional field is shared by all operation routes. Absent means no script location,
/// rather than a fabricated coordinate or JSON null for internal and historical work.
#[tokio::test]
async fn operation_routes_return_script_sites_and_omit_missing_sites() -> TestResult {
    let h = harness().await?;
    let run = start_run(&h.sink, Start::default());
    let id = OperationId::new(run, 1);
    h.sink.op_started(OperationStarted {
        site: Some(OperationSite {
            line: 2,
            column: 17,
        }),
        id,
        parent: None,
        name: "page.title".into(),
        kind: OperationKind::Builtin,
        effect: Effect::Read,
        tab: None,
        args: json!({}),
        at: at(1),
    });
    op_started(&h.sink, OperationId::new(run, 2), Op::write(3, 2));
    h.flushed().await?;
    let detail = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(detail.status, StatusCode::OK);
    assert_eq!(
        detail.body["operations"][0]["site"],
        json!({"line": 2, "column": 17})
    );
    assert!(detail.body["operations"][1].get("site").is_none());
    let operation = h.get(&format!("/v1/operations/{id}")).await?;
    assert_eq!(operation.status, StatusCode::OK);
    assert_eq!(operation.body["site"], detail.body["operations"][0]["site"]);
    let tail = h.get("/v1/trace").await?;
    assert_eq!(tail.body["operations"][0]["site"], operation.body["site"]);
    Ok(())
}

/// Every operation route says whether an ended operation ran script in the page's main world,
/// true or false, and leaves `mainWorld` out while one runs, rather than guessing.
#[tokio::test]
async fn operation_routes_say_which_operations_ran_in_the_main_world() -> TestResult {
    let h = harness().await?;
    let run = start_run(&h.sink, Start::default());
    let evaluate = OperationId::new(run, 1);
    let click = OperationId::new(run, 2);
    let running = OperationId::new(run, 3);
    for (id, name) in [
        (evaluate, "page.evaluate"),
        (click, "locator.click"),
        (running, "page.goto"),
    ] {
        op_started(
            &h.sink,
            id,
            Op {
                name,
                ..Op::write(3, 1)
            },
        );
    }
    for (id, main_world) in [(evaluate, true), (click, false)] {
        h.sink.op_ended(OperationEnded {
            id,
            outcome: Outcome::Ok,
            delivery: Delivery::Done,
            sent_at: Some(at(2)),
            result: None,
            error: None,
            page_after: None,
            main_world,
            at: at(3),
        });
    }
    h.flushed().await?;
    let detail = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.body);
    let in_run: Vec<_> = detail.body["operations"]
        .as_array()
        .ok_or("the run's operations")?
        .iter()
        .map(|op| (op["name"].clone(), op.get("mainWorld").cloned()))
        .collect();
    assert_eq!(
        in_run,
        [
            (json!("page.evaluate"), Some(json!(true))),
            (json!("locator.click"), Some(json!(false))),
            (json!("page.goto"), None),
        ]
    );
    let operation = h.get(&format!("/v1/operations/{evaluate}")).await?;
    assert_eq!(operation.body["mainWorld"], json!(true));
    let tail = h.get("/v1/trace").await?;
    let in_tail: Vec<_> = tail.body["operations"]
        .as_array()
        .ok_or("the tail's operations")?
        .iter()
        .map(|op| op.get("mainWorld").cloned())
        .collect();
    assert_eq!(in_tail, [Some(json!(true)), Some(json!(false)), None]);
    Ok(())
}
