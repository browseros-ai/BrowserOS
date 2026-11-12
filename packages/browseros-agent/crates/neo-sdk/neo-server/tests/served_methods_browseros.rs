//! What the script's realm receives on a real BrowserOS, through MCP `run`, for a method Neo does
//! not serve: the prelude's refusal is a door call (FR/RC), which keeps its code in the run's error
//! and is one row of the run's trace.
//!
//! ```text
//!  throw __neo.refuse('page.route', …) ─ the door: refused, recorded ─▶ script / method_unavailable
//! ```
//!
//! Like `e2e_browseros.rs`, it runs only when `CLAW_SDK_E2E_BROWSER` names a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use browseros_rig::{CaptchaKey, HOST_TOKEN, Rig, TestResult};
use neo_server::test_support;
use serde_json::{Value, json};
use tokio::time;

/// The whole drive; see `e2e_browseros.rs` for why the budget sits inside the test.
const TEST_BUDGET: Duration = Duration::from_secs(240);

/// What the Playwright prelude raises for `page.route`, which Neo does not serve.
const ROUTE_REASON: &str = "page.route: network interception is not available in Neo";

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_is_one_trace_row_and_fails_the_run_with_its_code() -> TestResult {
    on_browseros(|rig| Box::pin(refusal_on_the_record(rig))).await
}

/// `drive` on a fresh BrowserOS within the test's budget, with both processes' logs on failure.
async fn on_browseros(
    drive: impl for<'a> FnOnce(&'a Rig) -> Pin<Box<dyn Future<Output = TestResult> + 'a>>,
) -> TestResult {
    let Some(browser) = browseros_rig::browser_or_skip() else {
        return Ok(());
    };
    let rig = Rig::start(&browser, CaptchaKey::Absent)?;
    let failure = match time::timeout(TEST_BUDGET, drive(&rig)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

async fn refusal_on_the_record(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let code = format!(
        "throw __neo.refuse('page.route', 'method_unavailable', {});",
        json!(ROUTE_REASON)
    );
    let result = browseros_rig::run(&client, &code, &json!({ "taskId": "refusal" })).await?;
    client.cancel().await?;
    let meta = browseros_rig::outcome(&result)?;
    let operation = meta
        .pointer("/error/operationId")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the run's error names the refusal's op: {result:?}"))?;
    assert_eq!(
        meta.get("error"),
        Some(&json!({
            "owner": "script", "code": "method_unavailable", "message": ROUTE_REASON,
            "retryable": true, "operationId": operation, "delivery": "not_sent",
        })),
        "the run fails with the refusal's code: {result:?}"
    );
    let seq = operation
        .rsplit_once('.')
        .map(|(_, seq)| seq)
        .ok_or_else(|| format!("an op id is `<run>.<seq>`: {operation}"))?;
    assert_eq!(
        test_support::run_value(&result),
        Err(format!(
            "script / method_unavailable: {ROUTE_REASON} (op {seq}, delivery not_sent)"
        )),
        "what the model reads"
    );

    let id = meta
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the run has an id: {meta}"))?;
    let read = rig
        .host(
            "GET",
            &format!("/v1/runs/{id}?wait=30"),
            Some(HOST_TOKEN),
            None,
        )
        .await?;
    assert_eq!(read.status, 200, "{}", read.text());
    let recorded = read.json()?;
    let rows: Vec<&Value> = recorded
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("a run read lists its ops: {recorded}"))?
        .iter()
        .filter(|row| row.get("name") == Some(&json!("page.route")))
        .collect();
    let [row] = rows.as_slice() else {
        return Err(format!("one row for the one try: {recorded}").into());
    };
    let fields = |names: &[&str]| -> Value {
        names
            .iter()
            .map(|name| {
                (
                    (*name).to_owned(),
                    row.get(*name).cloned().unwrap_or(Value::Null),
                )
            })
            .collect()
    };
    assert_eq!(
        fields(&[
            "id", "kind", "effect", "args", "outcome", "delivery", "error"
        ]),
        json!({
            "id": operation, "kind": "builtin", "effect": "write",
            "args": { "$neo": "refused", "code": "method_unavailable", "reason": ROUTE_REASON },
            "outcome": "error", "delivery": "not_sent",
            "error": {
                "owner": "script", "code": "method_unavailable", "message": ROUTE_REASON,
                "retryable": true, "operationId": operation,
            },
        }),
        "the refusal's stored row: {row}"
    );
    Ok(())
}
