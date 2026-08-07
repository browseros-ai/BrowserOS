use crate::{CoreError, SessionId};
use browseros_cdp::{CdpClient, CdpError, CdpEvent};
use futures_util::future::BoxFuture;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{fmt, sync::Arc};
use tokio::sync::{broadcast, mpsc};

pub const EXCLUDED_URL_PREFIXES: &[&str] = &[
    "chrome-extension://",
    "chrome-untrusted://",
    "chrome-search://",
    "devtools://",
];

pub trait CdpConnection: Send + Sync {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>>;

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>>;

    fn events(&self) -> broadcast::Receiver<CdpEvent>;

    /// Dedicated single-method event channel that bypasses the broadcast
    /// ring (see `CdpClient::events_targeted`). Default: a closed channel —
    /// transports without targeted delivery yield no events here.
    fn events_targeted(&self, _method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        let (_tx, rx) = mpsc::unbounded_channel();
        rx
    }

    fn is_connected(&self) -> bool;
    fn connection_epoch(&self) -> u64;
}

impl CdpConnection for CdpClient {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { CdpClient::send(self, method, params, session).await })
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move { CdpClient::send_raw_json(self, method, params_json, session).await })
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

#[derive(Clone)]
pub struct ProtocolSession {
    connection: Arc<dyn CdpConnection>,
    session_id: Option<SessionId>,
}

impl fmt::Debug for ProtocolSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProtocolSession")
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

impl ProtocolSession {
    #[must_use]
    pub fn root(connection: Arc<dyn CdpConnection>) -> Self {
        Self {
            connection,
            session_id: None,
        }
    }

    #[must_use]
    pub fn for_session(connection: Arc<dyn CdpConnection>, session_id: SessionId) -> Self {
        Self {
            connection,
            session_id: Some(session_id),
        }
    }

    #[must_use]
    pub fn session_id(&self) -> Option<&SessionId> {
        self.session_id.as_ref()
    }

    #[must_use]
    pub fn same_session(&self, other: &Self) -> bool {
        self.session_id == other.session_id
    }

    pub async fn send<P, R>(&self, method: &str, params: P) -> Result<R, CoreError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let value =
            serde_json::to_value(params).map_err(|err| CoreError::Message(err.to_string()))?;
        let response = self
            .connection
            .send(method, value, self.session_id.as_ref())
            .await?;
        serde_json::from_value(response).map_err(|err| CoreError::Message(err.to_string()))
    }

    pub async fn send_value(&self, method: &str, params: Value) -> Result<Value, CoreError> {
        self.connection
            .send(method, params, self.session_id.as_ref())
            .await
            .map_err(CoreError::from)
    }

    pub async fn send_raw_json(
        &self,
        method: &str,
        params_json: &str,
    ) -> Result<String, CoreError> {
        self.connection
            .send_raw_json(method, params_json, self.session_id.as_ref())
            .await
            .map_err(CoreError::from)
    }
}
