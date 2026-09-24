//! Protocol fakes exercise the host contract; the pinned JavaScript itself is covered
//! by the isolated-browser probe, not by reimplementing its DOM semantics in this fake.
use super::*;
use crate::{BrowserSession, BrowserSessionHooks, CdpConnection, SessionId};
use browseros_cdp::{CdpError, CdpEvent};
use futures_util::future::BoxFuture;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::broadcast;

type Reply = Result<Value, CdpError>;
type Calls = Vec<(String, Value, Option<SessionId>)>;
struct Fake {
    calls: Mutex<Calls>,
    replies: Mutex<HashMap<String, VecDeque<Reply>>>,
    events: broadcast::Sender<CdpEvent>,
    epoch: AtomicU64,
    contexts: AtomicU64,
}
impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::default(),
            replies: Mutex::default(),
            events: broadcast::channel(64).0,
            epoch: AtomicU64::new(1),
            contexts: AtomicU64::new(0),
        })
    }
    fn push(&self, method: &str, reply: Reply) -> Result<(), Box<dyn std::error::Error>> {
        self.replies
            .lock()
            .map_err(|_| "poisoned replies")?
            .entry(method.to_owned())
            .or_default()
            .push_back(reply);
        Ok(())
    }
    fn calls(&self) -> Result<Calls, Box<dyn std::error::Error>> {
        Ok(self.calls.lock().map_err(|_| "poisoned calls")?.clone())
    }
    fn event(&self, method: &str, params: Value) -> Result<(), Box<dyn std::error::Error>> {
        self.events.send(CdpEvent {
            method: method.to_owned(),
            params,
            session_id: Some(SessionId::from("page")),
        })?;
        Ok(())
    }
}
fn fake_error() -> CdpError {
    CdpError::Protocol {
        code: -1,
        message: "poisoned fake".to_owned(),
    }
}
impl CdpConnection for Fake {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Reply> {
        Box::pin(async move {
            self.calls.lock().map_err(|_| fake_error())?.push((
                method.to_owned(),
                params.clone(),
                session.cloned(),
            ));
            if let Some(reply) = self
                .replies
                .lock()
                .map_err(|_| fake_error())?
                .get_mut(method)
                .and_then(VecDeque::pop_front)
            {
                return reply;
            }
            Ok(match method {
                "Browser.getTabs" => {
                    json!({"tabs": [{"tabId": 1, "targetId": "target", "url": "https://example.com", "title": "Example", "isActive": false, "isLoading": false, "loadProgress": 1.0, "isPinned": false, "isHidden": false, "windowId": 1}]})
                }
                "Target.attachToTarget" => json!({"sessionId": "page"}),
                "Page.getFrameTree" => json!({"frameTree": {"frame": {"id": "root"}}}),
                "Page.createIsolatedWorld" => {
                    json!({"executionContextId": self.contexts.fetch_add(1, Ordering::SeqCst) + 1})
                }
                "Runtime.evaluate" => {
                    json!({"result": {"objectId": format!("engine-{}", params["contextId"]), "type": "object"}})
                }
                "Runtime.callFunctionOn"
                    if params["functionDeclaration"]
                        .as_str()
                        .is_some_and(|s| s.contains("return this.parseSelector")) =>
                {
                    json!({"result": {"value": {"parts": [{"name": "css", "body": [], "source": "button"}]}}})
                }
                "Page.enable"
                | "DOM.enable"
                | "Runtime.enable"
                | "Accessibility.enable"
                | "Runtime.runIfWaitingForDebugger"
                | "Target.setAutoAttach"
                | "Emulation.setFocusEmulationEnabled"
                | "Runtime.releaseObject" => json!({}),
                _ => {
                    return Err(CdpError::Protocol {
                        code: -1,
                        message: format!("unexpected fake call {method}"),
                    });
                }
            })
        })
    }
    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            Ok(self
                .send(method, serde_json::from_str(params)?, session)
                .await?
                .to_string())
        })
    }
    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn connection_epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }
}
fn dl() -> Deadline {
    Deadline {
        at: Instant::now() + Duration::from_secs(1),
        cancel: CancellationToken::new(),
    }
}
async fn engine(fake: Arc<Fake>) -> Result<Arc<LocatorEngine>, CoreError> {
    let session = BrowserSession::new(fake, BrowserSessionHooks::default());
    session.pages.list().await?;
    Ok(session.locator())
}
fn parsed() -> Value {
    json!({"result": {"value": {"parts": [{"name": "css", "body": [], "source": "button"}]}}})
}
fn value(value: Value) -> Value {
    json!({"result": {"value": value}})
}

