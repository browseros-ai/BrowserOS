//! A browser that is never there: a `BrowserSession` and `BrowserView` over a CDP connection
//! that refuses every call, for tests that build the crate's objects without a browser.
//! Building one needs a tokio runtime, because the session starts its listener task.

use std::sync::Arc;

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, SessionId};
use futures_util::future::BoxFuture;
use serde_json::Value;
use tokio::sync::broadcast;

/// A CDP connection that refuses every call and never delivers an event.
struct Offline;

impl CdpConnection for Offline {
    fn send<'a>(
        &'a self,
        _method: &'a str,
        _params: Value,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async { Err(CdpError::NotConnected) })
    }

    fn send_raw_json<'a>(
        &'a self,
        _method: &'a str,
        _params_json: &'a str,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async { Err(CdpError::NotConnected) })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        // The sender drops at once, so every listener sees the stream closed and ends.
        broadcast::channel(1).1
    }

    fn is_connected(&self) -> bool {
        false
    }

    fn connection_epoch(&self) -> u64 {
        0
    }
}

/// A browser session over a connection that is never there.
pub(crate) fn offline_session() -> Arc<BrowserSession> {
    BrowserSession::new(Arc::new(Offline), BrowserSessionHooks::default())
}

/// An unbound view of [`offline_session`].
pub(crate) fn offline_view() -> BrowserView {
    BrowserView::unbound(offline_session(), None)
}
