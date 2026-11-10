//! Neo: Bridges a CDP target id to a fresh browseros-core view for one solve. The host's existing
//! client is shared, but every solver attachment is recorded and detached after input drains.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use browseros_cdp::{CdpClient, CdpError, CdpEvent};
use browseros_core::{Browser, BrowserView, SessionId, connection::CdpConnection};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::Instant;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{CaptchaSolver, HumanHelp, SolveRefusal, types::SolveOutcome};

/// Process-lifetime solver and target-to-reservation identities. Fresh browseros-core page
/// numbers can change with Browser.getTabs ordering, so they cannot identify reservations.
#[derive(Clone)]
pub struct TargetSolver {
    solver: Arc<CaptchaSolver>,
    connection: Arc<dyn CdpConnection>,
    keys: Arc<Mutex<HashMap<String, u32>>>,
}

impl TargetSolver {
    pub fn new(solver: Arc<CaptchaSolver>, cdp: CdpClient) -> Self {
        Self::over_connection(solver, Arc::new(cdp))
    }

    fn over_connection(solver: Arc<CaptchaSolver>, connection: Arc<dyn CdpConnection>) -> Self {
        Self {
            solver,
            connection,
            keys: Arc::default(),
        }
    }

    fn reservation_key(&self, target: &str) -> Result<u32, SolveRefusal> {
        let mut keys = self
            .keys
            .lock()
            .map_err(|_| SolveRefusal::BrowserUnavailable)?;
        if let Some(key) = keys.get(target) {
            return Ok(*key);
        }
        let key = u32::try_from(keys.len())
            .ok()
            .and_then(|key| key.checked_add(1))
            .ok_or(SolveRefusal::BrowserUnavailable)?;
        keys.insert(target.to_owned(), key);
        Ok(key)
    }

    /// The caller must await this future even after cancelling `cancel`: the final input
    /// task and late attachment replies are drained before any solver session detaches.
    pub async fn solve(
        &self,
        target_id: &str,
        cancel: CancellationToken,
    ) -> Result<SolveOutcome, SolveRefusal> {
        self.solve_using(target_id, cancel, &crate::LiveDriver)
            .await
    }

    async fn solve_using(
        &self,
        target_id: &str,
        cancel: CancellationToken,
        driver: &impl crate::SolveDriver,
    ) -> Result<SolveOutcome, SolveRefusal> {
        let started = Instant::now();
        let key = self.reservation_key(target_id)?;
        let connection = Arc::new(SolveConnection::new(Arc::clone(&self.connection)));
        let result = async {
            let browser = Browser::new(connection.clone());
            let view = BrowserView::unbound(browser.session(), None);
            // No input or attachment has started, so this read may be dropped on stop.
            // Keeping it alive would retain the runtime's claim/input turn for a CDP timeout.
            let listing = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Ok(self.cancelled_preflight(key, started)),
                listing = view.list_pages() => listing
                    .map_err(|error| SolveRefusal::InvalidPage(error.to_string()))?,
            };
            let page = listing
                .pages
                .into_iter()
                .find(|page| page.target_id.as_str() == target_id)
                .ok_or_else(|| {
                    SolveRefusal::InvalidPage("target is missing or excluded".to_owned())
                })?;
            self.solver
                .solve_target(
                    view,
                    page.page_id,
                    key,
                    cancel,
                    HumanHelp::Unavailable,
                    driver,
                )
                .await
        }
        .await;
        // solve_target has drained LivePage's input. Also settle attachments whose caller
        // stopped waiting, or an acknowledged root session could escape this cleanup.
        connection.detach_all().await;
        result
    }

    /// Before a page is known, cancellation still uses the ordinary caller-stop outcome.
    /// The host maps this to Stopped/unknown delivery, rather than an unsent page refusal.
    fn cancelled_preflight(&self, key: u32, started: Instant) -> SolveOutcome {
        let flow = crate::page_failure(crate::types::PageError::Cancelled);
        let request_id = format!("cap_{}", ulid::Ulid::new());
        let events =
            crate::events::EventLog::new(self.solver.events().clone(), request_id.clone(), key);
        events.emit(
            crate::types::CaptchaEventName::Failed,
            json!({"type":null,"status":flow.status,"reason":flow.reason}),
        );
        let outcome = SolveOutcome {
            any_click_failed: false,
            status: flow.status,
            kind: None,
            method: flow.method,
            reason: flow.reason,
            request_id,
            page: key,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            attempts: flow.attempts,
            paid_tasks: flow.paid_tasks,
            cost_usd: flow.cost_usd,
            message: flow.message,
            events: events.events(),
        };
        crate::events::log_outcome(&outcome, None, None);
        outcome
    }
}

