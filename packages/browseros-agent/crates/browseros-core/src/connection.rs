use crate::{CoreError, SessionId, TargetError};
use browseros_cdp::{CdpClient, CdpError, CdpEvent, RawReply};
use futures_util::future::BoxFuture;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

    /// [`Self::send`], with the result kept as the browser's text for the caller to decode into
    /// its own type in its own task ([`RawReply::decode`]): a large result is then never built
    /// as a `Value`, and its decode does not hold up the client's reader loop, which every other
    /// reply and event waits for. A transport that does not override this answers through
    /// [`Self::send`] and encodes the `Value` back to text: the same result, without the saving.
    fn send_raw<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<RawReply, CdpError>> {
        Box::pin(async move { RawReply::from_value(&self.send(method, params, session).await?) })
    }

    /// [`Self::send_raw`] under [`Self::send_at_epoch`]'s atomic publication contract. A
    /// transport that does not override this answers through [`Self::send_at_epoch`].
    fn send_raw_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<RawReply, CdpError>> {
        Box::pin(async move {
            RawReply::from_value(
                &self
                    .send_at_epoch(expected_epoch, method, params, session)
                    .await?,
            )
        })
    }

    /// Every event no targeted subscriber took. A broadcast hands each receiver its own clone
    /// of what it carries, so the event rides behind an `Arc`: one allocation, whatever the
    /// receiver count. Check `method` and `session_id` through the `Arc` first, read params by
    /// reference (`T::deserialize(&event.params)`), and take an owned copy with
    /// `Arc::unwrap_or_clone` only of an event the reader keeps.
    fn events(&self) -> broadcast::Receiver<Arc<CdpEvent>>;

    /// Every later event of `method` on a dedicated, unbounded channel that bypasses the
    /// broadcast ring. Each subscriber of a method gets every one of its events, and while any
    /// subscriber is live, [`Self::events`] does not carry the method: a reader that needs a
    /// method someone else takes here must take it here too.
    ///
    /// Required of every transport, test doubles included, because a closed or unfed channel
    /// starves its subscriber with no sign. `browseros_cdp::TargetedEvents` is the shared
    /// router: subscribe through it, and offer each event to it before the broadcast, which
    /// carries only what no subscriber took. A transport that never emits still subscribes
    /// through a router it owns, so the channel stays open for as long as the transport lives.
    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent>;

    /// How many events [`Self::events`] has carried so far. Read after a reply, it counts every
    /// event that arrived before that reply, so a reader that numbers what it took knows when
    /// it has seen them all. A transport that does not count answers 0, and such a reader never
    /// waits.
    fn events_broadcast(&self) -> u64 {
        0
    }

    /// A receiver of [`Self::events`] and the [`Self::events_broadcast`] count of the events it
    /// will never receive, taken together so no event is in both or in neither. A transport
    /// that does not count answers 0.
    fn events_counted(&self) -> (broadcast::Receiver<Arc<CdpEvent>>, u64) {
        (self.events(), 0)
    }

    /// [`Self::events_targeted`], with the count of `method`'s events its subscribers took
    /// before this one, as [`Self::events_targeted_delivered`] numbers them. A transport that
    /// does not count answers 0.
    fn events_targeted_counted(&self, method: &str) -> (mpsc::UnboundedReceiver<CdpEvent>, u64) {
        (self.events_targeted(method), 0)
    }

    /// How many of `method`'s events the targeted subscribers took so far; read after a reply,
    /// it covers every one that arrived before it. A transport that does not count answers 0.
    fn events_targeted_delivered(&self, _method: &str) -> u64 {
        0
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

    fn send_raw<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<RawReply, CdpError>> {
        Box::pin(async move { CdpClient::send_raw(self, method, params, session).await })
    }

    fn send_raw_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<RawReply, CdpError>> {
        Box::pin(async move {
            CdpClient::send_raw_at_epoch(self, expected_epoch, method, params, session).await
        })
    }

    fn events(&self) -> broadcast::Receiver<Arc<CdpEvent>> {
        CdpClient::events(self)
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        CdpClient::events_targeted(self, method)
    }

    fn events_broadcast(&self) -> u64 {
        CdpClient::events_broadcast(self)
    }

    fn events_counted(&self) -> (broadcast::Receiver<Arc<CdpEvent>>, u64) {
        CdpClient::events_counted(self)
    }

    fn events_targeted_counted(&self, method: &str) -> (mpsc::UnboundedReceiver<CdpEvent>, u64) {
        CdpClient::events_targeted_counted(self, method)
    }

    fn events_targeted_delivered(&self, method: &str) -> u64 {
        CdpClient::events_targeted_delivered(self, method)
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
    /// Set once the browser answers any command sent through this handle or its clones. See
    /// [`Self::witnessed`].
    answered: Option<Arc<AtomicBool>>,
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
            answered: None,
        }
    }

    #[must_use]
    pub fn for_session(connection: Arc<dyn CdpConnection>, session_id: SessionId) -> Self {
        Self {
            connection,
            session_id: Some(session_id),
            admission: None,
            epoch_guard: None,
            answered: None,
        }
    }

    #[must_use]
    pub fn session_id(&self) -> Option<&SessionId> {
        self.session_id.as_ref()
    }

    pub(crate) fn subscribe_events(&self) -> broadcast::Receiver<Arc<CdpEvent>> {
        self.connection.events()
    }

    #[must_use]
    pub fn same_session(&self, other: &Self) -> bool {
        self.session_id == other.session_id
    }

    /// Cleanup must reach the original session even after page-scope admission
    /// has been revoked (for example a moved tab). Retain the socket epoch and
    /// session identity; callers may only use this for idempotent cancellation.
    #[must_use]
    pub fn cancellation_endpoint(&self) -> Self {
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

    /// Pin this endpoint to the current websocket incarnation without changing admission.
    /// Background renderer setup must never publish an old session id onto a new connection.
    #[must_use]
    pub fn at_connection_epoch(&self) -> Self {
        let mut endpoint = self.clone();
        if endpoint.epoch_guard.is_none() {
            endpoint.epoch_guard = Some(EpochGuard {
                expected: self.connection.connection_epoch(),
                invalidated: crate::TargetError::CdpUnsupported {
                    method: "renderer initialization".into(),
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

    /// This handle, plus a flag the browser sets by answering any command sent through it. A
    /// retry after a lost session asks the flag whether the session was already dead when the
    /// work began: if nothing was answered, nothing reached the page and the work can run again
    /// on a new attachment. Otherwise part of it may have landed, and running it again could
    /// repeat an effect such as a key press.
    pub(crate) fn witnessed(mut self) -> (Self, Arc<AtomicBool>) {
        let answered = Arc::new(AtomicBool::new(false));
        self.answered = Some(answered.clone());
        (self, answered)
    }

    fn mark_answered(&self) {
        if let Some(answered) = &self.answered {
            answered.store(true, Ordering::SeqCst);
        }
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
        let session = self.session_id.as_ref();
        self.dispatch(method, &params, |epoch| match epoch {
            Some(expected) => {
                self.connection
                    .send_at_epoch(expected, method, params.clone(), session)
            }
            None => self.connection.send(method, params.clone(), session),
        })
        .await
    }

    /// [`Self::send`] for a large result: it is decoded straight from the browser's text into
    /// `R`, here in the caller's task, with no `Value` in between ([`CdpConnection::send_raw`]).
    /// Admission and the epoch are checked exactly as for [`Self::send`].
    pub async fn send_decoded<P, R>(&self, method: &str, params: P) -> Result<R, CoreError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let params =
            serde_json::to_value(params).map_err(|err| CoreError::Message(err.to_string()))?;
        let session = self.session_id.as_ref();
        let reply = self
            .dispatch(method, &params, |epoch| match epoch {
                Some(expected) => {
                    self.connection
                        .send_raw_at_epoch(expected, method, params.clone(), session)
                }
                None => self.connection.send_raw(method, params.clone(), session),
            })
            .await?;
        reply.decode().map_err(|error| match error {
            CdpError::Json(message) => CoreError::Message(message),
            error => error.into(),
        })
    }

    /// Brackets one transport call with this handle's checks: admission before the command and
    /// again after its reply, and, for a handle pinned to a socket incarnation, the epoch at
    /// dispatch (inside the transport, `call` receives it) and again after the reply.
    async fn dispatch<'a, T>(
        &'a self,
        method: &str,
        params: &Value,
        call: impl FnOnce(Option<u64>) -> BoxFuture<'a, Result<T, CdpError>>,
    ) -> Result<T, CoreError> {
        if let Some(admission) = &self.admission {
            admission.check(method, params).await?;
        }
        let response = if let Some(guard) = &self.epoch_guard {
            guard.check(self.connection.as_ref())?;
            let response = call(Some(guard.expected))
                .await
                .map_err(|error| guard.transport_error(method, error))?;
            self.mark_answered();
            guard.check(self.connection.as_ref())?;
            response
        } else {
            let response = call(None).await?;
            self.mark_answered();
            response
        };
        if let Some(admission) = &self.admission {
            admission.check(method, params).await?;
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
            self.mark_answered();
            guard.check(self.connection.as_ref())?;
            response
        } else {
            let response = self
                .connection
                .send_raw_json(method, params_json, self.session_id.as_ref())
                .await?;
            self.mark_answered();
            response
        };
        if let Some(admission) = &self.admission
            && let Some(params) = params
        {
            admission.check(method, &params).await?;
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::{CdpConnection, ProtocolSession};
    use crate::{CoreError, SessionId};
    use browseros_cdp::{CdpError, CdpEvent, RawReply, TargetedEvents};
    use futures_util::future::BoxFuture;
    use serde::Deserialize;
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};
    use tokio::sync::{broadcast, mpsc};

    #[derive(Debug, Deserialize, PartialEq)]
    struct Shape {
        count: u32,
    }

    /// Answers every command with `answer` and records which of its methods each came through.
    /// It leaves the raw sends to the trait's defaults, as a test double or a host's own
    /// transport does.
    struct Plain {
        answer: Value,
        calls: Mutex<Vec<String>>,
        targeted: TargetedEvents,
    }

    impl Plain {
        fn new(answer: Value) -> Self {
            Self {
                answer,
                calls: Mutex::new(Vec::new()),
                targeted: TargetedEvents::default(),
            }
        }

        fn note(&self, call: String) {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push(call);
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls
                .lock()
                .map(|calls| calls.clone())
                .unwrap_or_default()
        }
    }

    /// [`Plain`], answering the raw sends itself, as `CdpClient` does.
    struct Raw(Plain);

    impl CdpConnection for Plain {
        fn send<'a>(
            &'a self,
            _method: &'a str,
            _params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            self.note("send".to_owned());
            Box::pin(async move { Ok(self.answer.clone()) })
        }

        fn send_at_epoch<'a>(
            &'a self,
            expected_epoch: u64,
            _method: &'a str,
            _params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            self.note(format!("send_at_epoch {expected_epoch}"));
            Box::pin(async move { Ok(self.answer.clone()) })
        }

        fn send_raw_json<'a>(
            &'a self,
            method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async move { Err(CdpError::Transport(format!("not sent: {method}"))) })
        }

        fn events(&self) -> broadcast::Receiver<Arc<CdpEvent>> {
            broadcast::channel(1).1
        }

        fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
            self.targeted.subscribe(method)
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            7
        }
    }

    impl CdpConnection for Raw {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            self.0.send(method, params, session)
        }

        fn send_raw<'a>(
            &'a self,
            _method: &'a str,
            _params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<RawReply, CdpError>> {
            self.0.note("send_raw".to_owned());
            Box::pin(async move { RawReply::from_value(&self.0.answer) })
        }

        fn send_raw_at_epoch<'a>(
            &'a self,
            expected_epoch: u64,
            _method: &'a str,
            _params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<RawReply, CdpError>> {
            self.0.note(format!("send_raw_at_epoch {expected_epoch}"));
            Box::pin(async move { RawReply::from_value(&self.0.answer) })
        }

        fn send_raw_json<'a>(
            &'a self,
            method: &'a str,
            params_json: &'a str,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            self.0.send_raw_json(method, params_json, session)
        }

        fn events(&self) -> broadcast::Receiver<Arc<CdpEvent>> {
            self.0.events()
        }

        fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
            self.0.events_targeted(method)
        }

        fn is_connected(&self) -> bool {
            self.0.is_connected()
        }

        fn connection_epoch(&self) -> u64 {
            self.0.connection_epoch()
        }
    }

    async fn both(root: &ProtocolSession) -> Result<(Shape, Shape), CoreError> {
        let plain = root.send_decoded("Test.read", json!({})).await?;
        let pinned = root
            .at_connection_epoch()
            .send_decoded("Test.read", json!({}))
            .await?;
        Ok((plain, pinned))
    }

    /// A decoded send reads the transport's raw reply, a pinned handle's at its epoch, and
    /// decodes it into the caller's type; a reply of another shape fails as `send`'s does.
    #[tokio::test]
    async fn a_decoded_send_reads_the_transports_raw_reply_and_a_pinned_one_at_its_epoch() {
        let transport = Arc::new(Raw(Plain::new(json!({ "count": 3, "other": [1, 2] }))));
        let decoded = both(&ProtocolSession::root(transport.clone()))
            .await
            .unwrap_or_else(|err| panic!("decoded sends: {err}"));
        assert_eq!(decoded, (Shape { count: 3 }, Shape { count: 3 }));
        assert_eq!(transport.0.calls(), ["send_raw", "send_raw_at_epoch 7"]);

        let other = Arc::new(Raw(Plain::new(json!({ "count": "three" }))));
        let refused = ProtocolSession::root(other)
            .send_decoded::<_, Shape>("Test.read", json!({}))
            .await;
        assert!(
            matches!(&refused, Err(CoreError::Message(message)) if message.contains("invalid type")),
            "{refused:?}"
        );
    }

    /// A transport that only answers `send` still serves a decoded send, through the trait's
    /// default raw sends.
    #[tokio::test]
    async fn a_transport_without_raw_sends_answers_a_decoded_send_through_send() {
        let transport = Arc::new(Plain::new(json!({ "count": 4 })));
        let decoded = both(&ProtocolSession::root(transport.clone()))
            .await
            .unwrap_or_else(|err| panic!("decoded sends: {err}"));
        assert_eq!(decoded, (Shape { count: 4 }, Shape { count: 4 }));
        assert_eq!(transport.calls(), ["send", "send_at_epoch 7"]);
    }
}
