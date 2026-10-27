use super::{OTHER_DOCUMENT, PageJs};
use crate::{
    CoreError, FrameId, ProtocolSession, SessionId,
    test_support::{TestCall, TestConnection, cdp_error},
};
use serde_json::{Value, json};
use std::sync::Arc;

// `PageJs::isolated` reads a process-wide switch that other tests in this binary must not see
// flip, so these tests build the isolated variant directly.
fn isolated(session: &ProtocolSession) -> PageJs<'_> {
    PageJs::new(session, true)
}

/// Each test names its own session: top frames are cached per session id for the process.
fn session(connection: &Arc<TestConnection>, id: &str) -> ProtocolSession {
    ProtocolSession::for_session(connection.clone(), SessionId::from(id))
}

fn methods(calls: &[TestCall]) -> Vec<&str> {
    calls.iter().map(|call| call.method.as_str()).collect()
}

fn top_frame(id: &str) -> Value {
    json!({ "frameTree": { "frame": { "id": id } } })
}

#[tokio::test]
async fn tests_that_page_world_eval_sends_a_single_main_world_evaluate()
-> Result<(), Box<dyn std::error::Error>> {
    let connection = TestConnection::new([("Runtime.evaluate", json!({ "result": { "value": 42 } }))]);
    let session = session(&connection, "page-world-eval");

    let answer: i64 = PageJs::page_world(&session).eval("6 * 7").await?;

    assert_eq!(answer, 42);
    assert_eq!(
        connection.calls()?,
        vec![TestCall {
            method: "Runtime.evaluate".into(),
            params: json!({ "expression": "6 * 7", "awaitPromise": true, "returnByValue": true }),
        }]
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_isolated_eval_runs_in_the_browseros_world_of_the_top_frame()
-> Result<(), Box<dyn std::error::Error>> {
    let connection = TestConnection::new([
        ("Page.getFrameTree", top_frame("main")),
        ("Page.createIsolatedWorld", json!({ "executionContextId": 7 })),
        ("Runtime.evaluate", json!({ "result": { "value": "first" } })),
        ("Page.createIsolatedWorld", json!({ "executionContextId": 7 })),
        ("Runtime.evaluate", json!({ "result": { "value": "second" } })),
    ]);
    let session = session(&connection, "isolated-eval");

    let first: String = isolated(&session).eval("probe()").await?;
    let second: String = isolated(&session).eval("probe()").await?;

    assert_eq!((first.as_str(), second.as_str()), ("first", "second"));
    let calls = connection.calls()?;
    // The top frame is looked up once for the session; the world is asked for on every call so a
    // navigation between calls cannot leave a stale context behind.
    assert_eq!(
        methods(&calls),
        [
            "Page.getFrameTree",
            "Page.createIsolatedWorld",
            "Runtime.evaluate",
            "Page.createIsolatedWorld",
            "Runtime.evaluate",
        ]
    );
    assert_eq!(
        calls[1].params,
        json!({ "frameId": "main", "worldName": "browseros" })
    );
    assert_eq!(calls[2].params["contextId"], 7);
    Ok(())
}

#[tokio::test]
async fn tests_that_isolated_eval_in_a_named_frame_uses_that_frames_world()
-> Result<(), Box<dyn std::error::Error>> {
    let connection = TestConnection::new([
        ("Page.createIsolatedWorld", json!({ "executionContextId": 11 })),
        ("Runtime.evaluate", json!({ "result": { "value": true } })),
    ]);
    let session = session(&connection, "isolated-named-frame");
    let child = FrameId("child".into());

    let ready: bool = isolated(&session).in_frame(&child).eval("ready()").await?;

    assert!(ready);
    let calls = connection.calls()?;
    assert_eq!(methods(&calls), ["Page.createIsolatedWorld", "Runtime.evaluate"]);
    assert_eq!(calls[0].params["frameId"], "child");
    assert_eq!(calls[1].params["contextId"], 11);
    Ok(())
}

#[tokio::test]
async fn tests_that_isolated_eval_falls_back_to_the_page_world_when_chromium_refuses_the_world()
-> Result<(), Box<dyn std::error::Error>> {
    let connection = TestConnection::new([
        ("Page.getFrameTree", top_frame("main")),
        (
            "Page.createIsolatedWorld",
            cdp_error("No frame for given id found"),
        ),
        ("Runtime.evaluate", json!({ "result": { "value": 3 } })),
    ]);
    let session = session(&connection, "isolated-refused");

    let value: i64 = isolated(&session).eval("count()").await?;

    assert_eq!(value, 3);
    let calls = connection.calls()?;
    assert_eq!(calls[2].method, "Runtime.evaluate");
    assert!(calls[2].params.get("contextId").is_none());
    Ok(())
}

#[tokio::test]
async fn tests_that_isolated_eval_retries_once_in_a_fresh_world_when_the_context_went_stale()
-> Result<(), Box<dyn std::error::Error>> {
    let connection = TestConnection::new([
        ("Page.getFrameTree", top_frame("main")),
        ("Page.createIsolatedWorld", json!({ "executionContextId": 7 })),
        (
            "Runtime.evaluate",
            cdp_error("Cannot find context with specified id"),
        ),
        ("Page.createIsolatedWorld", json!({ "executionContextId": 8 })),
        ("Runtime.evaluate", json!({ "result": { "value": "fresh" } })),
    ]);
    let session = session(&connection, "isolated-stale");

    let value: String = isolated(&session).eval("probe()").await?;

    assert_eq!(value, "fresh");
    let calls = connection.calls()?;
    assert_eq!(calls[4].params["contextId"], 8);
    Ok(())
}

#[tokio::test]
async fn tests_that_a_thrown_exception_surfaces_as_a_page_script_error()
-> Result<(), Box<dyn std::error::Error>> {
    let connection = TestConnection::new([(
        "Runtime.evaluate",
        json!({
            "result": { "type": "object" },
            "exceptionDetails": { "text": "Uncaught", "exception": { "description": "Error: boom" } }
        }),
    )]);
    let session = session(&connection, "page-script-error");

    let result = PageJs::page_world(&session).eval::<Value>("boom()").await;

    assert!(matches!(result, Err(CoreError::PageScript(message)) if message == "Error: boom"));
    Ok(())
}

#[tokio::test]
async fn tests_that_isolated_call_runs_on_the_nodes_isolated_wrapper_and_releases_it()
-> Result<(), Box<dyn std::error::Error>> {
    let connection = TestConnection::new([
        ("Page.getFrameTree", top_frame("main")),
        ("Page.createIsolatedWorld", json!({ "executionContextId": 7 })),
        ("DOM.resolveNode", json!({ "object": { "objectId": "isolated-node" } })),
        ("Runtime.callFunctionOn", json!({ "result": { "value": "INPUT" } })),
        ("Runtime.releaseObject", json!({})),
    ]);
    let session = session(&connection, "isolated-call");

    let tag: String = isolated(&session)
        .call(42, "function(){return this.tagName}", &[])
        .await?;

    assert_eq!(tag, "INPUT");
    let calls = connection.calls()?;
    assert_eq!(
        calls[2].params,
        json!({ "backendNodeId": 42, "executionContextId": 7 })
    );
    let declaration = calls[3].params["functionDeclaration"]
        .as_str()
        .unwrap_or_default();
    assert!(declaration.contains("function(){return this.tagName}"));
    assert!(declaration.contains(OTHER_DOCUMENT));
    assert_eq!(calls[3].params["objectId"], "isolated-node");
    assert!(calls[3].params.get("arguments").is_none());
    assert_eq!(calls[4].params, json!({ "objectId": "isolated-node" }));
    Ok(())
}

#[tokio::test]
async fn tests_that_isolated_call_reruns_in_the_page_world_for_a_node_in_another_document()
-> Result<(), Box<dyn std::error::Error>> {
    let connection = TestConnection::new([
        ("Page.getFrameTree", top_frame("main")),
        ("Page.createIsolatedWorld", json!({ "executionContextId": 7 })),
        ("DOM.resolveNode", json!({ "object": { "objectId": "isolated-node" } })),
        (
            "Runtime.callFunctionOn",
            json!({ "result": { "value": { OTHER_DOCUMENT: true } } }),
        ),
        ("Runtime.releaseObject", json!({})),
        ("DOM.resolveNode", json!({ "object": { "objectId": "page-node" } })),
        ("Runtime.callFunctionOn", json!({ "result": { "value": 12.5 } })),
        ("Runtime.releaseObject", json!({})),
    ]);
    let session = session(&connection, "isolated-other-document");

    let width: f64 = isolated(&session)
        .call(42, "function(){return this.offsetWidth}", &[])
        .await?;

    assert!((width - 12.5).abs() < f64::EPSILON);
    let calls = connection.calls()?;
    assert_eq!(calls[5].params, json!({ "backendNodeId": 42 }));
    assert_eq!(
        calls[6].params["functionDeclaration"],
        "function(){return this.offsetWidth}"
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_page_world_call_passes_arguments_and_reports_a_missing_node()
-> Result<(), Box<dyn std::error::Error>> {
    let connection = TestConnection::new([
        ("DOM.resolveNode", json!({ "object": { "objectId": "node" } })),
        ("Runtime.callFunctionOn", json!({ "result": { "value": "picked" } })),
        ("Runtime.releaseObject", json!({})),
        ("DOM.resolveNode", json!({ "object": { "type": "object", "subtype": "null" } })),
    ]);
    let session = session(&connection, "page-world-call");

    let picked: String = PageJs::page_world(&session)
        .call(5, "function(v){return v}", &[json!("picked")])
        .await?;
    let missing = PageJs::page_world(&session)
        .call::<Value>(6, "function(){}", &[])
        .await;

    assert_eq!(picked, "picked");
    assert_eq!(
        connection.calls()?[1].params["arguments"],
        json!([{ "value": "picked" }])
    );
    assert!(matches!(missing, Err(CoreError::Message(message)) if message.contains("Take a new snapshot")));
    Ok(())
}

#[tokio::test]
async fn tests_that_page_world_eval_in_a_child_frame_runs_on_that_frames_document()
-> Result<(), Box<dyn std::error::Error>> {
    let connection = TestConnection::new([
        ("Page.getFrameTree", top_frame("main")),
        ("DOM.getFrameOwner", json!({ "backendNodeId": 5 })),
        (
            "DOM.describeNode",
            json!({ "node": { "contentDocument": { "backendNodeId": 9 } } }),
        ),
        ("DOM.resolveNode", json!({ "object": { "objectId": "child-document" } })),
        ("Runtime.callFunctionOn", json!({ "result": { "value": "child" } })),
        ("Runtime.releaseObject", json!({})),
    ]);
    let session = session(&connection, "page-world-child-frame");
    let child = FrameId("child".into());

    let name: String = PageJs::page_world(&session)
        .in_frame(&child)
        .eval("window.name")
        .await?;

    assert_eq!(name, "child");
    let calls = connection.calls()?;
    assert_eq!(calls[1].params, json!({ "frameId": "child" }));
    assert_eq!(calls[3].params, json!({ "backendNodeId": 9 }));
    let declaration = calls[4].params["functionDeclaration"]
        .as_str()
        .unwrap_or_default();
    assert!(declaration.contains("window.name"));
    Ok(())
}

#[tokio::test]
async fn tests_that_user_gesture_and_timeout_reach_the_evaluate()
-> Result<(), Box<dyn std::error::Error>> {
    let connection = TestConnection::new([("Runtime.evaluate", json!({ "result": {} }))]);
    let session = session(&connection, "eval-options");

    let value: Value = PageJs::page_world(&session)
        .user_gesture()
        .timeout(std::time::Duration::from_millis(2_000))
        .eval("open()")
        .await?;

    assert_eq!(value, Value::Null);
    let params = &connection.calls()?[0].params;
    assert_eq!(params["userGesture"], true);
    assert_eq!(params["timeout"], 2_000);
    Ok(())
}
