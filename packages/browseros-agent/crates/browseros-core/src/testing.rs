//! A scripted [`CdpConnection`] for tests: core's own, and those of the crates built on core
//! that drive a browser through a `BrowserSession`. Hidden from core's documented API; nothing
//! in production builds one.
//!
//! The script is a reply per method. Queued replies answer one call each, in order; a standing
//! reply ([`TestConnection::always`]) answers every call after the queue runs dry. A method with
//! neither fails, so a test notices a command it did not script instead of reading `{}` as
//! success. One reply is built in: an unscripted `Target.attachToTarget` gets a session id that
//! no other connection in the process hands out. Core serializes captures with a process-wide
//! lock per session id, so two tests whose fakes both answered `session-1` would wait on each
//! other's captures.

use crate::{CdpConnection, SessionId};
use browseros_cdp::{CdpError, CdpEvent};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::broadcast;

const TEST_ERROR_KEY: &str = "__test_cdp_error";
const TEST_PENDING_KEY: &str = "__test_cdp_pending";

/// Numbers connections, so each one's session ids are its own.
static CONNECTIONS: AtomicU64 = AtomicU64::new(0);

/// A reply that makes the call fail with a CDP protocol error carrying `message`.
pub fn cdp_error(message: &str) -> Value {
    json!({ TEST_ERROR_KEY: message })
}

/// A reply the browser never sends: the call stays pending until its future is dropped.
pub fn cdp_pending() -> Value {
    json!({ TEST_PENDING_KEY: true })
}

async fn answer(response: Result<Value, CdpError>) -> Result<Value, CdpError> {
    if response
        .as_ref()
        .is_ok_and(|response| response.get(TEST_PENDING_KEY).is_some())
    {
        std::future::pending::<()>().await;
    }
    response
}

/// One command as the connection received it.
#[derive(Clone, Debug, PartialEq)]
pub struct TestCall {
    pub method: String,
    pub params: Value,
    /// The session it was sent on; `None` for the browser's own.
    pub session: Option<SessionId>,
}

pub struct TestConnection {
    /// This connection's number, the prefix of every session id it hands out.
    id: u64,
    script: Mutex<Script>,
    events: broadcast::Sender<CdpEvent>,
}

struct Script {
    calls: Vec<TestCall>,
    queued: HashMap<String, VecDeque<Value>>,
    standing: HashMap<String, Value>,
    attached: u64,
}

impl TestConnection {
    /// A connection that answers each `(method, reply)` once, in the order given per method.
    pub fn new(responses: impl IntoIterator<Item = (&'static str, Value)>) -> Arc<Self> {
        let mut queued = HashMap::<String, VecDeque<Value>>::new();
        for (method, response) in responses {
            queued
                .entry(method.to_string())
                .or_default()
                .push_back(response);
        }
        Arc::new(Self {
            id: CONNECTIONS.fetch_add(1, Ordering::Relaxed) + 1,
            script: Mutex::new(Script {
                calls: Vec::new(),
                queued,
                standing: HashMap::new(),
                attached: 0,
            }),
            events: broadcast::channel(64).0,
        })
    }

    /// Answers every later `method` call with `reply` once its queued replies are used, until
    /// the next `always` for the same method replaces it.
    pub fn always(&self, method: &str, reply: Value) -> Result<(), CdpError> {
        self.script
            .lock()
            .map_err(|_| test_state_error())?
            .standing
            .insert(method.to_string(), reply);
        Ok(())
    }

    /// Every command received so far, in order.
    pub fn calls(&self) -> Result<Vec<TestCall>, CdpError> {
        self.script
            .lock()
            .map(|script| script.calls.clone())
            .map_err(|_| test_state_error())
    }

    /// Delivers an event to every receiver of [`CdpConnection::events`], as the browser would.
    pub fn emit(&self, method: &str, params: Value, session: Option<SessionId>) {
        // No receiver yet is not an error: the browser does not wait for listeners either.
        let _ = self.events.send(CdpEvent {
            method: method.to_string(),
            params,
            session_id: session,
        });
    }

    fn reply(
        &self,
        expected: Option<u64>,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        let mut script = self.script.lock().map_err(|_| test_state_error())?;
        // This transport has one immutable epoch and cannot reconnect. Guard validation and
        // request recording still share the publication lock, matching the production seam.
        if let Some(expected) = expected
            && expected != 1
        {
            return Err(CdpError::ConnectionEpochChanged {
                expected,
                actual: 1,
            });
        }
        script.calls.push(TestCall {
            method: method.to_string(),
            params,
            session: session.cloned(),
        });
        let scripted = script
            .queued
            .get_mut(method)
            .and_then(VecDeque::pop_front)
            .or_else(|| script.standing.get(method).cloned());
        let response = match scripted {
            Some(response) => response,
            None if method == "Target.attachToTarget" => {
                script.attached += 1;
                json!({ "sessionId": format!("test-{}-session-{}", self.id, script.attached) })
            }
            None => {
                return Err(CdpError::Protocol {
                    code: -1,
                    message: format!("unexpected test CDP call: {method}"),
                });
            }
        };
        match response.get(TEST_ERROR_KEY).and_then(Value::as_str) {
            Some(message) => Err(CdpError::Protocol {
                code: -32000,
                message: message.to_string(),
            }),
            None => Ok(response),
        }
    }
}

impl CdpConnection for TestConnection {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { answer(self.reply(None, method, params, session)).await })
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let params = serde_json::from_str(params_json)?;
            let value = self.send(method, params, session).await?;
            serde_json::to_string(&value).map_err(CdpError::from)
        })
    }

    fn send_at_epoch<'a>(
        &'a self,
        expected: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { answer(self.reply(Some(expected), method, params, session)).await })
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected: u64,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let params = serde_json::from_str(params_json)?;
            let value = answer(self.reply(Some(expected), method, params, session)).await?;
            serde_json::to_string(&value).map_err(CdpError::from)
        })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn connection_epoch(&self) -> u64 {
        1
    }
}

