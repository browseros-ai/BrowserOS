//! Neo: the small CDP connection contract needed by captures, without the core runtime.

use browseros_cdp::{CdpClient, CdpError, CdpEvent, SessionId};
use futures_util::future::BoxFuture;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

/// A capture-owned socket. Session ids are valid only within its connection epoch.
pub trait CaptureConnection: Send + Sync {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>>;
    fn events(&self) -> broadcast::Receiver<CdpEvent>;
    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent>;
    fn is_connected(&self) -> bool;
    fn connection_epoch(&self) -> u64;
}

impl CaptureConnection for CdpClient {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(CdpClient::send(self, method, params, session))
    }
    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        CdpClient::events(self)
    }
    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        CdpClient::events_targeted(self, method)
    }
    fn is_connected(&self) -> bool {
        CdpClient::is_connected(self)
    }
    fn connection_epoch(&self) -> u64 {
        CdpClient::epoch(self)
    }
}
