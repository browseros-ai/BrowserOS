//! APC capture transport, off-reader-loop decoding, contract gating, and request-time coalescing.
//!
//! Each capture runs `Page.getAnnotatedPageContent` through [`ProtocolSession::send_decoded`] so
//! large base64 replies never build a `serde_json::Value` on the shared CDP reader loop. The
//! decoded payload is checked against `min_contract_version` before any normalization callback
//! runs, failing closed on stock Chromium builds.
//!
//! Coalescing follows design section 7.3:
//!
//! ```text
//!  attempt.started_at() < in_flight.requested_at  ─▶ join the in-flight Shared capture
//!  attempt.started_at() >= in_flight.requested_at ─▶ start a fresh capture (becomes new slot)
//!  capture completes (Ok or Err)                  ─▶ clear slot BEFORE returning (never cached)
//!  last waiter drops while in flight              ─▶ clear slot & drop Shared (cancels CDP)
//! ```

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use browseros_cdp::page::{GetAnnotatedPageContentParams, GetAnnotatedPageContentResult};
use browseros_core::{CoreError, ProtocolSession};
use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use neo_snapshot_apc::tree::{BuildError, MIN_CONTRACT_VERSION, SnapshotTree};
use neo_snapshot_apc::{AnnotatedPageContent, DecodeError};

use crate::op_scope::OpScope;
use crate::world::Stamp;
use crate::{ErrorCode, PlaywrightError};

type CaptureOutcome = Result<Arc<CapturedTree>, Arc<CaptureError>>;
type SharedCapture = Shared<BoxFuture<'static, CaptureOutcome>>;

/// Logical start timestamp captured when an attempt begins.
///
/// Passed to [`PageCapture::capture`] so an attempt may join an in-flight capture only when that
/// capture was requested strictly after this attempt started (`started < requested_at`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct AttemptStart(Stamp);

impl AttemptStart {
    /// Captures the current monotonic logical timestamp for a newly started attempt.
    #[must_use]
    pub(crate) fn now() -> Self {
        Self(Stamp::next())
    }

    /// Returns the underlying logical [`Stamp`].
    #[must_use]
    pub(crate) const fn stamp(self) -> Stamp {
        self.0
    }
}

/// A normalized [`SnapshotTree`] paired with the logical timestamp at which its capture was
/// requested.
///
/// Section 6.2 compares `world.installed_at() < captured.requested_at()` to prove that a world
/// predates the capture whose `NodeKey` is about to be hydrated.
#[derive(Debug)]
pub(crate) struct CapturedTree {
    tree: Arc<SnapshotTree>,
    requested_at: Stamp,
}

impl CapturedTree {
    /// Returns the normalized immutable [`SnapshotTree`].
    #[must_use]
    pub(crate) fn tree(&self) -> &Arc<SnapshotTree> {
        &self.tree
    }

    /// Returns the logical [`Stamp`] assigned when this capture was dispatched.
    #[must_use]
    pub(crate) const fn requested_at(&self) -> Stamp {
        self.requested_at
    }
}

/// Failure acquiring, gating, decoding, or building an APC [`SnapshotTree`].
#[derive(Debug)]
pub(crate) enum CaptureError {
    /// BrowserOS APC extension metadata is absent (stock Chromium) or below `required`.
    Unpatched { found: Option<u32>, required: u32 },
    /// The base64 or protobuf payload from `Page.getAnnotatedPageContent` could not be decoded.
    Decode(DecodeError),
    /// Normalizing the decoded `AnnotatedPageContent` into a [`SnapshotTree`] failed.
    Build(BuildError),
    /// Sending `Page.getAnnotatedPageContent` over CDP failed.
    Cdp(CoreError),
}

