use crate::{CdpConnection, SessionId};
use browseros_cdp::{CdpError, CdpEvent};
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;

const TEST_ERROR_KEY: &str = "__test_cdp_error";
const TEST_PENDING_KEY: &str = "__test_cdp_pending";

/// A queued response that makes the call fail with a CDP protocol error carrying `message`.
pub fn cdp_error(message: &str) -> Value {
    serde_json::json!({ TEST_ERROR_KEY: message })
}

/// A queued response the browser never sends: the call stays pending until its future is dropped.
pub fn cdp_pending() -> Value {
    serde_json::json!({ TEST_PENDING_KEY: true })
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

#[derive(Clone, Debug, PartialEq)]
pub struct TestCall {
    pub method: String,
    pub params: Value,
}

pub struct TestConnection {
    calls: Mutex<Vec<TestCall>>,
    responses: Mutex<HashMap<String, VecDeque<Value>>>,
    events: broadcast::Sender<CdpEvent>,
}

impl TestConnection {
    pub fn new(responses: impl IntoIterator<Item = (&'static str, Value)>) -> Arc<Self> {
        let mut queued = HashMap::<String, VecDeque<Value>>::new();
        for (method, response) in responses {
            queued
                .entry(method.to_string())
                .or_default()
                .push_back(response);
        }
        let (events, _receiver) = broadcast::channel(4);
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            responses: Mutex::new(queued),
            events,
        })
    }

    pub fn calls(&self) -> Result<Vec<TestCall>, CdpError> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .map_err(|_error| test_state_error())
    }

    pub fn publish_event(&self, event: CdpEvent) {
        let _ = self.events.send(event);
    }

    fn reply(&self, expected: Option<u64>, method: &str, params: Value) -> Result<Value, CdpError> {
        let mut calls = self.calls.lock().map_err(|_| test_state_error())?;
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
        calls.push(TestCall {
            method: method.to_string(),
            params,
        });
        let response = self
            .responses
            .lock()
            .map_err(|_| test_state_error())?
            .get_mut(method)
            .and_then(VecDeque::pop_front)
            .ok_or_else(|| CdpError::Protocol {
                code: -1,
                message: format!("unexpected test CDP call: {method}"),
            })?;
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
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { answer(self.reply(None, method, params)).await })
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
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { answer(self.reply(Some(expected), method, params)).await })
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected: u64,
        method: &'a str,
        params_json: &'a str,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let value = self.reply(Some(expected), method, serde_json::from_str(params_json)?)?;
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
