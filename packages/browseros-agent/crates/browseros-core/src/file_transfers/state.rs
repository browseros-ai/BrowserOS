//! Connection-owned chooser state. Protocol events outlive individual tool calls, so a click
//! and a later upload must share the same request without retaining a stale document handle.
use super::{FileChooser, FileChooserInterception};
use crate::{CdpConnection, CoreError, PageId, ProtocolSession, SessionId};
use browseros_cdp::{CdpError, CdpEvent};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use tokio::sync::{Mutex as AsyncMutex, broadcast, watch};

#[derive(Default)]
struct State {
    epoch: u64,
    next_chooser: u64,
    choosers: HashMap<SessionId, FileChooser>,
    intercepted: HashMap<SessionId, ProtocolSession>,
    /// Agent creation receipts own interception permanently, independently of borrowed leases.
    permanent: HashSet<SessionId>,
    parents: HashMap<SessionId, SessionId>,
    setup_errors: HashMap<SessionId, String>,
}

/// One connection's native transfer state, shared by event observation and scoped tool calls.
pub(crate) struct TransferState {
    connection: Arc<dyn CdpConnection>,
    state: Mutex<State>,
    changed: watch::Sender<u64>,
    pub(crate) download_contexts: AsyncMutex<HashSet<(u64, Option<String>)>>,
    /// Bookkeeping only, never held across an await. The owning tab's `TabTurns::leases`
    /// orders each change with the native setup or restore it triggers.
    leases: Mutex<HashMap<(u64, SessionId), BorrowedInterception>>,
    /// Keyed by each tab's root session; see `TransferState::turns`.
    turns: Mutex<HashMap<SessionId, Weak<TabTurns>>>,
    operations: Mutex<HashMap<PageId, Weak<AsyncMutex<()>>>>,
}

struct BorrowedInterception {
    owners: usize,
    restore: bool,
}

/// One tab's chooser turns, shared by its page session and every iframe session under it.
/// Setup and restore await that tab's renderer, bounded only by the 60 s CDP request timeout,
/// so the turns order one tab and never couple tabs. A connection-wide lock here would let a
/// busy tab's restore hold every agent tab's admission, cached sessions included, because each
/// admission promotes its tab through `intercept`. A caller holds one tab's turns at most,
/// `leases` before `setup`, so turns cannot deadlock.
#[derive(Default)]
struct TabTurns {
    /// Orders lease bookkeeping, and the native setup or restore it triggers, against other
    /// lease changes and permanent promotion on this tab. A newcomer queued behind a last-owner
    /// restore then sees the override gone and installs it again.
    leases: AsyncMutex<()>,
    /// Keeps setup, child backfill included, and restore of this subtree from interleaving.
    setup: AsyncMutex<()>,
}