impl CaptureError {
    /// Translates this capture failure into a [`PlaywrightError`] attributed to `scope`.
    #[must_use]
    pub(crate) fn to_playwright_error(&self, scope: &OpScope<'_>) -> PlaywrightError {
        match self {
            Self::Unpatched { .. } => scope.fail(ErrorCode::Unsupported, &self.to_string()),
            Self::Decode(_) | Self::Build(_) => scope.fail(ErrorCode::Internal, &self.to_string()),
            Self::Cdp(error) => scope.core_error(error),
        }
    }
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unpatched { found, required } => write!(
                f,
                "unpatched Chromium APC payload (found contract version {found:?}, required >= \
                 {required})"
            ),
            Self::Decode(error) => error.fmt(f),
            Self::Build(error) => error.fmt(f),
            Self::Cdp(error) => error.fmt(f),
        }
    }
}

impl Error for CaptureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Unpatched { .. } => None,
            Self::Decode(error) => Some(error),
            Self::Build(error) => Some(error),
            Self::Cdp(error) => Some(error),
        }
    }
}

struct InFlight {
    id: Stamp,
    requested_at: Stamp,
    shared: SharedCapture,
    waiters: usize,
}

/// Per-tab coordinator for `Page.getAnnotatedPageContent` captures.
///
/// Guarantees four invariants:
/// - **Off-reader-loop decode:** replies stay raw until decoded on the caller's task.
/// - **Fail-closed contract gate:** stock Chromium or under-versioned payloads fail before `build`.
/// - **Freshness without caching:** completed captures clear the slot before returning so no later
///   attempt can observe a finished tree, and only attempts that started before `requested_at`
///   coalesce into an active capture.
/// - **Cancellation safety:** dropping one of several coalesced waiters lets the rest finish;
///   dropping the last waiter clears the slot and drops the underlying CDP future immediately.
pub(crate) struct PageCapture {
    min_contract_version: u32,
    in_flight: Arc<Mutex<Option<InFlight>>>,
}

impl Default for PageCapture {
    fn default() -> Self {
        Self::new(MIN_CONTRACT_VERSION)
    }
}

impl PageCapture {
    /// Creates a capture coordinator requiring at least `min_contract_version`, clamped up to
    /// [`MIN_CONTRACT_VERSION`] so callers can tighten but never disable the contract gate.
    #[must_use]
    pub(crate) fn new(min_contract_version: u32) -> Self {
        Self {
            min_contract_version: min_contract_version.max(MIN_CONTRACT_VERSION),
            in_flight: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns the effective minimum BrowserOS APC contract version floor.
    #[must_use]
    pub(crate) const fn min_contract_version(&self) -> u32 {
        self.min_contract_version
    }

    /// Joins an eligible in-flight capture (`started.stamp() < flight.requested_at`) or dispatches
    /// a fresh `Page.getAnnotatedPageContent` capture on `root_session`.
    ///
    /// Slot selection and `requested_at` assignment happen synchronously before returning the
    /// future so the guard is live immediately (even if dropped before the first poll) and no lock
    /// is ever held across an `.await`.
    pub(crate) fn capture<F, Fut>(
        &self,
        root_session: ProtocolSession,
        started: AttemptStart,
        build: F,
    ) -> impl Future<Output = CaptureOutcome> + Send + use<F, Fut>
    where
        F: FnOnce(AnnotatedPageContent, Stamp) -> Fut + Send + 'static,
        Fut: Future<Output = Result<SnapshotTree, CaptureError>> + Send + 'static,
    {
        let (id, shared) = self.join_or_dispatch(root_session, started, build);
        let guard = WaiterGuard {
            in_flight: Arc::clone(&self.in_flight),
            id,
        };
        async move {
            let _guard = guard;
            shared.await
        }
    }

    /// Either attaches this caller to the active capture or replaces the slot with a new one.
    /// Any replaced `InFlight` handle is dropped outside the mutex to prevent re-entrant lock
    /// acquisition if a dropped `build` closure touches shared state.
    fn join_or_dispatch<F, Fut>(
        &self,
        root_session: ProtocolSession,
        started: AttemptStart,
        build: F,
    ) -> (Stamp, SharedCapture)
    where
        F: FnOnce(AnnotatedPageContent, Stamp) -> Fut + Send + 'static,
        Fut: Future<Output = Result<SnapshotTree, CaptureError>> + Send + 'static,
    {
        let mut replaced: Option<InFlight> = None;
        let result = {
            let mut slot = self
                .in_flight
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(flight) = slot.as_mut()
                && started.stamp() < flight.requested_at
            {
                flight.waiters += 1;
                (flight.id, flight.shared.clone())
            } else {
                let requested_at = Stamp::next();
                let shared = run_capture(
                    root_session,
                    self.min_contract_version,
                    requested_at,
                    Arc::clone(&self.in_flight),
                    build,
                )
                .boxed()
                .shared();
                replaced = slot.replace(InFlight {
                    id: requested_at,
                    requested_at,
                    shared: shared.clone(),
                    waiters: 1,
                });
                (requested_at, shared)
            }
        };
        drop(replaced);
        result
    }
}

impl fmt::Debug for PageCapture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let slot = self
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut debug = f.debug_struct("PageCapture");
        debug.field("min_contract_version", &self.min_contract_version);
        if let Some(flight) = slot.as_ref() {
            debug
                .field("in_flight_id", &flight.id)
                .field("in_flight_waiters", &flight.waiters);
        } else {
            debug.field("in_flight", &false);
        }
        debug.finish()
    }
}

