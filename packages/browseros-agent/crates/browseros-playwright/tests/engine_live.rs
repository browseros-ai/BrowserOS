//! The built page asset against a real browser: evaluated in an isolated world named like the
//! sealed world (`browseros-sdk`), it defines `__neoProbe` there and not in the page's own
//! world, and a world without core's seal refuses to boot, so the probe protocol's call
//! answers `uninstalled` there. The sealed path the crate installs (seal, boot, requests) is
//! covered by its own live tests in `src/world/live_tests.rs`.
//!
//! Needs a BrowserOS (or Chromium) instance with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p browseros-playwright --test engine_live -- --ignored`. The test opens and
//! closes its own tab.

use browseros_cdp::{CdpClient, ConnectOptions, SessionId};
use serde_json::{Value, json};
use std::env;
use std::error::Error;
use std::time::Duration;
use tokio::time;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const PAGE: &str = "data:text/html,<button>Go</button>";

/// The probe protocol's examples; `callFunction` is the declaration Rust sends per request.
const PROTOCOL: &str = include_str!("data/probe-protocol.json");

/// The page asset the crate installs, evaluated here without the seal.
const INJECTED_SCRIPT: &str = include_str!("../assets/injected.js");

const BOOT_UNSEALED: &str = r"(() => {
  try {
    __neoProbe.boot({ frameSeq: 0, epoch: 'engine-live' });
    return 'booted';
  } catch (error) {
    return error.message;
  }
})()";

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_the_probe_lives_only_in_its_world_and_needs_the_seal() -> TestResult {
    let port = env::var("BROWSEROS_CDP_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(9005);
    let client = CdpClient::connect(ConnectOptions::new(port)).await?;
    let target = client
        .send("Target.createTarget", json!({ "url": "about:blank" }), None)
        .await?;
    let target_id = target
        .get("targetId")
        .and_then(Value::as_str)
        .ok_or("no target id")?
        .to_string();
    let result = exercise(&client, &target_id).await;
    // Close the tab whatever happened, then report the test's own failure first.
    let closed = client
        .send("Target.closeTarget", json!({ "targetId": target_id }), None)
        .await;
    result?;
    closed?;
    Ok(())
}

async fn exercise(client: &CdpClient, target_id: &str) -> TestResult {
    let attached = client
        .send(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
        )
        .await?;
    let session = SessionId::from(
        attached
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or("no session id")?,
    );
    let session = Some(&session);
    client.send("Page.enable", json!({}), session).await?;
    client
        .send("Page.navigate", json!({ "url": PAGE }), session)
        .await?;
    wait_for_page(client, session).await?;

    let tree = client.send("Page.getFrameTree", json!({}), session).await?;
    let frame_id = tree
        .pointer("/frameTree/frame/id")
        .and_then(Value::as_str)
        .ok_or("no main frame")?;
    let world = client
        .send(
            "Page.createIsolatedWorld",
            json!({ "frameId": frame_id, "worldName": "browseros-sdk" }),
            session,
        )
        .await?;
    let context_id = world
        .get("executionContextId")
        .and_then(Value::as_i64)
        .ok_or("no execution context")?;

    evaluate(client, session, Some(context_id), INJECTED_SCRIPT).await?;
    let boot = evaluate(client, session, Some(context_id), BOOT_UNSEALED).await?;
    assert_eq!(
        boot,
        json!("refusing to boot: this world has no seal (__clawSeal)")
    );

    let protocol: Value = serde_json::from_str(PROTOCOL)?;
    let call = protocol
        .get("callFunction")
        .and_then(Value::as_str)
        .ok_or("probe-protocol.json has no callFunction")?;
    let reply = client
        .send(
            "Runtime.callFunctionOn",
            json!({
                "functionDeclaration": call,
                "executionContextId": context_id,
                "arguments": [{ "value": { "op": "describe" } }],
                "returnByValue": true,
            }),
            session,
        )
        .await?;
    assert_eq!(
        reply.pointer("/result/value"),
        Some(&json!({ "kind": "uninstalled" })),
        "an unbooted world must send Rust back to install: {reply}"
    );

    let seen_by_page = evaluate(client, session, None, "typeof __neoProbe").await?;
    assert_eq!(seen_by_page, json!("undefined"));
    Ok(())
}

/// Polls the page's own world until the data: document has loaded. Evaluations during the
/// navigation may hit a destroyed context, which only means "not yet".
async fn wait_for_page(client: &CdpClient, session: Option<&SessionId>) -> TestResult {
    let loaded = "document.readyState === 'complete' && location.protocol === 'data:'";
    for _ in 0..100 {
        if matches!(
            evaluate(client, session, None, loaded).await,
            Ok(Value::Bool(true))
        ) {
            return Ok(());
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    Err("the data: page did not load within 5 s".into())
}

/// `Runtime.evaluate` by value; a page exception becomes the test's error.
async fn evaluate(
    client: &CdpClient,
    session: Option<&SessionId>,
    context_id: Option<i64>,
    expression: &str,
) -> TestResult<Value> {
    let mut params = json!({ "expression": expression, "returnByValue": true });
    if let (Some(id), Some(object)) = (context_id, params.as_object_mut()) {
        object.insert("contextId".to_string(), json!(id));
    }
    let response = client.send("Runtime.evaluate", params, session).await?;
    if let Some(details) = response.get("exceptionDetails") {
        return Err(format!("page exception: {details}").into());
    }
    Ok(response
        .pointer("/result/value")
        .cloned()
        .unwrap_or(Value::Null))
}