struct SolveConnection {
    inner: Arc<dyn CdpConnection>,
    sessions: Arc<Mutex<HashSet<SessionId>>>,
    event_sessions: Arc<Mutex<HashSet<SessionId>>>,
    attachments: TaskTracker,
    event_tasks: TaskTracker,
    closed: CancellationToken,
    admission: Mutex<()>,
}

impl SolveConnection {
    fn new(inner: Arc<dyn CdpConnection>) -> Self {
        Self {
            inner,
            sessions: Arc::default(),
            event_sessions: Arc::default(),
            attachments: TaskTracker::new(),
            event_tasks: TaskTracker::new(),
            closed: CancellationToken::new(),
            admission: Mutex::new(()),
        }
    }

    fn request<'a>(
        &'a self,
        epoch: Option<u64>,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        // Registration and cleanup's close share this synchronous boundary. TaskTracker::close
        // alone permits new tasks, so a check-then-spawn race could otherwise leak an attach.
        let Ok(_admission) = self.admission.lock() else {
            return Box::pin(async { Err(CdpError::SessionGone) });
        };
        if self.closed.is_cancelled() {
            return Box::pin(async { Err(CdpError::SessionGone) });
        }
        if method != "Target.attachToTarget" || session.is_some() {
            return match epoch {
                Some(epoch) => self.inner.send_at_epoch(epoch, method, params, session),
                None => self.inner.send(method, params, session),
            };
        }
        let inner = Arc::clone(&self.inner);
        let sessions = Arc::clone(&self.sessions);
        let event_sessions = Arc::clone(&self.event_sessions);
        let (reply, receiver) = oneshot::channel();
        self.attachments.spawn(async move {
            let result = match epoch {
                Some(epoch) => {
                    inner
                        .send_at_epoch(epoch, "Target.attachToTarget", params, None)
                        .await
                }
                None => inner.send("Target.attachToTarget", params, None).await,
            };
            if let Ok(value) = &result
                && let Some(session) = value["sessionId"].as_str()
                && let Ok(mut sessions) = sessions.lock()
            {
                let session = SessionId::from(session.to_owned());
                sessions.insert(session.clone());
                if let Ok(mut known) = event_sessions.lock() {
                    known.insert(session);
                }
            }
            let _ = reply.send(result);
        });
        Box::pin(async move {
            receiver
                .await
                .map_err(|_| CdpError::Transport("solver attachment task ended".to_owned()))?
        })
    }

    async fn detach_all(&self) {
        // Core's frame/detach listeners retain their registries until events close. Forwarding
        // the host's global stream directly would keep a fresh per-solve cache alive forever.
        // Closing this local stream also refuses late listener setup before sessions detach.
        {
            let _admission = self.admission.lock();
            self.closed.cancel();
        }
        self.event_tasks.close();
        self.event_tasks.wait().await;
        self.attachments.close();
        self.attachments.wait().await;
        let sessions = self
            .sessions
            .lock()
            .map(|sessions| sessions.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for session in sessions {
            // Input and attachment work has already settled. A vanished tab/session needs no
            // further cleanup; a dead link must not make detach itself hold the claim forever.
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                self.inner.send(
                    "Target.detachFromTarget",
                    json!({"sessionId":session.as_str()}),
                    None,
                ),
            )
            .await;
        }
    }
}