#[tokio::test]
async fn bootstrap_resolve_and_cache() -> Result<(), Box<dyn std::error::Error>> {
    let fake = Fake::new();
    let engine = engine(fake.clone()).await?;
    for _ in 0..2 {
        fake.push("Runtime.callFunctionOn", Ok(parsed()))?;
        fake.push(
            "Runtime.callFunctionOn",
            Ok(json!({"result": {"objectId": "element"}})),
        )?;
        fake.push(
            "DOM.describeNode",
            Ok(json!({"node": {"backendNodeId": 42}})),
        )?;
        let node = engine
            .resolve(
                PageId(1),
                "button",
                Strictness::Strict,
                Some(ElementState::Stable),
                &dl(),
            )
            .await?;
        assert_eq!(node.backend_node_id, 42);
        assert_eq!(node.frame_id, Some(FrameId("root".to_owned())));
    }
    let calls = fake.calls()?;
    let bootstrap = calls
        .iter()
        .position(|c| c.0 == "Page.createIsolatedWorld")
        .ok_or("missing createWorld")?;
    assert_eq!(calls[bootstrap].1["grantUniveralAccess"], false);
    assert_eq!(calls[bootstrap + 1].0, "Runtime.evaluate");
    let expression = calls[bootstrap + 1].1["expression"]
        .as_str()
        .ok_or("missing expression")?;
    assert!(expression.contains("const module = {}"));
    assert!(expression.contains("new (module.exports.InjectedScript())(globalThis"));
    assert_eq!(calls[bootstrap + 2].0, "Runtime.callFunctionOn");
    assert_eq!(
        calls
            .iter()
            .filter(|c| c.0 == "Page.createIsolatedWorld")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| c.0 == "Emulation.setFocusEmulationEnabled")
            .count(),
        1
    );
    assert!(
        calls
            .iter()
            .any(|c| c.1["arguments"][2]["value"] == "stable")
    );
    Ok(())
}

#[tokio::test]
async fn strict_error_preserves_playwright_count_and_call_log()
-> Result<(), Box<dyn std::error::Error>> {
    let fake = Fake::new();
    let engine = engine(fake.clone()).await?;
    fake.push("Runtime.callFunctionOn", Ok(parsed()))?;
    fake.push("Runtime.callFunctionOn", Ok(json!({"exceptionDetails": {"exception": {"description": "Error: strict mode violation: button resolved to 2 elements"}}})))?;
    let result = engine
        .resolve(PageId(1), "button", Strictness::Strict, None, &dl())
        .await;
    let Err(error) = result else {
        return Err("strict resolve unexpectedly succeeded".into());
    };
    assert!(error.to_string().contains("resolved to 2 elements"));
    assert!(error.to_string().contains("waiting for button"));
    Ok(())
}

#[tokio::test]
async fn context_events_and_epoch_recreate_world_without_repeating_focus()
-> Result<(), Box<dyn std::error::Error>> {
    let fake = Fake::new();
    let worlds = Worlds::default();
    let session = ProtocolSession::for_session(fake.clone(), SessionId::from("page"));
    let frame = FrameId("root".to_owned());
    assert_eq!(worlds.get(&session, &frame).await?.context, 1);
    fake.event("Runtime.executionContextsCleared", json!({}))?;
    fake.event("Runtime.executionContextsCleared", json!({}))?;
    assert_eq!(worlds.get(&session, &frame).await?.context, 2);
    fake.event(
        "Runtime.executionContextDestroyed",
        json!({"executionContextId": 2}),
    )?;
    assert_eq!(worlds.get(&session, &frame).await?.context, 3);
    assert_eq!(
        fake.calls()?
            .iter()
            .filter(|c| c.0 == "Emulation.setFocusEmulationEnabled")
            .count(),
        1
    );
    fake.epoch.store(2, Ordering::SeqCst);
    assert_eq!(worlds.get(&session, &frame).await?.context, 4);
    assert_eq!(
        fake.calls()?
            .iter()
            .filter(|c| c.0 == "Emulation.setFocusEmulationEnabled")
            .count(),
        2
    );
    Ok(())
}