impl TransferState {
    pub(crate) fn new(connection: Arc<dyn CdpConnection>) -> Arc<Self> {
        let (changed, _) = watch::channel(0);
        let state = Arc::new(Self {
            connection,
            state: Mutex::new(State::default()),
            changed,
            download_contexts: AsyncMutex::new(HashSet::new()),
            leases: Mutex::new(HashMap::new()),
            turns: Mutex::new(HashMap::new()),
            operations: Mutex::new(HashMap::new()),
        });
        // Install the subscriber synchronously, before any page can be attached or clicked.
        let mut events = state.connection.events();
        let weak = Arc::downgrade(&state);
        tokio::spawn(async move {
            loop {
                let event = events.recv().await;
                let Some(state) = weak.upgrade() else { break };
                match event {
                    Ok(event) => state.observe(event).await,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // A missed navigation could otherwise let an old chooser select files.
                        state.lock().choosers.clear();
                        state.notify();
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        state
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let epoch = self.connection.connection_epoch();
        if state.epoch != epoch {
            *state = State {
                epoch,
                ..Default::default()
            };
        }
        state
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.connection.connection_epoch()
    }

    fn leases(&self) -> MutexGuard<'_, HashMap<(u64, SessionId), BorrowedInterception>> {
        self.leases
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// The turns of the tab `session` belongs to, keyed by its root: the topmost ancestor in
    /// `parents`. Every session of one subtree must resolve to the same turns. A child iframe
    /// keyed by itself would install its override while its root's restore is in flight, and
    /// that override would outlive the cleanup.
    fn turns(&self, session: &SessionId) -> Arc<TabTurns> {
        let root = root_of(&self.lock().parents, session);
        let mut turns = self.turns.lock().unwrap_or_else(|error| error.into_inner());
        turns.retain(|_, turn| turn.strong_count() > 0);
        if let Some(turn) = turns.get(&root).and_then(Weak::upgrade) {
            return turn;
        }
        let turn = Arc::new(TabTurns::default());
        turns.insert(root, Arc::downgrade(&turn));
        turn
    }

    pub(super) async fn lease_interception(
        self: &Arc<Self>,
        session: &ProtocolSession,
    ) -> Result<FileChooserInterception, CoreError> {
        let id = chooser_session(session)?;
        let epoch = self.epoch();
        let turns = self.turns(id);
        let _leases = turns.leases.lock().await;
        {
            let mut leases = self.leases();
            // The epoch is read again here, under the bookkeeping lock. The turn awaited above
            // belongs to this tab alone, so a reconnect can land while this caller waits, and
            // other tabs may already hold leases of the new epoch. A stale caller takes no lease
            // and prunes nothing: pruning by its old epoch would delete another tab's live lease,
            // whose release would then find nothing to restore and leave that tab intercepted.
            let current = self.epoch();
            if current != epoch {
                return Err(CoreError::Cdp(CdpError::ConnectionEpochChanged {
                    expected: epoch,
                    actual: current,
                }));
            }
            leases.retain(|(known, _), _| *known >= current);
            let entry = leases
                .entry((epoch, id.clone()))
                .or_insert_with(|| BorrowedInterception {
                    owners: 0,
                    restore: !self.intercepts(id),
                });
            entry.owners += 1;
        }
        // Arm Drop before awaiting Chrome: cancellation can follow native application but
        // precede its acknowledgement. The retained lease also retries earlier setup errors.
        let lease = FileChooserInterception::new(Arc::clone(self), epoch, id.clone());
        self.intercept_tree(session, None).await?;
        Ok(lease)
    }

    pub(super) async fn release_interception(
        &self,
        epoch: u64,
        session: &SessionId,
    ) -> Result<(), CoreError> {
        let turns = self.turns(session);
        let _leases = turns.leases.lock().await;
        let key = (epoch, session.clone());
        {
            let mut leases = self.leases();
            if epoch != self.epoch() {
                leases.remove(&key);
                return Ok(());
            }
            let Some(entry) = leases.get_mut(&key) else {
                return Ok(());
            };
            if entry.owners > 0 {
                entry.owners -= 1;
            }
            if entry.owners > 0 {
                return Ok(());
            }
            if !entry.restore || self.lock().permanent.contains(session) {
                leases.remove(&key);
                return Ok(());
            }
        }
        // A failed cleanup remains at zero owners with restore=true. The next borrower
        // inherits that obligation rather than treating the leftover override as permanent.
        self.restore(session).await?;
        self.leases().remove(&key);
        Ok(())
    }

    /// Drops a session Chromium no longer has, whether its detach event or a refused command
    /// said so. Interception keeps per-session state like the page and frame registries do, so
    /// a session left here would be set up again, and refused, on every later attachment of
    /// its page. See `BrowserSession::forget_session`.
    pub(crate) fn forget_session(&self, id: &SessionId) {
        let mut state = self.lock();
        state.choosers.remove(id);
        state.intercepted.remove(id);
        state.permanent.remove(id);
        state.parents.remove(id);
        state.setup_errors.remove(id);
        drop(state);
        self.notify();
    }

    pub(crate) fn changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
    fn notify(&self) {
        self.changed
            .send_modify(|version| *version = version.wrapping_add(1));
    }

    pub(crate) fn operation(&self, page: PageId) -> Arc<AsyncMutex<()>> {
        let mut operations = self
            .operations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        operations.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = operations.get(&page).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(AsyncMutex::new(()));
        operations.insert(page, Arc::downgrade(&lock));
        lock
    }

    pub(crate) fn has_interception(&self) -> bool {
        !self.lock().intercepted.is_empty()
    }

    pub(crate) fn intercepts(&self, session: &SessionId) -> bool {
        self.lock().intercepted.contains_key(session)
    }

    pub(crate) fn has_pending(&self) -> bool {
        let state = self.lock();
        !state.choosers.is_empty() || !state.setup_errors.is_empty()
    }

    pub(crate) fn choosers(&self) -> Vec<FileChooser> {
        self.lock().choosers.values().cloned().collect()
    }

    pub(crate) fn clear(&self, chooser: &FileChooser) {
        let mut state = self.lock();
        if state
            .choosers
            .get(&chooser.session)
            .is_some_and(|current| current == chooser)
        {
            state.choosers.remove(&chooser.session);
        }
    }

    pub(crate) fn setup_error(&self, root: &SessionId) -> Option<String> {
        let state = self.lock();
        state
            .setup_errors
            .iter()
            .find(|(session, _)| belongs_to(&state.parents, session, root))
            .map(|(_, error)| error.clone())
    }

    /// Returns whether this call installed interception. A borrowed user page restores only
    /// its own lease; pages created by the agent keep interception for subsequent actions.
    pub(crate) async fn intercept(&self, session: &ProtocolSession) -> Result<bool, CoreError> {
        let id = chooser_session(session)?;
        // Promotion to permanent ownership is ordered with last-borrower restoration.
        let turns = self.turns(id);
        let _leases = turns.leases.lock().await;
        // A creation receipt can arrive while another caller already borrows this tab.
        // Its later last-lease cleanup must not undo the agent tab's permanent setup.
        self.lock().permanent.insert(id.clone());
        self.intercept_tree(session, None).await
    }

    async fn intercept_tree(
        &self,
        session: &ProtocolSession,
        parent: Option<&SessionId>,
    ) -> Result<bool, CoreError> {
        let id = chooser_session(session)?;
        // Setup and restoration share the tab's setup turn: an attachment queued during
        // cleanup must recheck its parent after cleanup, never install a late child override
        // behind it. A child resolves to its root's turns because `observe` records its parent
        // before calling here.
        let turns = self.turns(id);
        let _setup = turns.setup.lock().await;
        if parent.is_some_and(|parent| !self.lock().intercepted.contains_key(parent)) {
            return Ok(false);
        }
        let installed = {
            let state = self.lock();
            !state.intercepted.contains_key(id) || state.setup_errors.contains_key(id)
        };
        if installed {
            let cleanup = session.cancellation_endpoint();
            // Publish cleanup before awaiting the native acknowledgement. Cancellation can
            // drop this future after Chrome applies the override but before it replies.
            self.lock().intercepted.insert(id.clone(), cleanup.clone());
            let enabled = session
                .send_value(
                    "Page.setInterceptFileChooserDialog",
                    json!({"enabled":true}),
                )
                .await;
            // A post-dispatch admission failure may follow a successful native mutation.
            // Retain an epoch-pinned cleanup handle even on that error path.
            if let Err(error) = enabled {
                if session_gone(&error) {
                    // Nothing to clean up on a session that is gone. A recorded error would
                    // outlive it, since the page's next attachment is a new session.
                    self.forget_session(id);
                    return Err(error);
                }
                self.lock()
                    .setup_errors
                    .insert(id.clone(), error.to_string());
                if cleanup
                    .send_value(
                        "Page.setInterceptFileChooserDialog",
                        json!({"enabled":false}),
                    )
                    .await
                    .is_ok()
                {
                    let mut state = self.lock();
                    state.intercepted.remove(id);
                    state.setup_errors.remove(id);
                }
                return Err(error);
            }
        }
        self.lock().setup_errors.remove(id);
        let children = {
            let state = self.lock();
            state
                .parents
                .keys()
                .filter(|child| {
                    belongs_to(&state.parents, child, id)
                        && (!state.intercepted.contains_key(*child)
                            || state.setup_errors.contains_key(*child))
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        // The background registry may have attached these before a creation receipt marked
        // the root agent-owned. Backfill that whole known subtree before exposing the root.
        for child in children {
            let endpoint = ProtocolSession::for_session(self.connection.clone(), child.clone())
                .with_epoch_guard(session.cancellation_endpoint().epoch_guard().cloned());
            // A child frame whose session is gone holds no chooser, so it is forgotten rather
            // than failing its page: left in place, it would fail every later attachment.
            match endpoint.send_value("Page.enable", json!({})).await {
                Err(error) if session_gone(&error) => {
                    self.forget_session(&child);
                    continue;
                }
                result => result?,
            };
            self.lock()
                .intercepted
                .insert(child.clone(), endpoint.clone());
            if let Err(error) = endpoint
                .send_value(
                    "Page.setInterceptFileChooserDialog",
                    json!({"enabled":true}),
                )
                .await
            {
                if session_gone(&error) {
                    self.forget_session(&child);
                    continue;
                }
                self.lock().setup_errors.insert(child, error.to_string());
                return Err(error);
            }
            self.lock().setup_errors.remove(&child);
        }
        self.lock().setup_errors.remove(id);
        Ok(installed)
    }

    pub(crate) async fn restore(&self, root: &SessionId) -> Result<(), CoreError> {
        let turns = self.turns(root);
        let _setup = turns.setup.lock().await;
        let sessions = {
            let state = self.lock();
            state
                .intercepted
                .iter()
                .filter(|(id, _)| belongs_to(&state.parents, id, root))
                .map(|(id, endpoint)| (id.clone(), endpoint.cancellation_endpoint()))
                .collect::<Vec<_>>()
        };
        let mut first_error = None;
        for (id, endpoint) in sessions {
            // Restoring our own override remains authorized after a tab moves out of scope;
            // the cancellation endpoint keeps its original socket epoch/session identity.
            match endpoint
                .send_value(
                    "Page.setInterceptFileChooserDialog",
                    json!({"enabled":false}),
                )
                .await
            {
                Ok(_) => {
                    let mut state = self.lock();
                    state.choosers.remove(&id);
                    state.intercepted.remove(&id);
                    state.setup_errors.remove(&id);
                }
                Err(error) => {
                    let mut state = self.lock();
                    // A detached/reconnected endpoint is already retired. Otherwise retain
                    // both its cleanup handle and failure so later interactions cannot hide it.
                    if state.intercepted.contains_key(&id) {
                        state.setup_errors.insert(id, error.to_string());
                        if first_error.is_none() {
                            first_error = Some(error);
                        }
                    }
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn observe(&self, event: CdpEvent) {
        match event.method.as_str() {
            "Page.fileChooserOpened" => {
                if let (Some(session), Some(frame)) =
                    (event.session_id, event.params["frameId"].as_str())
                {
                    let session = SessionId::from(session);
                    let mut state = self.lock();
                    state.next_chooser = state.next_chooser.wrapping_add(1);
                    let chooser = FileChooser {
                        epoch: state.epoch,
                        request: state.next_chooser,
                        session: session.clone(),
                        frame: crate::FrameId(frame.into()),
                        backend_node_id: event.params["backendNodeId"].as_i64(),
                        multiple: event.params["mode"] == "selectMultiple",
                    };
                    state.choosers.insert(session, chooser);
                    drop(state);
                    self.notify();
                }
            }
            "Page.frameNavigated" | "Page.frameDetached" => {
                let frame = event.params["frame"]["id"]
                    .as_str()
                    .or_else(|| event.params["frameId"].as_str());
                let mut state = self.lock();
                // Root navigation also invalidates choosers in remote descendants, whose
                // detach notifications can arrive after the new document becomes actionable.
                let root_navigation = event.method == "Page.frameNavigated"
                    && event.params["frame"].get("parentId").is_none();
                let invalid = state
                    .choosers
                    .iter()
                    .filter(|(session, chooser)| {
                        frame == Some(chooser.frame.as_str())
                            || (root_navigation
                                && event.session_id.as_ref().is_some_and(|root| {
                                    belongs_to(
                                        &state.parents,
                                        session,
                                        &SessionId::from(root.as_str()),
                                    )
                                }))
                    })
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();
                for id in invalid {
                    state.choosers.remove(&id);
                }
                drop(state);
                self.notify();
            }
            "Target.detachedFromTarget" => {
                if let Some(id) = event.params["sessionId"].as_str() {
                    self.forget_session(&SessionId::from(id));
                }
            }
            "Target.attachedToTarget" if event.params["targetInfo"]["type"] == "iframe" => {
                if let (Some(parent), Some(child)) =
                    (event.session_id, event.params["sessionId"].as_str())
                {
                    let parent = SessionId::from(parent);
                    let child = SessionId::from(child);
                    let parent_session = {
                        let mut state = self.lock();
                        state.parents.insert(child.clone(), parent.clone());
                        state.intercepted.get(&parent).cloned()
                    };
                    if let Some(parent_session) = parent_session {
                        let session =
                            ProtocolSession::for_session(self.connection.clone(), child.clone())
                                .with_epoch_guard(parent_session.epoch_guard().cloned());
                        let setup = async {
                            session.send_value("Page.enable", json!({})).await?;
                            self.intercept_tree(&session, Some(&parent))
                                .await
                                .map(|_| ())
                        }
                        .await;
                        match setup {
                            Ok(()) => {}
                            // The frame is gone, so there is no failure left to report.
                            Err(error) if session_gone(&error) => self.forget_session(&child),
                            Err(error) => {
                                // Keep the failure observable to the next scoped interaction.
                                self.lock().setup_errors.insert(child, error.to_string());
                                self.notify();
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn session_gone(error: &CoreError) -> bool {
    matches!(error, CoreError::Cdp(CdpError::SessionGone))
}

fn chooser_session(session: &ProtocolSession) -> Result<&SessionId, CoreError> {
    session
        .session_id()
        .ok_or_else(|| CoreError::Message("A file chooser needs a page session".into()))
}

/// The topmost ancestor of `session` in `parents`, walked with the same bound as `belongs_to`.
fn root_of(parents: &HashMap<SessionId, SessionId>, session: &SessionId) -> SessionId {
    let mut current = session;
    for _ in 0..64 {
        let Some(parent) = parents.get(current) else {
            break;
        };
        current = parent;
    }
    current.clone()
}

fn belongs_to(
    parents: &HashMap<SessionId, SessionId>,
    session: &SessionId,
    root: &SessionId,
) -> bool {
    let mut current = session;
    for _ in 0..64 {
        if current == root {
            return true;
        }
        let Some(parent) = parents.get(current) else {
            return false;
        };
        current = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::TransferState;
    use crate::{CdpConnection, CoreError, ProtocolSession, SessionId};
    use browseros_cdp::{CdpError, CdpEvent, TargetedEvents};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use std::error::Error;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;
    use tokio::sync::{broadcast, mpsc, watch};

    type TestResult = Result<(), Box<dyn Error>>;

    /// One command the fake browser received, with the session it was sent on.
    struct Call {
        method: String,
        params: Value,
        session: Option<SessionId>,
    }

    /// Answers every command, except that Chromium refuses each one sent on `gone` as gone, and
    /// one sent on the hung session waits until it answers, as a busy renderer's does. Every
    /// command is recorded when it is sent, before any answer.
    struct Browser {
        gone: SessionId,
        hung: watch::Sender<Option<SessionId>>,
        calls: watch::Sender<Vec<Call>>,
        events: broadcast::Sender<CdpEvent>,
        targeted: TargetedEvents,
        epoch: AtomicU64,
    }

    impl Browser {
        fn new(gone: &str) -> Arc<Self> {
            Arc::new(Self {
                gone: SessionId::from(gone),
                hung: watch::channel(None).0,
                calls: watch::channel(Vec::new()).0,
                events: broadcast::channel(16).0,
                targeted: TargetedEvents::default(),
                epoch: AtomicU64::new(1),
            })
        }

        /// The socket reconnects: every later command and lease belongs to the next epoch.
        fn reconnect(&self) {
            self.epoch.fetch_add(1, Ordering::SeqCst);
        }

        /// Commands on `session`, those already waiting included, go unanswered until `answer`.
        fn hang(&self, session: &str) {
            self.hung.send_replace(Some(SessionId::from(session)));
        }

        fn answer(&self) {
            self.hung.send_replace(None);
        }

        /// Waits, for 5 s at most, until the commands sent so far satisfy `sent`.
        async fn until(&self, what: &str, sent: impl Fn(&[Call]) -> bool) -> TestResult {
            let mut calls = self.calls.subscribe();
            tokio::time::timeout(
                Duration::from_secs(5),
                calls.wait_for(|calls| sent(calls.as_slice())),
            )
            .await
            .map_err(|_| format!("never sent: {what}"))??;
            Ok(())
        }

        fn overrides(&self, session: &str) -> Vec<bool> {
            overrides(self.calls.borrow().as_slice(), session)
        }

        /// Offers `event` to its method's targeted subscribers and broadcasts it only when none
        /// took it, as `CdpClient` does. Fails when the broadcast is needed and no one reads it.
        fn emit(&self, event: CdpEvent) -> Result<(), broadcast::error::SendError<CdpEvent>> {
            if let Some(event) = self.targeted.deliver(event) {
                self.events.send(event)?;
            }
            Ok(())
        }
    }

    /// The `enabled` value of every chooser override sent on `session`, in sending order.
    fn overrides(calls: &[Call], session: &str) -> Vec<bool> {
        calls
            .iter()
            .filter(|call| {
                call.method == "Page.setInterceptFileChooserDialog"
                    && call
                        .session
                        .as_ref()
                        .is_some_and(|id| id.as_str() == session)
            })
            .filter_map(|call| call.params["enabled"].as_bool())
            .collect()
    }

    fn sent_on(calls: &[Call], method: &str, session: &str) -> bool {
        calls.iter().any(|call| {
            call.method == method
                && call
                    .session
                    .as_ref()
                    .is_some_and(|id| id.as_str() == session)
        })
    }

    impl CdpConnection for Browser {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            self.calls.send_modify(|calls| {
                calls.push(Call {
                    method: method.to_owned(),
                    params,
                    session: session.cloned(),
                });
            });
            let gone = session == Some(&self.gone);
            let mut hung = self.hung.subscribe();
            Box::pin(async move {
                // The sender lives as long as the fake, so this ends only when the session answers.
                if hung
                    .wait_for(|hung| hung.as_ref() != session)
                    .await
                    .is_err()
                {
                    return Err(CdpError::ConnectionLost);
                }
                if gone {
                    Err(CdpError::SessionGone)
                } else {
                    Ok(json!({}))
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_string()) })
        }

        fn send_at_epoch<'a>(
            &'a self,
            _expected_epoch: u64,
            method: &'a str,
            params: Value,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            self.send(method, params, session)
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            self.events.subscribe()
        }

        fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
            self.targeted.subscribe(method)
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            self.epoch.load(Ordering::SeqCst)
        }
    }

    /// A lease that waited on its own tab's turn across a reconnect neither takes a lease nor
    /// prunes the new epoch's: another tab's lease taken meanwhile survives, and its release
    /// still restores that tab's native picker.
    #[tokio::test]
    async fn tests_that_a_lease_stale_after_a_reconnect_leaves_another_tabs_new_lease() -> TestResult
    {
        let browser = Browser::new("gone");
        let transfers = TransferState::new(browser.clone());
        let tab_a = ProtocolSession::for_session(browser.clone(), SessionId::from("a"));
        let tab_b = ProtocolSession::for_session(browser.clone(), SessionId::from("b"));

        // Tab A's first lease holds A's turn across a setup its busy renderer leaves unanswered,
        // and a second lease of A waits behind it, its epoch read before the reconnect.
        browser.hang("a");
        let first = tokio::spawn({
            let (transfers, tab_a) = (Arc::clone(&transfers), tab_a.clone());
            async move { transfers.lease_interception(&tab_a).await }
        });
        browser
            .until("tab A's setup", |calls| overrides(calls, "a") == [true])
            .await?;
        let stale = tokio::spawn({
            let (transfers, tab_a) = (Arc::clone(&transfers), tab_a.clone());
            async move { transfers.lease_interception(&tab_a).await }
        });
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        browser.reconnect();
        let mut lease_b = transfers.lease_interception(&tab_b).await?;
        assert_eq!(
            browser.overrides("b"),
            [true],
            "tab B is intercepted at the new epoch"
        );

        browser.answer();
        let _first = first.await??;
        let stale = stale.await?;
        lease_b.release().await?;
        assert_eq!(
            browser.overrides("b"),
            [true, false],
            "tab B's lease survived the stale caller, so its release restores the native picker"
        );
        assert!(
            stale.is_err(),
            "the lease whose epoch went stale while it waited takes nothing"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_child_frame_gone_during_setup_never_blocks_its_page()
    -> Result<(), CoreError> {
        let browser = Browser::new("child");
        let transfers = TransferState::new(browser.clone());
        let root = ProtocolSession::for_session(browser.clone(), SessionId::from("root"));
        transfers.intercept(&root).await?;

        // The child iframe attaches under the intercepted page and dies before its own setup.
        let mut changes = transfers.changes();
        browser
            .emit(CdpEvent {
                method: "Target.attachedToTarget".into(),
                params: json!({"sessionId":"child","targetInfo":{"type":"iframe"}}),
                session_id: Some(SessionId::from("root")),
            })
            .map_err(|error| CoreError::Message(error.to_string()))?;
        tokio::time::timeout(Duration::from_secs(5), changes.changed())
            .await
            .map_err(|_| CoreError::Message("the attach was never observed".into()))?
            .map_err(|error| CoreError::Message(error.to_string()))?;

        transfers.intercept(&root).await?;
        assert!(!transfers.has_pending());
        Ok(())
    }

    /// Admits an agent tab as a cached session does, by promoting it through `intercept`,
    /// within 1 s: long enough for any healthy admission, far below a hung renderer's 60 s.
    async fn admit(transfers: &TransferState, browser: &Arc<Browser>, tab: &str) -> TestResult {
        let tab = ProtocolSession::for_session(browser.clone(), SessionId::from(tab));
        tokio::time::timeout(Duration::from_secs(1), transfers.intercept(&tab))
            .await
            .map_err(|_| "the agent tab's admission waited on another tab")??;
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_another_tabs_hung_restore_never_holds_an_agent_tabs_admission() -> TestResult
    {
        let browser = Browser::new("gone");
        let transfers = TransferState::new(browser.clone());
        let busy = ProtocolSession::for_session(browser.clone(), SessionId::from("busy"));
        let mut lease = transfers.lease_interception(&busy).await?;

        // The borrowed tab's renderer stops answering while its last lease restores the picker.
        browser.hang("busy");
        let release = tokio::spawn(async move { lease.release().await });
        browser
            .until("the busy tab's restore", |calls| {
                overrides(calls, "busy") == [true, false]
            })
            .await?;

        admit(&transfers, &browser, "healthy").await?;
        assert_eq!(browser.overrides("healthy"), [true]);
        browser.answer();
        release.await??;
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_another_tabs_hung_setup_never_holds_an_agent_tabs_admission() -> TestResult
    {
        let browser = Browser::new("gone");
        let transfers = TransferState::new(browser.clone());
        browser.hang("busy");
        let lease = tokio::spawn({
            let transfers = transfers.clone();
            let busy = ProtocolSession::for_session(browser.clone(), SessionId::from("busy"));
            async move { transfers.lease_interception(&busy).await }
        });
        browser
            .until("the busy tab's setup", |calls| {
                overrides(calls, "busy") == [true]
            })
            .await?;

        admit(&transfers, &browser, "healthy").await?;
        assert_eq!(browser.overrides("healthy"), [true]);
        browser.answer();
        lease.await??.release().await?;
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_new_lease_waits_for_its_tabs_restore_then_installs_the_override_again()
    -> TestResult {
        let browser = Browser::new("gone");
        let transfers = TransferState::new(browser.clone());
        let tab = ProtocolSession::for_session(browser.clone(), SessionId::from("busy"));
        let mut first = transfers.lease_interception(&tab).await?;
        browser.hang("busy");
        let restore = tokio::spawn(async move { first.release().await });
        browser
            .until("the last owner's restore", |calls| {
                overrides(calls, "busy") == [true, false]
            })
            .await?;

        let mut second = tokio::spawn({
            let transfers = transfers.clone();
            let tab = tab.clone();
            async move { transfers.lease_interception(&tab).await }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut second)
                .await
                .is_err(),
            "a new lease must wait while its tab's last owner restores the picker"
        );
        browser.answer();
        restore.await??;
        let mut second = second.await??;
        assert_eq!(
            browser.overrides("busy"),
            [true, false, true],
            "the newcomer finds the override gone and installs it again"
        );
        assert!(transfers.intercepts(&SessionId::from("busy")));
        second.release().await?;
        assert_eq!(
            browser.overrides("busy"),
            [true, false, true, false],
            "the newcomer's own lease still restores the picker when it ends"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_child_frame_attached_during_its_tabs_restore_gets_no_late_override()
    -> TestResult {
        let browser = Browser::new("gone");
        let transfers = TransferState::new(browser.clone());
        let tab = ProtocolSession::for_session(browser.clone(), SessionId::from("busy"));
        let mut lease = transfers.lease_interception(&tab).await?;
        browser.hang("busy");
        let restore = tokio::spawn(async move { lease.release().await });
        browser
            .until("the tab's restore", |calls| {
                overrides(calls, "busy") == [true, false]
            })
            .await?;

        // The iframe attaches while its parent still reads as intercepted. Its setup sends
        // Page.enable, then must queue behind the restore and find the parent restored.
        browser.emit(CdpEvent {
            method: "Target.attachedToTarget".into(),
            params: json!({"sessionId":"child","targetInfo":{"type":"iframe"}}),
            session_id: Some(SessionId::from("busy")),
        })?;
        browser
            .until("the child's Page.enable", |calls| {
                sent_on(calls, "Page.enable", "child")
            })
            .await?;
        tokio::task::yield_now().await;
        browser.answer();
        restore.await??;

        // Events are observed one at a time, so this detach's notification follows the attach.
        let mut changes = transfers.changes();
        browser.emit(CdpEvent {
            method: "Target.detachedFromTarget".into(),
            params: json!({"sessionId":"after-attach"}),
            session_id: None,
        })?;
        tokio::time::timeout(Duration::from_secs(5), changes.changed())
            .await
            .map_err(|_| "the detach after the attach was never observed")??;
        assert!(
            browser.overrides("child").is_empty(),
            "a child queued behind its tab's restore installs no override"
        );
        assert!(!transfers.intercepts(&SessionId::from("child")));
        Ok(())
    }

    #[test]
    fn tests_that_an_emitted_event_reaches_its_targeted_subscriber_and_not_the_broadcast()
    -> Result<(), Box<dyn Error>> {
        let browser = Browser::new("child");
        let mut broadcast = browser.events();
        let mut targeted = browser.events_targeted("Runtime.bindingCalled");
        browser.emit(CdpEvent {
            method: "Runtime.bindingCalled".into(),
            params: json!({ "name": "probe" }),
            session_id: Some(SessionId::from("root")),
        })?;
        assert_eq!(targeted.try_recv()?.params["name"], "probe");
        assert!(
            broadcast.try_recv().is_err(),
            "a method with a live targeted subscriber must bypass the broadcast"
        );
        Ok(())
    }
}
