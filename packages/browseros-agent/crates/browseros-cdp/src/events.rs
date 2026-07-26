use crate::SessionId;
use serde_json::Value;
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<SessionId>,
}

pub struct EventStream {
    method: String,
    inner: broadcast::Receiver<CdpEvent>,
}

impl EventStream {
    pub(crate) fn new(method: impl Into<String>, inner: broadcast::Receiver<CdpEvent>) -> Self {
        Self {
            method: method.into(),
            inner,
        }
    }

    pub async fn recv(&mut self) -> Result<CdpEvent, broadcast::error::RecvError> {
        loop {
            let event = self.inner.recv().await?;
            if event.method == self.method {
                return Ok(event);
            }
        }
    }
}