#[tokio::test]
async fn stale_context_retries_once() -> Result<(), Box<dyn std::error::Error>> {
    let fake = Fake::new();
    let worlds = Worlds::default();
    let session = ProtocolSession::for_session(fake.clone(), SessionId::from("page"));
    let frame = FrameId("root".to_owned());
    for _ in 0..2 {
        fake.push(
            "Runtime.callFunctionOn",
            Err(CdpError::Protocol {
                code: -32000,
                message: "Cannot find context with specified id".to_owned(),
            }),
        )?;
    }
    let result = worlds
        .call(&session, &frame, "function(){}", vec![], true)
        .await;
    assert!(result.is_err());
    assert_eq!(fake.contexts.load(Ordering::SeqCst), 2);
    fake.push(
        "Runtime.callFunctionOn",
        Err(CdpError::Protocol {
            code: -32000,
            message: "Cannot find context with specified id".to_owned(),
        }),
    )?;
    fake.push("Runtime.callFunctionOn", Ok(value(json!(7))))?;
    assert_eq!(
        worlds
            .call(&session, &frame, "function(){}", vec![], true)
            .await?["value"],
        7
    );
    assert_eq!(fake.contexts.load(Ordering::SeqCst), 3);
    Ok(())
}

#[tokio::test]
async fn frame_chain_preserves_quoted_body_and_routes_child()
-> Result<(), Box<dyn std::error::Error>> {
    let fake = Fake::new();
    let engine = engine(fake.clone()).await?;
    fake.push(
        "Runtime.callFunctionOn",
        Ok(value(json!({"parts": [
            {"name": "css", "body": [], "source": "iframe"},
            {"name": "internal:control", "body": "enter-frame", "source": "enter-frame"},
            {"name": "internal:text", "body": "\"x >> y\"i", "source": "\"x >> y\"i"}
        ]}))),
    )?;
    fake.push(
        "Runtime.callFunctionOn",
        Ok(json!({"result": {"objectId": "iframe"}})),
    )?;
    fake.push(
        "DOM.describeNode",
        Ok(json!({"node": {"frameId": "child"}})),
    )?;
    fake.push("Runtime.callFunctionOn", Ok(value(json!(3))))?;
    assert_eq!(
        engine
            .count(
                PageId(1),
                "iframe >> internal:control=enter-frame >> internal:text=\"x >> y\"i"
            )
            .await?,
        3
    );
    let calls = fake.calls()?;
    assert!(
        calls
            .iter()
            .any(|c| c.0 == "Page.createIsolatedWorld" && c.1["frameId"] == "child")
    );
    assert!(
        calls
            .iter()
            .any(|c| c.1["arguments"][0]["value"]["parts"][0]["body"] == "\"x >> y\"i")
    );
    Ok(())
}

