//! A default response captured on the pre-summary base pins the compatibility boundary.
//! The base fixture used the same recorder with listener sites absent, matching its dispatcher.
//! Runtime tests separately prove public listener calls produce the sites recorded here.

use crate::support::{Harness, TestResult, at, harness};
use axum::http::StatusCode;
use browseros_core::TabId;
use neo_trace::{
    Delivery, Effect, OperationEnded, OperationId, OperationKind, OperationSite, OperationStarted,
    Outcome, PageState, RunEnded, RunId, RunStarted, RunStatus, TaskId,
};
use serde_json::{Value, json};

const SCRIPT: &str =
    "page.on('dialog', () => {});\ncontext.on('pageload', () => {});\nreturn await page.title();";

async fn record_listeners(h: &Harness) -> Result<RunId, crate::support::BoxError> {
    let run: RunId = "0193f3c2-0000-7000-8000-000000000354".parse()?;
    h.sink.run_started(RunStarted {
        id: run,
        code: SCRIPT.into(),
        request_id: None,
        task_id: Some(TaskId::parse("listener-compatibility")?),
        host_data: Some(json!({"fixture": "default-listeners"})),
        at: at(0),
    });
    let calls = [
        (
            "events.listen",
            None,
            Some(3),
            json!({"event": "workerstate", "enabled": true}),
        ),
        (
            "events.listen",
            Some(OperationSite { line: 1, column: 1 }),
            Some(3),
            json!({"call": {"api": "page.on"}, "event": "dialog", "enabled": true}),
        ),
        (
            "events.listen",
            Some(OperationSite { line: 2, column: 1 }),
            None,
            json!({"call": {"api": "browserContext.on"}, "event": "pageload", "enabled": true}),
        ),
        (
            "page.title",
            Some(OperationSite {
                line: 3,
                column: 14,
            }),
            Some(3),
            json!({"page": 3}),
        ),
        ("events.next", None, None, json!({})),
    ];
    for (seq, (name, site, tab, args)) in (1_u32..).zip(calls) {
        let id = OperationId::new(run, seq);
        h.sink.op_started(OperationStarted {
            id,
            site,
            parent: None,
            name: name.into(),
            kind: OperationKind::Builtin,
            effect: Effect::Read,
            tab: tab.map(TabId),
            args,
            at: at(i64::from(seq)),
        });
        h.sink.op_ended(OperationEnded {
            id,
            outcome: Outcome::Ok,
            delivery: Delivery::Done,
            sent_at: Some(at(i64::from(seq) + 1)),
            result: Some(if name == "page.title" {
                json!("Compatibility")
            } else {
                Value::Null
            }),
            error: None,
            page_after: tab.map(|tab| PageState {
                tab_id: Some(TabId(tab)),
                url: Some("https://compat.test/".into()),
                title: Some("Compatibility".into()),
            }),
            main_world: false,
            frames_incomplete: false,
            at: at(100 + i64::from(seq)),
        });
    }
    h.sink.run_ended(RunEnded {
        id: run,
        status: RunStatus::Ok,
        error: None,
        result: Some(json!("Compatibility")),
        replay: None,
        at: at(200),
    });
    h.flushed().await?;
    Ok(run)
}

#[tokio::test]
async fn default_listener_response_changes_only_internal_and_explicit_listener_sites() -> TestResult
{
    let h = harness().await?;
    let run = record_listeners(&h).await?;
    let baseline: Value = serde_json::from_str(include_str!(
        "fixtures/default-listeners-base-26d99d358.json"
    ))?;
    assert_eq!(
        baseline["baseCommit"],
        "26d99d3586caa95bf2126bc6cbf34396401b676f"
    );
    let mut expected = baseline["run"].clone();
    let operations = expected["operations"]
        .as_array_mut()
        .ok_or("baseline operations")?;
    for (index, internal) in [true, false, false, false, true].into_iter().enumerate() {
        let op = operations[index]
            .as_object_mut()
            .ok_or("baseline operation")?;
        assert!(!op.contains_key("internal"));
        op.insert("internal".into(), json!(internal));
        if index == 1 || index == 2 {
            assert!(
                !op.contains_key("site"),
                "base listeners had no script site"
            );
            op.insert("site".into(), json!({"line": index, "column": 1}));
        }
    }
    let reply = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.body, expected,
        "no other default response field may change"
    );
    for operation in reply.body["operations"].as_array().ok_or("operations")? {
        let id = operation["id"].as_str().ok_or("operation id")?;
        let single = h.get(&format!("/v1/operations/{id}")).await?;
        assert_eq!(single.status, StatusCode::OK);
        assert_eq!(
            &single.body, operation,
            "single-operation reads use the same default shape"
        );
    }
    let mut expected_tail = baseline["trace"].clone();
    expected_tail["operations"] = expected["operations"].clone();
    let tail = h.get("/v1/trace?taskId=listener-compatibility").await?;
    assert_eq!(tail.status, StatusCode::OK);
    assert_eq!(tail.body, expected_tail);
    Ok(())
}
