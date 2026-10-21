use crate::{CoreError, SessionId, TargetError};
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

/// Shared transport seam: ordinary browser infrastructure follows the current socket, while
/// captured capabilities require atomic epoch dispatch from the transport implementation.
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

    /// Check the epoch and publish the request under the same transport lock that publishes
    /// a replacement socket. A preflight followed by `send` leaves a reconnect race.
    /// Custom transports must opt in; the default emits no request.
    fn send_at_epoch<'a>(
        &'a self,
        _expected_epoch: u64,
        method: &'a str,
        _params: Value,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { Err(unsupported_epoch_transport(method)) })
    }

    /// Raw JSON has the same atomic publication contract as `send_at_epoch`.
    fn send_raw_json_at_epoch<'a>(
        &'a self,
        _expected_epoch: u64,
        method: &'a str,
        _params_json: &'a str,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move { Err(unsupported_epoch_transport(method)) })
    }

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

    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            CdpClient::send_at_epoch(self, expected_epoch, method, params, session).await
        })
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            CdpClient::send_raw_json_at_epoch(self, expected_epoch, method, params_json, session)
                .await
        })
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

const UNSUPPORTED_EPOCH_TRANSPORT: &str = "atomic epoch dispatch unsupported: ";

fn unsupported_epoch_transport(method: &str) -> CdpError {
    CdpError::Transport(format!("{UNSUPPORTED_EPOCH_TRANSPORT}{method}"))
}

/// Capture carried even by policy-free infrastructure sessions (attachment/domain setup).
/// Admission can await native lookups; only the transport can close the race at publication.
#[derive(Clone, Debug)]
pub(crate) struct EpochGuard {
    pub(crate) expected: u64,
    pub(crate) invalidated: TargetError,
}

impl EpochGuard {
    pub(crate) fn check(&self, connection: &dyn CdpConnection) -> Result<(), CoreError> {
        if !connection.is_connected() || connection.connection_epoch() != self.expected {
            return Err(self.invalidated.clone().into());
        }
        Ok(())
    }

    fn transport_error(&self, method: &str, error: CdpError) -> CoreError {
        match error {
            CdpError::ConnectionEpochChanged { .. }
            | CdpError::NotConnected
            | CdpError::ConnectionLost => self.invalidated.clone().into(),
            CdpError::Transport(message) if message.starts_with(UNSUPPORTED_EPOCH_TRANSPORT) => {
                TargetError::CdpUnsupported {
                    method: method.into(),
                }
                .into()
            }
            error => error.into(),
        }
    }
}

#[derive(Clone)]
pub struct ProtocolSession {
    connection: Arc<dyn CdpConnection>,
    session_id: Option<SessionId>,
    admission: Option<Arc<dyn ProtocolAdmission>>,
    epoch_guard: Option<EpochGuard>,
}

/// Private capability carried by sessions returned from a bound view. It brackets every send,
/// so caching a handle or resolving an OOPIF cannot bypass the capture after an async handoff.
pub(crate) trait ProtocolAdmission: Send + Sync {
    fn check<'a>(
        &'a self,
        method: &'a str,
        params: &'a Value,
    ) -> BoxFuture<'a, Result<(), CoreError>>;
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
            admission: None,
            epoch_guard: None,
        }
    }

    #[must_use]
    pub fn for_session(connection: Arc<dyn CdpConnection>, session_id: SessionId) -> Self {
        Self {
            connection,
            session_id: Some(session_id),
            admission: None,
            epoch_guard: None,
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

    /// Cleanup must reach the original session even after page-scope admission
    /// has been revoked (for example a moved tab). Retain the socket epoch and
    /// session identity; callers may only use this for idempotent cancellation.
    pub(crate) fn cancellation_endpoint(&self) -> Self {
        let mut endpoint = self.clone();
        endpoint.admission = None;
        if endpoint.epoch_guard.is_none() {
            endpoint.epoch_guard = Some(EpochGuard {
                expected: self.connection.connection_epoch(),
                invalidated: crate::TargetError::CdpUnsupported {
                    method: "credential cancellation".into(),
                },
            });
        }
        endpoint
    }

    pub(crate) fn with_admission(mut self, admission: Arc<dyn ProtocolAdmission>) -> Self {
        self.admission = Some(admission);
        self
    }

    pub(crate) fn with_epoch_guard(mut self, guard: Option<EpochGuard>) -> Self {
        self.epoch_guard = guard;
        self
    }

    pub(crate) fn epoch_guard(&self) -> Option<&EpochGuard> {
        self.epoch_guard.as_ref()
    }

    pub async fn send<P, R>(&self, method: &str, params: P) -> Result<R, CoreError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let value =
            serde_json::to_value(params).map_err(|err| CoreError::Message(err.to_string()))?;
        let response = self.send_value(method, value).await?;
        serde_json::from_value(response).map_err(|err| CoreError::Message(err.to_string()))
    }

    pub async fn send_value(&self, method: &str, params: Value) -> Result<Value, CoreError> {
        if let Some(admission) = &self.admission {
            admission.check(method, &params).await?;
        }
        let response = if let Some(guard) = &self.epoch_guard {
            guard.check(self.connection.as_ref())?;
            let response = self
                .connection
                .send_at_epoch(
                    guard.expected,
                    method,
                    params.clone(),
                    self.session_id.as_ref(),
                )
                .await
                .map_err(|error| guard.transport_error(method, error))?;
            guard.check(self.connection.as_ref())?;
            response
        } else {
            self.connection
                .send(method, params.clone(), self.session_id.as_ref())
                .await?
        };
        if let Some(admission) = &self.admission {
            admission.check(method, &params).await?;
        }
        Ok(response)
    }

    pub async fn send_raw_json(
        &self,
        method: &str,
        params_json: &str,
    ) -> Result<String, CoreError> {
        let params = if let Some(admission) = &self.admission {
            let params = serde_json::from_str(params_json)
                .map_err(|err| CoreError::Message(err.to_string()))?;
            admission.check(method, &params).await?;
            Some(params)
        } else {
            None
        };
        let response = if let Some(guard) = &self.epoch_guard {
            guard.check(self.connection.as_ref())?;
            let response = self
                .connection
                .send_raw_json_at_epoch(
                    guard.expected,
                    method,
                    params_json,
                    self.session_id.as_ref(),
                )
                .await
                .map_err(|error| guard.transport_error(method, error))?;
            guard.check(self.connection.as_ref())?;
            response
        } else {
            self.connection
                .send_raw_json(method, params_json, self.session_id.as_ref())
                .await?
        };
        if let Some(admission) = &self.admission
            && let Some(params) = params
        {
            admission.check(method, &params).await?;
        }
        Ok(response)
    }
}