/// Tracks one active waiter on an in-flight capture and cancels the slot once every waiter drops.
struct WaiterGuard {
    in_flight: Arc<Mutex<Option<InFlight>>>,
    id: Stamp,
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        let mut cleared: Option<InFlight> = None;
        {
            let mut slot = self
                .in_flight
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(flight) = slot.as_mut()
                && flight.id == self.id
            {
                flight.waiters = flight.waiters.saturating_sub(1);
                if flight.waiters == 0 {
                    cleared = slot.take();
                }
            }
        }
        // Dropping `cleared` outside the mutex releases `InFlight::shared` without holding the
        // lock, cancelling the underlying CDP future once the caller's own clone has also dropped.
        drop(cleared);
    }
}

/// Clears `in_flight` if it still holds `id`, dropping the removed `InFlight` outside the lock.
fn clear_if_current(in_flight: &Mutex<Option<InFlight>>, id: Stamp) {
    let cleared = {
        let mut slot = in_flight.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.as_ref().is_some_and(|flight| flight.id == id) {
            slot.take()
        } else {
            None
        }
    };
    drop(cleared);
}

/// Drives one CDP capture, contract gate, and `build`, clearing the in-flight slot before
/// returning so completed outcomes (success or error) are never reused by a later attempt.
async fn run_capture<F, Fut>(
    root_session: ProtocolSession,
    min_contract_version: u32,
    requested_at: Stamp,
    in_flight: Arc<Mutex<Option<InFlight>>>,
    build: F,
) -> CaptureOutcome
where
    F: FnOnce(AnnotatedPageContent, Stamp) -> Fut + Send + 'static,
    Fut: Future<Output = Result<SnapshotTree, CaptureError>> + Send + 'static,
{
    let outcome = execute_capture(root_session, min_contract_version, requested_at, build).await;
    // Clear before returning so a caller woken after this capture completes never sees it in the
    // in-flight slot.
    clear_if_current(&in_flight, requested_at);
    outcome.map_err(Arc::new)
}