#[tokio::test]
async fn main_world_adoption_and_describe() -> Result<(), Box<dyn std::error::Error>> {
    let fake = Fake::new();
    let engine = engine(fake.clone()).await?;
    let r = Resolved {
        session: ProtocolSession::for_session(fake.clone(), SessionId::from("page")),
        backend_node_id: 42,
        object_id: "isolated-element".to_owned(),
        frame_id: Some(FrameId("child".to_owned())),
    };
    fake.push(
        "DOM.resolveNode",
        Ok(json!({"object": {"objectId": "main-element"}})),
    )?;
    fake.push("Runtime.callFunctionOn", Ok(value(json!("page global"))))?;
    assert_eq!(
        engine
            .call_on(&r, "(el, arg) => window.marker", Value::Null)
            .await?,
        "page global"
    );
    let calls = fake
        .calls()?
        .into_iter()
        .filter(|c| c.0 != "Browser.getTabs")
        .collect::<Vec<_>>();
    assert_eq!(calls[1].1["objectId"], "main-element");
    assert!(calls[0].1.get("executionContextId").is_none());
    assert_eq!(calls[2].0, "Runtime.releaseObject");
    fake.push("DOM.describeNode", Ok(json!({"node": {"nodeName": "INPUT", "attributes": ["type", "password", "autocomplete", "current-password"]}})))?;
    assert_eq!(
        engine.describe(&r).await?,
        json!({"tag": "input", "type": "password", "autocomplete": "current-password"})
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn expect_retains_last_received_and_logs_on_timeout() -> Result<(), Box<dyn std::error::Error>>
{
    let fake = Fake::new();
    let engine = engine(fake.clone()).await?;
    fake.push("Runtime.callFunctionOn", Ok(parsed()))?;
    fake.push("Runtime.callFunctionOn", Ok(value(json!({"matches": false, "received": "hidden", "log": "  locator resolved to <button hidden>"}))))?;
    let deadline = Deadline {
        at: Instant::now() + Duration::from_millis(15),
        cancel: CancellationToken::new(),
    };
    let result = engine
        .expect(
            PageId(1),
            Some("button"),
            "to.be.visible",
            json!({}),
            &deadline,
        )
        .await?;
    assert!(result.timed_out);
    assert!(!result.matches);
    assert_eq!(result.received, Some(json!("hidden")));
    assert!(
        result
            .log
            .iter()
            .any(|l| l == "  locator resolved to <button hidden>")
    );
    Ok(())
}

#[tokio::test]
async fn cancellation_stops_polling() -> Result<(), Box<dyn std::error::Error>> {
    let fake = Fake::new();
    let engine = engine(fake.clone()).await?;
    let deadline = dl();
    deadline.cancel.cancel();
    let result = engine
        .resolve(PageId(1), "button", Strictness::Strict, None, &deadline)
        .await;
    let Err(error) = result else {
        return Err("cancelled resolve succeeded".into());
    };
    assert!(error.to_string().contains("cancelled"));
    assert!(!fake.calls()?.iter().any(|c| c.0 == "Runtime.evaluate"));
    Ok(())
}

#[test]
fn absent_negation_and_arrays() {
    for (expression, is_not, expected, matches) in [
        ("to.be.hidden", false, 0, true),
        ("to.be.visible", true, 0, false),
        ("to.have.text", true, 0, true),
        ("to.have.count", false, 0, true),
        ("to.have.count", false, 1, false),
    ] {
        assert_eq!(
            missing_expect(
                &json!({"expression": expression, "isNot": is_not, "expectedNumber": expected})
            )["matches"],
            matches
        );
    }
}

#[tokio::test]
async fn destroyed_context_only_invalidates_its_frame_and_lag_invalidates_all()
-> Result<(), Box<dyn std::error::Error>> {
    let fake = Fake::new();
    let worlds = Worlds::default();
    let session = ProtocolSession::for_session(fake.clone(), SessionId::from("page"));
    let root = FrameId("root".to_owned());
    let child = FrameId("child".to_owned());
    assert_eq!(worlds.get(&session, &root).await?.context, 1);
    assert_eq!(worlds.get(&session, &child).await?.context, 2);
    fake.event(
        "Runtime.executionContextDestroyed",
        json!({"executionContextId": 1}),
    )?;
    assert_eq!(worlds.get(&session, &child).await?.context, 2);
    assert_eq!(worlds.get(&session, &root).await?.context, 3);
    for _ in 0..70 {
        fake.event("Runtime.consoleAPICalled", json!({}))?;
    }
    assert_eq!(worlds.get(&session, &child).await?.context, 4);
    Ok(())
}

#[tokio::test]
async fn wait_states_preserves_missing_state_and_honors_cancellation()
-> Result<(), Box<dyn std::error::Error>> {
    let fake = Fake::new();
    let engine = engine(fake.clone()).await?;
    let node = Resolved {
        session: ProtocolSession::for_session(fake.clone(), SessionId::from("page")),
        backend_node_id: 7,
        object_id: "node".to_owned(),
        frame_id: Some(FrameId("root".to_owned())),
    };
    fake.push(
        "Runtime.callFunctionOn",
        Ok(value(json!({"missingState": "stable"}))),
    )?;
    fake.push("Runtime.callFunctionOn", Ok(value(Value::Null)))?;
    engine
        .wait_for_states(&node, &[ElementState::Stable], &dl())
        .await?;
    let deadline = dl();
    deadline.cancel.cancel();
    assert!(
        engine
            .wait_for_states(&node, &[ElementState::Visible], &deadline)
            .await
            .is_err()
    );
    Ok(())
}