/// Core sees only sessions attached by this solve and their auto-attached descendants. Without
/// this filter its frame listener would enable domains on the runtime's own OOPIF sessions.
fn owns_event(sessions: &Mutex<HashSet<SessionId>>, event: &CdpEvent) -> bool {
    let Ok(mut sessions) = sessions.lock() else {
        return false;
    };
    if let Some(parent) = &event.session_id {
        if !sessions.contains(parent) {
            return false;
        }
        if event.method == "Target.attachedToTarget"
            && let Some(child) = event.params["sessionId"].as_str()
        {
            sessions.insert(SessionId::from(child.to_owned()));
        }
        return true;
    }
    if matches!(
        event.method.as_str(),
        "Target.attachedToTarget" | "Target.detachedFromTarget"
    ) {
        return event.params["sessionId"]
            .as_str()
            .is_some_and(|session| sessions.contains(&SessionId::from(session.to_owned())));
    }
    true
}

impl CdpConnection for SolveConnection {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        self.request(None, method, params, session)
    }
    fn send_at_epoch<'a>(
        &'a self,
        epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        self.request(Some(epoch), method, params, session)
    }
    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        if self.closed.is_cancelled() {
            return Box::pin(async { Err(CdpError::SessionGone) });
        }
        if method != "Target.attachToTarget" || session.is_some() {
            return self.inner.send_raw_json(method, params, session);
        }
        Box::pin(async move {
            let params =
                serde_json::from_str(params).map_err(|error| CdpError::Json(error.to_string()))?;
            self.request(None, method, params, session)
                .await
                .map(|value| value.to_string())
        })
    }
    fn send_raw_json_at_epoch<'a>(
        &'a self,
        epoch: u64,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        if self.closed.is_cancelled() {
            return Box::pin(async { Err(CdpError::SessionGone) });
        }
        if method != "Target.attachToTarget" || session.is_some() {
            return self
                .inner
                .send_raw_json_at_epoch(epoch, method, params, session);
        }
        Box::pin(async move {
            let params =
                serde_json::from_str(params).map_err(|error| CdpError::Json(error.to_string()))?;
            self.request(Some(epoch), method, params, session)
                .await
                .map(|value| value.to_string())
        })
    }
    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        let (sender, receiver) = broadcast::channel(256);
        let mut source = self.inner.events();
        let closed = self.closed.clone();
        let sessions = Arc::clone(&self.event_sessions);
        self.event_tasks.spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = closed.cancelled() => break,
                    event = source.recv() => match event {
                        Ok(event) if owns_event(&sessions, &event) => { let _ = sender.send(event); }
                        Ok(_) => {},
                        Err(broadcast::error::RecvError::Lagged(_)) => {},
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        });
        receiver
    }
    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        // A targeted subscription on the shared client is exclusive. Filter its broadcast
        // instead, so a solver cannot replace another host consumer's targeted subscription.
        let (sender, receiver) = mpsc::unbounded_channel();
        let mut source = self.inner.events();
        let closed = self.closed.clone();
        let method = method.to_owned();
        let sessions = Arc::clone(&self.event_sessions);
        self.event_tasks.spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = closed.cancelled() => break,
                    event = source.recv() => match event {
                        Ok(event) if owns_event(&sessions, &event) && event.method == method => { if sender.send(event).is_err() { break; } }
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {},
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        });
        receiver
    }
    fn is_connected(&self) -> bool {
        !self.closed.is_cancelled() && self.inner.is_connected()
    }
    fn connection_epoch(&self) -> u64 {
        self.inner.connection_epoch()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::{CaptchaPage, Native};
    use crate::providers::{CaptchaBackend, ProviderChain};
    use crate::types::{
        Challenge, Detection, FlowResult, PageError, ScanOutcome, SolveReason, SolveStatus,
    };
    use crate::{SolveCtx, SolveDriver};
    use std::sync::atomic::Ordering;

    fn solver(native: Arc<Native>) -> TargetSolver {
        TargetSolver::over_connection(
            Arc::new(CaptchaSolver {
                providers: ProviderChain::new(&crate::config::CaptchaConfig::default()),
                reservations: Arc::default(),
                events: crate::events::CaptchaEventBus::new(),
                forced_method: None,
            }),
            native,
        )
    }

    struct Clicking {
        twice: bool,
    }
    impl SolveDriver for Clicking {
        async fn detect<P: CaptchaPage>(&self, page: &P) -> Result<Detection, PageError> {
            let first = page.click_at(20.0, 30.0).await;
            if self.twice {
                page.click_at(60.0, 70.0).await?;
            } else {
                first?;
            }
            Ok(Detection {
                outcome: ScanOutcome::None,
                challenges: vec![],
                complete: true,
                document: page.document().await?,
            })
        }
        async fn solve<P: CaptchaPage, B: CaptchaBackend>(
            &self,
            _: &SolveCtx<'_, P, B>,
            _: &Challenge,
        ) -> FlowResult {
            unreachable!("the driver detects a clear page after exercising input")
        }
    }

    #[tokio::test(start_paused = true)]
    async fn captcha_target_precancelled_preflight_returns_caller_stop_without_reading() {
        let native = Native::new("https://widget.test");
        let driver = solver(Arc::clone(&native));
        let cancel = CancellationToken::new();
        cancel.cancel();
        let outcome = driver.solve("page", cancel).await.expect("caller stop");
        assert_eq!(outcome.status, SolveStatus::Cancelled);
        assert_eq!(outcome.reason, SolveReason::CallerCancelled);
        assert!(!outcome.any_click_failed);
        assert_eq!(outcome.attempts, crate::types::Attempts::default());
        assert_eq!(outcome.paid_tasks, 0);
        assert_eq!(outcome.events.len(), 1);
        assert!(crate::script::ScriptOutcome::from_outcome(&outcome).is_none());
        assert!(native.calls("Browser.getTabs").expect("tabs").is_empty());
        assert!(
            native
                .calls("Target.attachToTarget")
                .expect("attach")
                .is_empty()
        );
        assert!(
            native
                .calls("Input.dispatchMouseEvent")
                .expect("input")
                .is_empty()
        );
        assert!(driver.solver.reservations().active(1).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn captcha_target_soft_timeout_drains_release_before_detach_and_remains_settled() {
        let native = Native::new("https://widget.test");
        *native.release_delay.lock().expect("delay") = Duration::from_secs(5);
        let outcome = solver(Arc::clone(&native))
            .solve_using("page", CancellationToken::new(), &Clicking { twice: false })
            .await
            .expect("outcome");
        assert!(!outcome.any_click_failed);
        assert_eq!(outcome.reason, SolveReason::BrowserError);
        let ack = native.calls("release_ack").expect("ack");
        let detach = native.calls("Target.detachFromTarget").expect("detach");
        assert_eq!(ack.len(), 1);
        assert_eq!(detach.len(), 1);
        assert!(detach[0].3 >= ack[0].3);
    }

    #[tokio::test(start_paused = true)]
    async fn captcha_target_failure_then_success_is_sticky_without_legacy_json_fields() {
        let native = Native::new("https://widget.test");
        native.release_failure.store(true, Ordering::SeqCst);
        let outcome = solver(Arc::clone(&native))
            .solve_using("page", CancellationToken::new(), &Clicking { twice: true })
            .await
            .expect("outcome");
        assert_eq!(outcome.status, SolveStatus::AlreadyClear);
        assert!(outcome.any_click_failed);
        let wire = serde_json::to_value(&outcome).expect("legacy JSON");
        assert!(wire.get("anyClickFailed").is_none());
        assert!(wire.get("any_click_failed").is_none());
        assert_eq!(wire.as_object().expect("object").len(), 12);
        assert!(crate::script::ScriptOutcome::from_outcome(&outcome).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn captcha_target_cancel_inside_press_keeps_reservation_through_release_and_detach() {
        let native = Native::new("https://widget.test");
        let driver = solver(Arc::clone(&native));
        let cancel = CancellationToken::new();
        let running = {
            let driver = driver.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                driver
                    .solve_using("page", cancel, &Clicking { twice: false })
                    .await
            })
        };
        for _ in 0..50 {
            tokio::task::yield_now().await;
            tokio::time::advance(Duration::from_millis(12)).await;
            if native
                .calls("Input.dispatchMouseEvent")
                .expect("mouse")
                .iter()
                .any(|call| call.1["type"] == "mousePressed")
            {
                break;
            }
        }
        assert!(
            native
                .calls("Input.dispatchMouseEvent")
                .expect("mouse")
                .iter()
                .any(|call| call.1["type"] == "mousePressed")
        );
        cancel.cancel();
        tokio::task::yield_now().await;
        assert!(
            driver.solver.reservations().active(1).is_some(),
            "reservation survives the dropped flow"
        );
        assert!(
            native
                .calls("Target.detachFromTarget")
                .expect("detach")
                .is_empty()
        );
        let outcome = running.await.expect("task").expect("outcome");
        assert_eq!(outcome.reason, SolveReason::CallerCancelled);
        assert!(!outcome.any_click_failed);
        let ack = native.calls("release_ack").expect("ack");
        let detach = native.calls("Target.detachFromTarget").expect("detach");
        assert_eq!(ack.len(), 1);
        assert!(detach[0].3 >= ack[0].3);
        assert!(driver.solver.reservations().active(1).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn captcha_target_missing_and_excluded_are_refusals_without_attaching() {
        let native = Native::new("https://widget.test");
        let driver = solver(Arc::clone(&native));
        assert!(matches!(
            driver.solve("missing", CancellationToken::new()).await,
            Err(SolveRefusal::InvalidPage(_))
        ));
        for prefix in browseros_core::connection::EXCLUDED_URL_PREFIXES {
            *native.tabs_override.lock().expect("tabs") = Some(vec![
                json!({"tabId":1,"targetId":"page","url":format!("{prefix}x"),"title":"fixture","isActive":true,"isLoading":false,"loadProgress":1.0,"isPinned":false,"isHidden":false,"windowId":1}),
            ]);
            let refusal = driver
                .solve("page", CancellationToken::new())
                .await
                .expect_err("excluded");
            assert!(matches!(refusal, SolveRefusal::InvalidPage(_)));
            assert!(
                crate::script::refusal(&refusal)
                    .1
                    .starts_with("neo.solveCaptcha: the solver cannot reach this page")
            );
        }
        assert!(
            native
                .calls("Target.attachToTarget")
                .expect("attach")
                .is_empty()
        );
    }

    #[test]
    fn captcha_reservation_identity_survives_reordering_and_closed_tabs() {
        let driver = solver(Native::new("https://widget.test"));
        let first = driver.reservation_key("first").expect("first");
        let next = driver.reservation_key("next").expect("next");
        assert_ne!(first, next);
        // Browser-local page numbering can reuse 1 after first closes. Keys never reuse it.
        assert_eq!(driver.reservation_key("next").expect("next again"), next);
        assert_eq!(driver.reservation_key("first").expect("first again"), first);
    }

    #[tokio::test]
    async fn captcha_records_every_root_attachment_transport_variant() {
        let native = Native::new("https://widget.test");
        let connection = SolveConnection::new(native.clone());
        connection
            .send(
                "Target.attachToTarget",
                json!({"targetId":"a","flatten":true}),
                None,
            )
            .await
            .expect("send");
        connection
            .send_at_epoch(
                1,
                "Target.attachToTarget",
                json!({"targetId":"b","flatten":true}),
                None,
            )
            .await
            .expect("epoch send");
        connection
            .send_raw_json(
                "Target.attachToTarget",
                r#"{"targetId":"c","flatten":true}"#,
                None,
            )
            .await
            .expect("raw");
        connection
            .send_raw_json_at_epoch(
                1,
                "Target.attachToTarget",
                r#"{"targetId":"d","flatten":true}"#,
                None,
            )
            .await
            .expect("raw epoch");
        connection.detach_all().await;
        assert_eq!(
            native
                .calls("Target.detachFromTarget")
                .expect("detach")
                .len(),
            4
        );
    }
}

#[cfg(test)]
mod lifetime_tests {
    use super::*;
    use crate::page::Native;

    #[tokio::test]
    async fn captcha_cleanup_ends_core_listeners_instead_of_retaining_per_solve_caches() {
        let native = Native::new("https://widget.test");
        let connection = Arc::new(SolveConnection::new(native));
        let weak = Arc::downgrade(&connection);
        let browser = Browser::new(connection.clone());
        connection.detach_all().await;
        assert!(
            connection
                .send("Page.enable", json!({}), None)
                .await
                .is_err()
        );
        drop(browser);
        drop(connection);
        for _ in 0..100 {
            if weak.upgrade().is_none() {
                return;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            weak.upgrade().is_none(),
            "core's event listeners must release their caches after solve cleanup"
        );
    }
}

#[cfg(test)]
mod event_ownership_tests {
    use super::*;

    #[test]
    fn captcha_core_events_cover_only_owned_sessions_and_their_children() {
        let sessions = Mutex::new(HashSet::from([SessionId::from("solver")]));
        let event = |method: &str, parent: Option<&str>, params| CdpEvent {
            method: method.to_owned(),
            session_id: parent.map(SessionId::from),
            params,
        };
        assert!(!owns_event(
            &sessions,
            &event(
                "Target.attachedToTarget",
                Some("runtime"),
                json!({"sessionId":"foreign-child"})
            )
        ));
        assert!(!owns_event(
            &sessions,
            &event(
                "Target.attachedToTarget",
                None,
                json!({"sessionId":"runtime"})
            )
        ));
        assert!(owns_event(
            &sessions,
            &event(
                "Target.attachedToTarget",
                Some("solver"),
                json!({"sessionId":"solver-child"})
            )
        ));
        assert!(owns_event(
            &sessions,
            &event("Page.frameNavigated", Some("solver-child"), json!({}))
        ));
        assert!(!owns_event(
            &sessions,
            &event("Page.frameNavigated", Some("foreign-child"), json!({}))
        ));
        assert!(owns_event(
            &sessions,
            &event(
                "Target.detachedFromTarget",
                None,
                json!({"sessionId":"solver-child"})
            )
        ));
    }
}

#[cfg(test)]
mod late_attach_tests {
    use super::*;
    use crate::page::Native;

    #[tokio::test(start_paused = true)]
    async fn captcha_cancelled_attach_wait_still_records_and_detaches_its_late_session() {
        let native = Native::new("https://widget.test");
        *native.attach_delay.lock().expect("attach delay") = Duration::from_secs(5);
        let connection = SolveConnection::new(native.clone());
        let began = tokio::time::Instant::now();
        assert!(
            tokio::time::timeout(
                Duration::from_secs(1),
                connection.send(
                    "Target.attachToTarget",
                    json!({"targetId":"page","flatten":true}),
                    None
                )
            )
            .await
            .is_err()
        );
        connection.detach_all().await;
        let detach = native.calls("Target.detachFromTarget").expect("detach");
        assert_eq!(detach.len(), 1);
        assert!(detach[0].3.duration_since(began) >= Duration::from_secs(5));
    }
}
