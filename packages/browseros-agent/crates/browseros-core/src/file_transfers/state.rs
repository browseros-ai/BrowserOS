//! Connection-owned chooser state. Protocol events outlive individual tool calls, so a click
//! and a later upload must share the same request without retaining a stale document handle.
use super::FileChooser;
use crate::{CdpConnection, CoreError, PageId, ProtocolSession, SessionId};
use browseros_cdp::CdpEvent;
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard, Weak},
};
use tokio::sync::{Mutex as AsyncMutex, broadcast, watch};

#[derive(Default)]
struct State {
    epoch: u64,
    next_chooser: u64,
    choosers: HashMap<SessionId, FileChooser>,
    intercepted: HashMap<SessionId, ProtocolSession>,
    parents: HashMap<SessionId, SessionId>,
    setup_errors: HashMap<SessionId, String>,
}

/// One connection's native transfer state, shared by event observation and scoped tool calls.
pub(crate) struct TransferState {
    connection: Arc<dyn CdpConnection>,
    state: Mutex<State>,
    changed: watch::Sender<u64>,
    pub(crate) download_contexts: AsyncMutex<HashSet<(u64, Option<String>)>>,
    interception: AsyncMutex<()>,
    operations: Mutex<HashMap<PageId, Weak<AsyncMutex<()>>>>,
}

impl TransferState {
    pub(crate) fn new(connection: Arc<dyn CdpConnection>) -> Arc<Self> {
        let (changed, _) = watch::channel(0);
        let state = Arc::new(Self {
            connection,
            state: Mutex::new(State::default()),
            changed,
            download_contexts: AsyncMutex::new(HashSet::new()),
            interception: AsyncMutex::new(()),
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
        self.intercept_tree(session, None).await
    }

    async fn intercept_tree(
        &self,
        session: &ProtocolSession,
        parent: Option<&SessionId>,
    ) -> Result<bool, CoreError> {
        // Setup and restoration share a lock: an attachment queued during cleanup must
        // recheck its parent after cleanup, never install a late child override behind it.
        let _setup = self.interception.lock().await;
        if parent.is_some_and(|parent| !self.lock().intercepted.contains_key(parent)) {
            return Ok(false);
        }
        let id = session
            .session_id()
            .ok_or_else(|| CoreError::Message("A file chooser needs a page session".into()))?;
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
            endpoint.send_value("Page.enable", json!({})).await?;
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
                self.lock().setup_errors.insert(child, error.to_string());
                return Err(error);
            }
            self.lock().setup_errors.remove(&child);
        }
        self.lock().setup_errors.remove(id);
        Ok(installed)
    }

    pub(crate) async fn restore(&self, root: &SessionId) -> Result<(), CoreError> {
        let _setup = self.interception.lock().await;
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
                    let id = SessionId::from(id);
                    let mut state = self.lock();
                    state.choosers.remove(&id);
                    state.intercepted.remove(&id);
                    state.parents.remove(&id);
                    state.setup_errors.remove(&id);
                    drop(state);
                    self.notify();
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
                        if let Err(error) = setup {
                            // Keep the failure observable to the next scoped interaction.
                            self.lock().setup_errors.insert(child, error.to_string());
                            self.notify();
                        }
                    }
                }
            }
            _ => {}
        }
    }
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
