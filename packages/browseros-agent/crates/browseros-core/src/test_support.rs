use crate::{CdpConnection, SessionId};
use browseros_cdp::{CdpError, CdpEvent};
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;

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
        let (events, _receiver) = broadcast::channel(1);
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
}

impl CdpConnection for TestConnection {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            self.calls
                .lock()
                .map_err(|_error| test_state_error())?
                .push(TestCall {
                    method: method.to_string(),
                    params,
                });
            self.responses
                .lock()
                .map_err(|_error| test_state_error())?
                .get_mut(method)
                .and_then(VecDeque::pop_front)
                .ok_or_else(|| CdpError::Protocol {
                    code: -1,
                    message: format!("unexpected test CDP call: {method}"),
                })
        })
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