async fn execute_capture<F, Fut>(
    root_session: ProtocolSession,
    min_contract_version: u32,
    requested_at: Stamp,
    build: F,
) -> Result<Arc<CapturedTree>, CaptureError>
where
    F: FnOnce(AnnotatedPageContent, Stamp) -> Fut + Send + 'static,
    Fut: Future<Output = Result<SnapshotTree, CaptureError>> + Send + 'static,
{
    let reply: GetAnnotatedPageContentResult = root_session
        .send_decoded(
            "Page.getAnnotatedPageContent",
            GetAnnotatedPageContentParams {
                include_actionable_information: Some(true),
            },
        )
        .await
        .map_err(CaptureError::Cdp)?;
    let apc = neo_snapshot_apc::decode(&reply.content).map_err(CaptureError::Decode)?;
    let found = apc.browseros.and_then(|meta| meta.contract_version);
    if !matches!(found, Some(version) if version >= min_contract_version) {
        return Err(CaptureError::Unpatched {
            found,
            required: min_contract_version,
        });
    }
    let tree = build(apc, requested_at).await?;
    Ok(Arc::new(CapturedTree {
        tree: Arc::new(tree),
        requested_at,
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::error::Error;
    use std::future;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use browseros_cdp::{CdpError, CdpEvent, RawReply, TargetedEvents};
    use browseros_core::{CdpConnection, CoreError, ProtocolSession, SessionId};
    use futures_util::future::BoxFuture;
    use neo_snapshot_apc::AnnotatedPageContent;
    use neo_snapshot_apc::tree::{BuildError, BuildOptions, MIN_CONTRACT_VERSION, SnapshotTree};
    use serde_json::{Value, json};
    use tokio::sync::{broadcast, mpsc, oneshot};
    use tokio::task;

    use super::{AttemptStart, CaptureError};
    use crate::ErrorCode;
    use crate::op_scope::OpScope;
    use crate::target::CallHeader;
    use crate::test_env::TestEnv;
    use crate::test_support::{TestResult, count_op};
    use crate::tree::PageCapture;
    use crate::world::Stamp;

    fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn encode_len_delimited(tag: u32, payload: &[u8], out: &mut Vec<u8>) {
        encode_varint((u64::from(tag) << 3) | 2, out);
        encode_varint(payload.len() as u64, out);
        out.extend_from_slice(payload);
    }

    fn encode_varint_field(tag: u32, value: u64, out: &mut Vec<u8>) {
        encode_varint(u64::from(tag) << 3, out);
        encode_varint(value, out);
    }

    /// Encodes a minimal valid `AnnotatedPageContent` protobuf payload as standard base64.
    fn encode_apc_base64(doc_token: &str, contract_version: Option<u32>) -> String {
        let mut doc_id = Vec::new();
        encode_len_delimited(1, doc_token.as_bytes(), &mut doc_id);

        let mut frame_data = Vec::new();
        encode_len_delimited(3, &doc_id, &mut frame_data);

        let mut attrs = Vec::new();
        encode_varint_field(2, 1, &mut attrs); // common_ancestor_dom_node_id = 1
        encode_varint_field(3, 1, &mut attrs); // attribute_type = Root (1)
        let mut root_node = Vec::new();
        encode_len_delimited(2, &attrs, &mut root_node);

        let mut apc = Vec::new();
        encode_varint_field(1, 1, &mut apc); // version = Version10
        encode_len_delimited(2, &root_node, &mut apc); // root_node
        encode_len_delimited(3, &frame_data, &mut apc); // main_frame_data
        if let Some(version) = contract_version {
            let mut browseros = Vec::new();
            encode_varint_field(1, u64::from(version), &mut browseros);
            encode_len_delimited(1000, &browseros, &mut apc);
        }
        STANDARD.encode(apc)
    }

    fn default_build(
        apc: AnnotatedPageContent,
        _requested_at: Stamp,
    ) -> impl Future<Output = Result<SnapshotTree, CaptureError>> + Send + 'static {
        future::ready(SnapshotTree::build(apc, BuildOptions::new()).map_err(CaptureError::Build))
    }

    enum ScriptedStep {
        Immediate(Result<Value, CdpError>),
        Deferred {
            dispatched: oneshot::Sender<()>,
            reply_rx: oneshot::Receiver<Result<Value, CdpError>>,
        },
    }

    struct ActiveGuard(Arc<AtomicUsize>);

    impl ActiveGuard {
        fn enter(counter: &Arc<AtomicUsize>) -> Self {
            counter.fetch_add(1, Ordering::SeqCst);
            Self(Arc::clone(counter))
        }
    }

    impl Drop for ActiveGuard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    struct FakeApcCdp {
        steps: Mutex<VecDeque<ScriptedStep>>,
        raw_calls: Mutex<Vec<(String, Value)>>,
        plain_send_calls: AtomicUsize,
        active_calls: Arc<AtomicUsize>,
        targeted: TargetedEvents,
    }

    impl FakeApcCdp {
        fn new(steps: Vec<ScriptedStep>) -> Arc<Self> {
            Arc::new(Self {
                steps: Mutex::new(VecDeque::from(steps)),
                raw_calls: Mutex::default(),
                plain_send_calls: AtomicUsize::new(0),
                active_calls: Arc::new(AtomicUsize::new(0)),
                targeted: TargetedEvents::default(),
            })
        }

        fn session(self: &Arc<Self>) -> ProtocolSession {
            ProtocolSession::for_session(
                Arc::clone(self) as Arc<dyn CdpConnection>,
                SessionId::from("S-apc"),
            )
        }

        fn raw_calls(&self) -> Vec<(String, Value)> {
            self.raw_calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        fn active_calls(&self) -> usize {
            self.active_calls.load(Ordering::SeqCst)
        }
    }

    impl CdpConnection for FakeApcCdp {
        fn send<'a>(
            &'a self,
            _method: &'a str,
            _params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            self.plain_send_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(CdpError::NotConnected) })
        }

        fn send_raw<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<RawReply, CdpError>> {
            self.raw_calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((method.to_owned(), params));
            let step = self
                .steps
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front();
            let active = Arc::clone(&self.active_calls);
            Box::pin(async move {
                let _guard = ActiveGuard::enter(&active);
                let reply = match step {
                    Some(ScriptedStep::Immediate(result)) => result?,
                    Some(ScriptedStep::Deferred {
                        dispatched,
                        reply_rx,
                    }) => {
                        let _ = dispatched.send(());
                        reply_rx
                            .await
                            .map_err(|_closed| CdpError::ConnectionLost)??
                    }
                    None => return Err(CdpError::NotConnected),
                };
                RawReply::from_value(&reply)
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Err(CdpError::NotConnected) })
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
            1
        }
    }

    fn deferred_step() -> (
        ScriptedStep,
        oneshot::Receiver<()>,
        oneshot::Sender<Result<Value, CdpError>>,
    ) {
        let (dispatched_tx, dispatched_rx) = oneshot::channel();
        let (tx, rx) = oneshot::channel();
        (
            ScriptedStep::Deferred {
                dispatched: dispatched_tx,
                reply_rx: rx,
            },
            dispatched_rx,
            tx,
        )
    }

    fn apc_reply(doc_token: &str, contract_version: Option<u32>) -> Value {
        json!({ "content": encode_apc_base64(doc_token, contract_version) })
    }

    #[tokio::test]
    async fn contract_gate_refuses_stock_zero_and_lower_versions_and_accepts_floor() -> TestResult {
        let clamped = PageCapture::new(0);
        assert_eq!(
            clamped.min_contract_version(),
            MIN_CONTRACT_VERSION,
            "PageCapture::new(0) must clamp up to MIN_CONTRACT_VERSION"
        );

        let cdp = FakeApcCdp::new(vec![
            ScriptedStep::Immediate(Ok(apc_reply("doc-stock", None))),
            ScriptedStep::Immediate(Ok(apc_reply("doc-v0", Some(0)))),
            ScriptedStep::Immediate(Ok(apc_reply("doc-v1", Some(1)))),
            ScriptedStep::Immediate(Ok(apc_reply("doc-v2", Some(2)))),
            ScriptedStep::Immediate(Ok(apc_reply("doc-v3", Some(3)))),
        ]);
        let capture = PageCapture::new(2);
        assert_eq!(capture.min_contract_version(), 2);

        for expected_found in [None, Some(0), Some(1)] {
            let build_called = Arc::new(AtomicBool::new(false));
            let build_flag = Arc::clone(&build_called);
            let err = capture
                .capture(cdp.session(), AttemptStart::now(), move |apc, at| {
                    build_flag.store(true, Ordering::SeqCst);
                    default_build(apc, at)
                })
                .await
                .err()
                .ok_or("under-versioned payload must be refused")?;
            assert!(
                !build_called.load(Ordering::SeqCst),
                "contract gate must refuse before invoking build"
            );
            assert!(
                matches!(
                    err.as_ref(),
                    CaptureError::Unpatched { found, required: 2 } if *found == expected_found
                ),
                "expected Unpatched {{ found: {expected_found:?}, required: 2 }}, got {err:?}"
            );
        }

        for (expected_doc, expected_version) in [("doc-v2", 2), ("doc-v3", 3)] {
            let captured = capture
                .capture(cdp.session(), AttemptStart::now(), default_build)
                .await?;
            let main = captured.tree().main_document().ok_or("main doc")?;
            assert_eq!(main.token().as_str(), expected_doc);
            assert_eq!(captured.tree().contract_version(), expected_version);
        }

        assert_eq!(
            cdp.plain_send_calls.load(Ordering::SeqCst),
            0,
            "PageCapture must use send_decoded (send_raw) off the CDP reader loop, never send"
        );
        let calls = cdp.raw_calls();
        assert_eq!(calls.len(), 5);
        for (method, params) in calls {
            assert_eq!(method, "Page.getAnnotatedPageContent");
            assert_eq!(
                params,
                json!({ "includeActionableInformation": true }),
                "capture must request actionable information"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn coalesces_attempts_that_started_before_capture_dispatch() -> TestResult {
        let (step, dispatched, tx) = deferred_step();
        let cdp = FakeApcCdp::new(vec![step]);
        let capture = PageCapture::default();

        let start_a = AttemptStart::now();
        let start_b = AttemptStart::now();

        let task_a = tokio::spawn(capture.capture(cdp.session(), start_a, default_build));
        let task_b = tokio::spawn(capture.capture(cdp.session(), start_b, default_build));

        dispatched.await?;
        assert_eq!(
            cdp.raw_calls().len(),
            1,
            "two attempts that began before dispatch must share one CDP capture"
        );
        let debug_in_flight = format!("{capture:?}");
        assert!(
            debug_in_flight.contains("in_flight_waiters: 2"),
            "Debug must reflect both coalesced waiters: {debug_in_flight}"
        );

        tx.send(Ok(apc_reply("doc-shared", Some(MIN_CONTRACT_VERSION))))
            .map_err(|_unsent| "reply channel closed")?;
        let res_a = task_a.await??;
        let res_b = task_b.await??;

        assert!(
            Arc::ptr_eq(&res_a, &res_b),
            "coalesced attempts must receive the exact same Arc<CapturedTree>"
        );
        assert!(start_a.stamp() < res_a.requested_at() && start_b.stamp() < res_a.requested_at());
        assert_eq!(cdp.raw_calls().len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn attempt_started_after_dispatch_sends_its_own_capture() -> TestResult {
        let (step_1, dispatched_1, tx_1) = deferred_step();
        let (step_2, dispatched_2, tx_2) = deferred_step();
        let cdp = FakeApcCdp::new(vec![step_1, step_2]);
        let capture = PageCapture::default();

        let start_a = AttemptStart::now();
        let task_a = tokio::spawn(capture.capture(cdp.session(), start_a, default_build));
        dispatched_1.await?;

        let start_c = AttemptStart::now();
        let task_c = tokio::spawn(capture.capture(cdp.session(), start_c, default_build));
        dispatched_2.await?;

        assert_eq!(
            cdp.raw_calls().len(),
            2,
            "an attempt starting after Capture 1 was dispatched must dispatch Capture 2"
        );

        tx_1.send(Ok(apc_reply("doc-1", Some(MIN_CONTRACT_VERSION))))
            .map_err(|_unsent| "reply 1 closed")?;
        tx_2.send(Ok(apc_reply("doc-2", Some(MIN_CONTRACT_VERSION))))
            .map_err(|_unsent| "reply 2 closed")?;

        let tree_a = task_a.await??;
        let tree_c = task_c.await??;

        assert!(!Arc::ptr_eq(&tree_a, &tree_c));
        assert_eq!(
            tree_a
                .tree()
                .main_document()
                .ok_or("doc 1")?
                .token()
                .as_str(),
            "doc-1"
        );
        assert_eq!(
            tree_c
                .tree()
                .main_document()
                .ok_or("doc 2")?
                .token()
                .as_str(),
            "doc-2"
        );
        assert!(
            tree_a.requested_at() < start_c.stamp() && start_c.stamp() < tree_c.requested_at(),
            "Capture 1 must predate Attempt C, and Attempt C must predate Capture 2"
        );
        Ok(())
    }

    #[tokio::test]
    async fn completed_capture_is_never_reused_even_for_an_earlier_attempt_start() -> TestResult {
        let cdp = FakeApcCdp::new(vec![
            ScriptedStep::Immediate(Ok(apc_reply("doc-first", Some(MIN_CONTRACT_VERSION)))),
            ScriptedStep::Immediate(Ok(apc_reply("doc-second", Some(MIN_CONTRACT_VERSION)))),
        ]);
        let capture = PageCapture::default();

        let early_start = AttemptStart::now();
        let first = capture
            .capture(cdp.session(), AttemptStart::now(), default_build)
            .await?;
        assert_eq!(
            first
                .tree()
                .main_document()
                .ok_or("first")?
                .token()
                .as_str(),
            "doc-first"
        );

        let second = capture
            .capture(cdp.session(), early_start, default_build)
            .await?;
        assert_eq!(
            second
                .tree()
                .main_document()
                .ok_or("second")?
                .token()
                .as_str(),
            "doc-second"
        );
        assert!(
            !Arc::ptr_eq(&first, &second),
            "completed captures must be cleared before returning and never cached"
        );
        assert_eq!(cdp.raw_calls().len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_preserves_remaining_waiters_and_cancels_when_sole_waiter_drops()
    -> TestResult {
        // (a) Two coalesced waiters: aborting Attempt A while in flight lets Attempt B finish.
        let (step_1, dispatched_1, tx_1) = deferred_step();
        let (step_2, dispatched_2, _tx_2) = deferred_step();
        let cdp = FakeApcCdp::new(vec![
            step_1,
            step_2,
            ScriptedStep::Immediate(Ok(apc_reply(
                "doc-after-cancel",
                Some(MIN_CONTRACT_VERSION),
            ))),
        ]);
        let capture = PageCapture::default();

        let start_a = AttemptStart::now();
        let start_b = AttemptStart::now();
        let task_a = tokio::spawn(capture.capture(cdp.session(), start_a, default_build));
        let task_b = tokio::spawn(capture.capture(cdp.session(), start_b, default_build));

        dispatched_1.await?;
        assert_eq!(cdp.raw_calls().len(), 1);
        task_a.abort();
        let _ = task_a.await;
        task::yield_now().await;
        assert_eq!(
            cdp.active_calls(),
            1,
            "dropping one of two waiters must keep the shared CDP capture alive"
        );

        tx_1.send(Ok(apc_reply("doc-survivor", Some(MIN_CONTRACT_VERSION))))
            .map_err(|_unsent| "reply 1 closed")?;
        let survivor = task_b.await??;
        assert_eq!(
            survivor
                .tree()
                .main_document()
                .ok_or("survivor")?
                .token()
                .as_str(),
            "doc-survivor"
        );

        // (b) Sole waiter drops while in flight: slot clears and underlying CDP future drops.
        let earlier = AttemptStart::now();
        let task_sole =
            tokio::spawn(capture.capture(cdp.session(), AttemptStart::now(), default_build));
        dispatched_2.await?;
        assert_eq!(cdp.active_calls(), 1);

        task_sole.abort();
        let _ = task_sole.await;
        task::yield_now().await;
        assert_eq!(
            cdp.active_calls(),
            0,
            "dropping the sole waiter must cancel the in-flight CDP future with zero leaked tasks"
        );
        assert!(
            format!("{capture:?}").contains("in_flight: false"),
            "in_flight slot must be empty once the sole waiter drops"
        );

        let fresh = capture
            .capture(cdp.session(), earlier, default_build)
            .await?;
        assert_eq!(
            fresh
                .tree()
                .main_document()
                .ok_or("fresh")?
                .token()
                .as_str(),
            "doc-after-cancel"
        );
        assert_eq!(
            cdp.raw_calls().len(),
            3,
            "subsequent attempt after sole-waiter cancellation must send a fresh CDP call"
        );
        Ok(())
    }

    #[tokio::test]
    async fn invalid_base64_and_corrupt_protobuf_return_decode_error() -> TestResult {
        let cdp = FakeApcCdp::new(vec![
            ScriptedStep::Immediate(Ok(json!({ "content": "!!!not-base64!!!" }))),
            ScriptedStep::Immediate(Ok(
                json!({ "content": STANDARD.encode([0xff_u8, 0xff, 0xff]) }),
            )),
        ]);
        let capture = PageCapture::default();

        let bad_b64 = capture
            .capture(cdp.session(), AttemptStart::now(), default_build)
            .await
            .err()
            .ok_or("invalid base64 must fail")?;
        assert!(
            matches!(bad_b64.as_ref(), CaptureError::Decode(_)),
            "invalid base64 must return CaptureError::Decode: {bad_b64:?}"
        );
        assert!(bad_b64.source().is_some());

        let bad_proto = capture
            .capture(cdp.session(), AttemptStart::now(), default_build)
            .await
            .err()
            .ok_or("corrupt protobuf must fail")?;
        assert!(
            matches!(bad_proto.as_ref(), CaptureError::Decode(_)),
            "corrupt protobuf must return CaptureError::Decode: {bad_proto:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn capture_error_maps_to_playwright_error_codes_and_propagates_cdp_and_build_failures()
    -> TestResult {
        let env = TestEnv::new();
        let scope = OpScope::new(&env, count_op()?, CallHeader::default());

        let cdp = FakeApcCdp::new(vec![
            ScriptedStep::Immediate(Err(CdpError::ConnectionLost)),
            ScriptedStep::Immediate(Ok(apc_reply("doc-build-fail", Some(MIN_CONTRACT_VERSION)))),
        ]);
        let capture = PageCapture::default();

        let cdp_err = capture
            .capture(cdp.session(), AttemptStart::now(), default_build)
            .await
            .err()
            .ok_or("CDP failure must fail capture")?;
        assert!(matches!(
            cdp_err.as_ref(),
            CaptureError::Cdp(CoreError::Cdp(CdpError::ConnectionLost))
        ));
        assert!(cdp_err.source().is_some());
        assert_eq!(
            cdp_err.to_playwright_error(&scope).error_code(),
            Some(ErrorCode::BrowserDisconnected)
        );

        let build_err = capture
            .capture(cdp.session(), AttemptStart::now(), |_apc, _at| {
                future::ready(Err(CaptureError::Build(BuildError::MissingMainFrame)))
            })
            .await
            .err()
            .ok_or("build error must fail capture")?;
        assert!(matches!(
            build_err.as_ref(),
            CaptureError::Build(BuildError::MissingMainFrame)
        ));
        assert!(build_err.source().is_some());
        assert_eq!(
            build_err.to_playwright_error(&scope).error_code(),
            Some(ErrorCode::Internal)
        );

        let unpatched = CaptureError::Unpatched {
            found: None,
            required: MIN_CONTRACT_VERSION,
        };
        assert!(unpatched.source().is_none());
        assert_eq!(
            unpatched.to_playwright_error(&scope).error_code(),
            Some(ErrorCode::Unsupported)
        );

        let decode_err = CaptureError::Decode(
            neo_snapshot_apc::decode("!!!")
                .err()
                .ok_or("decode error")?,
        );
        assert_eq!(
            decode_err.to_playwright_error(&scope).error_code(),
            Some(ErrorCode::Internal)
        );
        Ok(())
    }
}
