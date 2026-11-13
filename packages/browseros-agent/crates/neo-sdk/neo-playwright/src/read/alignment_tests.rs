//! Task 006's boundary changes, through the real route: immediate versus waiting queries,
//! omitted selector-wait defaults, unchanged invalid element-state calls, and reply bytes.

use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use crate::test_support::{ProbeAnswer, ScriptedRun, ScriptedTab, TabScript, TestResult};
use crate::{ErrorCode, Op};

/// The element is attached immediately and becomes visible on the second read. That makes
/// an accidental attached default, or a query that starts waiting, observable without a clock.
#[derive(Default)]
struct ReadPage {
    reads: AtomicUsize,
}

impl TabScript for ReadPage {
    fn probe(&self, _frame: &str, request: &Value) -> ProbeAnswer {
        if request["op"] == "selector" {
            return ProbeAnswer::reply(json!({ "kind": "selector", "selector": "#go" }));
        }
        let read = self.reads.fetch_add(1, Ordering::SeqCst);
        ProbeAnswer::reply(json!({ "kind": "read", "value": {
            "pin": 1, "preview": "<button>Go</button>", "visible": read > 0, "count": 1
        } }))
    }

    fn cdp_reply(&self, method: &str, params: &Value) -> Option<Value> {
        (method == "Runtime.callFunctionOn"
            && params["arguments"][0]["value"]["request"]["op"] == "pinned")
            .then(|| {
                json!({ "result": {
                "type": "object", "subtype": "node", "objectId": "pinned-node"
            } })
            })
    }
}

fn target(run: &ScriptedRun) -> Value {
    json!({ "$neo": "locator", "page": run.page, "frame": null,
        "chunks": [{ "parts": [{ "name": "css", "body": "#go", "source": "#go" }] }],
        "display": "locator('#go')" })
}

fn op(name: &str) -> TestResult<Op> {
    Ok(Op::from_name(name).ok_or("Read op missing")?)
}

#[tokio::test]
async fn query_is_immediate_and_omitted_wait_fields_mean_visible_and_a_handle() -> TestResult {
    for query in [true, false] {
        let tab = ScriptedTab::new(ReadPage::default());
        let run = ScriptedRun::over(&tab).await?;
        let result = run
            .playwright
            .invoke(
                &run.env,
                op(if query {
                    "locator.elementHandle"
                } else {
                    "locator.waitFor"
                })?,
                json!({ "call": { "timeoutMs": 2_000 }, "target": target(&run),
                    "strict": "strict" }),
            )
            .await?;
        assert_eq!(
            result["$neo"], "element",
            "omitting omitReturnValue returns the handle"
        );
        assert_eq!(
            tab.script().reads.load(Ordering::SeqCst),
            if query { 1 } else { 2 }
        );
    }
    Ok(())
}

#[tokio::test]
async fn normalize_renames_only_the_route_reply_and_keeps_the_probe_selector() -> TestResult {
    let tab = ScriptedTab::new(ReadPage::default());
    let run = ScriptedRun::over(&tab).await?;
    let result = run
        .playwright
        .invoke(
            &run.env,
            op("locator.normalize")?,
            json!({ "target": target(&run), "selector": "locator('#go')" }),
        )
        .await?;
    assert_eq!(result.to_string(), r##"{"resolvedSelector":"#go"}"##);
    Ok(())
}

#[tokio::test]
async fn optional_wait_fields_still_reject_null_and_a_handle_still_requires_its_state() -> TestResult
{
    let tab = ScriptedTab::new(ReadPage::default());
    let run = ScriptedRun::over(&tab).await?;
    for field in ["state", "omitReturnValue"] {
        let mut args = json!({ "target": target(&run), "strict": "strict" });
        args[field] = Value::Null;
        let error = run
            .playwright
            .invoke(&run.env, op("locator.waitFor")?, args)
            .await
            .err()
            .ok_or("null must fail")?;
        assert_eq!(error.error_code(), Some(ErrorCode::InvalidInput));
        assert!(
            error
                .message()
                .contains(&format!("{field}: invalid type: null"))
        );
    }
    let handle = run
        .playwright
        .invoke(
            &run.env,
            op("locator.elementHandle")?,
            json!({ "target": target(&run), "strict": "strict" }),
        )
        .await?;
    let error = run
        .playwright
        .invoke(
            &run.env,
            op("locator.waitFor")?,
            json!({
                "call": { "api": "elementHandle.waitForElementState" }, "target": handle,
                "strict": "strict", "omitReturnValue": true
            }),
        )
        .await
        .err()
        .ok_or("an element-state call needs its state")?;
    assert_eq!(
        error.message(),
        "elementHandle.waitForElementState: missing field `state`"
    );
    Ok(())
}

#[tokio::test]
async fn the_query_decoder_no_longer_accepts_the_wait_switch() -> TestResult {
    let tab = ScriptedTab::new(ReadPage::default());
    let run = ScriptedRun::over(&tab).await?;
    let error = run
        .playwright
        .invoke(
            &run.env,
            op("locator.elementHandle")?,
            json!({
                "target": target(&run), "strict": "strict", "noWait": true
            }),
        )
        .await
        .err()
        .ok_or("the old switch must fail")?;
    assert_eq!(error.error_code(), Some(ErrorCode::InvalidInput));
    assert!(error.message().contains("unknown field `noWait`"));
    assert_eq!(tab.script().reads.load(Ordering::SeqCst), 0);
    Ok(())
}
