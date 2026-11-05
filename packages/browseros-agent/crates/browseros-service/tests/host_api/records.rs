//! The record routes over a real store: runs listed and filtered, one run with its operations,
//! one operation, and the trace tail.

use crate::support::{
    EventStream, Op, Start, TestResult, at, end_run, harness, op_ended, op_started, start_run,
};
use axum::http::StatusCode;
use browseros_core::TabId;
use browseros_trace::{
    Delivery, Effect, OperationEnded, OperationId, OperationKind, OperationStarted, Outcome,
    PageState, RunEnded, RunId, RunStarted, RunStatus, SecretSet, SecretSetConfig, SecretValue,
    TargetSensitivity, TaskId,
};
use serde_json::json;
use std::sync::Arc;
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
    assert_eq!(tail.body["visuals"], "available");

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
async fn an_operations_notes_read_back_masked_on_the_operation_and_its_run() -> TestResult {
    let h = harness().await?;
    let secret = "hunter2-swordfish";
    let registered = h
        .post(
            "/v1/secrets",
            &json!({"value": secret, "ttlSeconds": 600}).to_string(),
        )
        .await?;
    assert_eq!(registered.status, StatusCode::NO_CONTENT);
    let run = start_run(&h.sink, Start::default());
    let click = OperationId::new(run, 1);
    let title = OperationId::new(run, 2);
    op_started(&h.sink, click, Op::write(3, 1));
    h.sink.op_notes(
        click,
        vec![
            "waiting for getByRole('button')".to_string(),
            format!("  typed {secret}"),
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
        "  typed [private]",
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
async fn a_value_only_a_records_also_mask_knows_reads_masked_in_every_channel() -> TestResult {
    let h = harness().await?;
    let mut live = EventStream::open(&h, "/v1/events", None).await?;
    // What claw-sdk still holds unregistered (sdk-traces decision 32): the record's own set.
    let held_value = "held-7Qx-swordfish";
    let held = SecretSet::new(SecretSetConfig::default());
    let _registration = held.register(SecretValue::new(held_value.to_string()), None);
    let held = Some(Arc::new(held));
    let run = RunId::new();
    h.sink.run_started(RunStarted {
        id: run,
        code: format!("return '{held_value}'"),
        request_id: None,
        task_id: Some(TaskId::parse("checkout")?),
        host_data: Some(json!({"for": held_value})),
        also_mask: held.clone(),
        at: at(0),
    });
    let read = OperationId::new(run, 1);
    h.sink.op_started(OperationStarted {
        id: read,
        parent: None,
        name: "page.title".to_string(),
        kind: OperationKind::Builtin,
        effect: Effect::Read,
        tab: Some(TabId(3)),
        args: json!({"expected": held_value}),
        policy: Arc::default(),
        also_mask: held.clone(),
        at: at(1),
    });
    h.sink.op_notes(read, vec![format!("read {held_value}")]);
    h.sink.op_ended(OperationEnded {
        id: read,
        outcome: Outcome::Ok,
        delivery: Delivery::Done,
        sent_at: None,
        result: Some(json!({"title": format!("Welcome {held_value}")})),
        error: None,
        page_after: Some(PageState {
            page_id: Some(3),
            tab_id: Some(TabId(3)),
            url: Some(format!("https://b.test/?name={held_value}")),
            title: Some(format!("Welcome {held_value}")),
        }),
        target: TargetSensitivity::Ordinary,
        also_mask: held.clone(),
        at: at(2),
    });
    h.sink.run_ended(RunEnded {
        id: run,
        status: RunStatus::Ok,
        error: None,
        result: Some(json!({"echo": held_value})),
        replay: None,
        also_mask: held,
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
            !text.contains(held_value),
            "{channel} keeps the held value: {text}"
        );
    }
    let masked = |channel: &str| {
        channels
            .iter()
            .any(|(name, body)| name == channel && body.to_string().contains("[private]"))
    };
    for channel in [
        format!("/v1/runs/{run}"),
        format!("/v1/operations/{read}"),
        "/v1/trace?taskId=checkout".to_string(),
        "live operation.ended".to_string(),
        "replayed operation.ended".to_string(),
    ] {
        assert!(masked(&channel), "{channel} does not show the mask");
    }

    let wal = match fs::read(h.dir.path().join("trace.db-wal")) {
        Ok(wal) => wal,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    for file in [fs::read(h.dir.path().join("trace.db"))?, wal] {
        assert!(
            !file
                .windows(held_value.len())
                .any(|window| window == held_value.as_bytes()),
            "trace.db keeps the held value"
        );
    }
    Ok(())
}