fn test_state_error() -> CdpError {
    CdpError::Protocol {
        code: -1,
        message: "poisoned test state".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{TestCall, TestConnection, cdp_error, cdp_pending};
    use crate::{CdpConnection, SessionId};
    use browseros_cdp::CdpError;
    use serde_json::{Value, json};
    use std::{collections::HashSet, error::Error, time::Duration};

    type TestResult = Result<(), Box<dyn Error>>;

    fn protocol_message(result: Result<Value, CdpError>) -> Option<String> {
        match result {
            Err(CdpError::Protocol { message, .. }) => Some(message),
            _ => None,
        }
    }

    #[tokio::test]
    async fn queued_replies_answer_once_each_then_the_standing_reply_answers_the_rest() -> TestResult
    {
        let connection =
            TestConnection::new([("Page.reload", json!(1)), ("Page.reload", json!(2))]);
        connection.always("Page.reload", json!(3))?;
        let mut replies = Vec::new();
        for _ in 0..4 {
            replies.push(connection.send("Page.reload", json!({}), None).await?);
        }
        assert_eq!(replies, [json!(1), json!(2), json!(3), json!(3)]);
        connection.always("Page.reload", json!(4))?;
        assert_eq!(
            connection.send("Page.reload", json!({}), None).await?,
            json!(4),
            "a later standing reply replaces the earlier one"
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_unscripted_method_fails_instead_of_answering_empty() -> TestResult {
        let connection = TestConnection::new([]);
        assert_eq!(
            protocol_message(connection.send("DOM.enable", json!({}), None).await).as_deref(),
            Some("unexpected test CDP call: DOM.enable")
        );
        connection.always("DOM.enable", cdp_error("Target closed"))?;
        assert_eq!(
            protocol_message(connection.send("DOM.enable", json!({}), None).await).as_deref(),
            Some("Target closed")
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_pending_reply_never_answers() -> TestResult {
        let connection = TestConnection::new([("Page.navigate", cdp_pending())]);
        let call = connection.send("Page.navigate", json!({}), None);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), call)
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn every_send_is_recorded_with_its_params_and_session() -> TestResult {
        let connection = TestConnection::new([
            ("Browser.getTabs", json!({ "tabs": [] })),
            ("Input.dispatchKeyEvent", json!({})),
        ]);
        let page = SessionId::from("page-7");
        connection.send("Browser.getTabs", json!({}), None).await?;
        let raw = connection
            .send_raw_json(
                "Input.dispatchKeyEvent",
                r#"{"type":"keyUp","key":"0"}"#,
                Some(&page),
            )
            .await?;
        assert_eq!(raw, "{}", "raw JSON gets the scripted reply as JSON text");
        assert_eq!(
            connection.calls()?,
            [
                TestCall {
                    method: "Browser.getTabs".into(),
                    params: json!({}),
                    session: None,
                },
                TestCall {
                    method: "Input.dispatchKeyEvent".into(),
                    params: json!({ "type": "keyUp", "key": "0" }),
                    session: Some(page),
                },
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn emitted_events_reach_every_receiver() -> TestResult {
        let connection = TestConnection::new([]);
        let mut first = connection.events();
        let mut second = connection.events();
        let session = SessionId::from("page-3");
        connection.emit(
            "Page.frameNavigated",
            json!({ "frame": { "id": "main" } }),
            Some(session.clone()),
        );
        for receiver in [&mut first, &mut second] {
            let event = receiver.recv().await?;
            assert_eq!(event.method, "Page.frameNavigated");
            assert_eq!(event.params["frame"]["id"], "main");
            assert_eq!(event.session_id.as_ref(), Some(&session));
        }
        Ok(())
    }

    #[tokio::test]
    async fn attached_sessions_are_unique_across_connections() -> TestResult {
        let mut ids = HashSet::new();
        for connection in [TestConnection::new([]), TestConnection::new([])] {
            for _ in 0..2 {
                let attached = connection
                    .send("Target.attachToTarget", json!({ "targetId": "t" }), None)
                    .await?;
                let id = attached["sessionId"]
                    .as_str()
                    .ok_or("no session id")?
                    .to_string();
                assert!(ids.insert(id.clone()), "{id} was handed out twice");
            }
        }
        let scripted =
            TestConnection::new([("Target.attachToTarget", json!({ "sessionId": "mine" }))]);
        assert_eq!(
            scripted
                .send("Target.attachToTarget", json!({}), None)
                .await?["sessionId"],
            "mine",
            "a scripted attach wins over the built-in one"
        );
        Ok(())
    }
}
