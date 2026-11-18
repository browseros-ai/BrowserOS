use crate::SessionId;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<SessionId>,
}

/// One event method's events, from [`crate::CdpClient::on_event`].
///
/// Every stream reads every event of the connection, so the channel carries each event once,
/// shared: a broadcast receiver clones what it carries, and cloning a `CdpEvent` copies its whole
/// params tree, a megabyte for a large console message. A stream checks the method on the
/// shared event and copies only one it returns; the last reader of an event takes it without
/// a copy.
pub struct EventStream {
    method: String,
    inner: broadcast::Receiver<Arc<CdpEvent>>,
}

impl EventStream {
    pub(crate) fn new(
        method: impl Into<String>,
        inner: broadcast::Receiver<Arc<CdpEvent>>,
    ) -> Self {
        Self {
            method: method.into(),
            inner,
        }
    }

    pub async fn recv(&mut self) -> Result<CdpEvent, broadcast::error::RecvError> {
        loop {
            let event = self.inner.recv().await?;
            if event.method == self.method {
                return Ok(Arc::unwrap_or_clone(event));
            }
        }
    }
}
