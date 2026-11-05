//! Which pixels Neo may capture. A private session gets no more screenshots, frames or live
//! views. A document hold blocks one page while the document a secret was typed into is still
//! loaded. Privacy is sticky and durable: a private session stays private across restarts.
//!
//! Ordering is the guarantee. Making a session private sets the flag in memory and bumps its
//! epoch first, then commits the flag durably, and only then returns. Only after that does a
//! protected fill type or an owner's display open. Every capture holds a [`CaptureGuard`] taken
//! when it started, and calls [`CaptureGuard::admit`] once its pixels arrive:
//!
//! ```text
//!  make_private:  flag + epoch bump ──▶ durable commit ──▶ return ──▶ first keystroke
//!  a capture:     guard (epochs) ──▶ CDP capture ──▶ pixels arrive ──▶ admit? store : drop
//! ```
//!
//! Pixels that could show the secret are produced after the first keystroke, which comes after
//! the bump. They arrive after they are produced, so `admit` sees the new epoch and drops them.
//! Pixels a guard admits were produced before the bump. That is why visuals from before a
//! session went private are kept, and why nothing here waits for in-flight captures.

use crate::{EpochMillis, SecretSet, SessionId, TraceError};
use futures_util::future::BoxFuture;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Why a session went private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivateReason {
    /// A protected fill typed a secret (`[privacy] after_protected_fill = "session"`).
    SecretInput,
    /// A takeover entered `control`: the owner may type anything.
    Takeover,
    /// A control display forwarded input, or the session started while one was open.
    DisplayInput,
    /// The host asked (`POST /v1/sessions/{id}/private`).
    Host,
}

impl PrivateReason {
    /// The stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SecretInput => "secret_input",
            Self::Takeover => "takeover",
            Self::DisplayInput => "display_input",
            Self::Host => "host",
        }
    }

    /// Reads a stored name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "secret_input" => Some(Self::SecretInput),
            "takeover" => Some(Self::Takeover),
            "display_input" => Some(Self::DisplayInput),
            "host" => Some(Self::Host),
            _ => None,
        }
    }
}

/// When and why a session went private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivacyRecord {
    /// No pixel at or after this time is captured.
    pub since: EpochMillis,
    /// Why.
    pub reason: PrivateReason,
}

/// A browser page, by its DevTools target id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PageKey(pub String);

/// One document in a page, by its loader id. A navigation commits a new one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DocumentKey(pub String);

/// Where private flags are made durable. The trace writer implements it, so a flag is committed
/// in order with the records around it.
pub trait PrivacyStore: Send + Sync {
    /// Commits `session`'s flag. Returns once it is on disk.
    fn persist_private(
        &self,
        session: SessionId,
        record: PrivacyRecord,
    ) -> BoxFuture<'_, Result<(), TraceError>>;
}

/// Privacy could not be committed. Callers refuse the action: the fill is not sent, and the
/// control is not given.
#[derive(Debug, thiserror::Error)]
pub enum PrivacyError {
    /// The flag is set in memory (fail closed) but not on disk.
    #[error("the private flag could not be committed: {0}")]
    NotCommitted(#[source] TraceError),
}

/// A session's flag. It refuses captures from the moment it is set; `committed` says whether
/// it is also on disk, which is what `make_private` waits for before it returns.
#[derive(Clone, Copy)]
struct PrivateFlag {
    record: PrivacyRecord,
    committed: bool,
}

#[derive(Default)]
struct State {
    private: HashMap<SessionId, PrivateFlag>,
    session_epochs: HashMap<SessionId, u64>,
    page_epochs: HashMap<PageKey, u64>,
    holds: HashMap<PageKey, DocumentKey>,
    control_displays: usize,
}

/// Pixel privacy for the whole process, and the door to the secret set.
pub struct Privacy {
    secrets: Arc<SecretSet>,
    store: Arc<dyn PrivacyStore>,
    state: Mutex<State>,
}

impl Privacy {
    /// Starts from the flags the store loaded, so a restart keeps every private session private.
    #[must_use]
    pub fn new(
        secrets: Arc<SecretSet>,
        store: Arc<dyn PrivacyStore>,
        loaded: HashMap<SessionId, PrivacyRecord>,
    ) -> Arc<Self> {
        Arc::new(Self {
            secrets,
            store,
            state: Mutex::new(State {
                private: loaded
                    .into_iter()
                    .map(|(session, record)| {
                        let flag = PrivateFlag {
                            record,
                            committed: true,
                        };
                        (session, flag)
                    })
                    .collect(),
                ..State::default()
            }),
        })
    }

    /// The process-wide secret set this privacy masks text with.
    #[must_use]
    pub fn secrets(&self) -> &Arc<SecretSet> {
        &self.secrets
    }

    /// Makes `session` private, durably. Returns only once the flag is on disk, so a caller that
    /// gets `Ok` may type a secret. An already private session keeps its first record. A flag
    /// that is set but not yet committed (another call is committing it, or an earlier commit
    /// failed) is committed again here rather than trusted. The commit is an idempotent upsert.
    ///
    /// # Errors
    /// When the flag could not be committed. It still holds in memory, so captures stay refused.
    pub async fn make_private(
        &self,
        session: SessionId,
        reason: PrivateReason,
        at: EpochMillis,
    ) -> Result<(), PrivacyError> {
        let record = {
            let mut state = self.lock();
            match state.private.get(&session) {
                Some(flag) if flag.committed => return Ok(()),
                Some(flag) => flag.record,
                None => {
                    let record = PrivacyRecord { since: at, reason };
                    state.private.insert(
                        session,
                        PrivateFlag {
                            record,
                            committed: false,
                        },
                    );
                    *state.session_epochs.entry(session).or_default() += 1;
                    record
                }
            }
        };
        self.store
            .persist_private(session, record)
            .await
            .map_err(PrivacyError::NotCommitted)?;
        if let Some(flag) = self.lock().private.get_mut(&session) {
            flag.committed = true;
        }
        Ok(())
    }

    /// The session's privacy, when it is private.
    #[must_use]
    pub fn is_private(&self, session: SessionId) -> Option<PrivacyRecord> {
        self.lock().private.get(&session).map(|flag| flag.record)
    }

    /// Blocks captures of `page` while `document` is loaded in it. This is
    /// `after_protected_fill = "document"`.
    pub fn hold_document(&self, page: PageKey, document: DocumentKey) {
        let mut state = self.lock();
        *state.page_epochs.entry(page.clone()).or_default() += 1;
        state.holds.insert(page, document);
    }

    /// `page` committed `document`. A hold on any other document of that page ends.
    pub fn document_committed(&self, page: &PageKey, document: &DocumentKey) {
        let mut state = self.lock();
        if state.holds.get(page).is_some_and(|held| held != document) {
            state.holds.remove(page);
        }
    }

    /// A page closed: its hold ends. Its epoch moves on and is kept, never removed. A guard begun
    /// before the close then can never be admitted, even one a hold had already invalidated
    /// (a missing epoch would read as 0 and match it again). Only held pages have an entry.
    pub fn page_closed(&self, page: &PageKey) {
        let mut state = self.lock();
        state.holds.remove(page);
        if let Some(epoch) = state.page_epochs.get_mut(page) {
            *epoch += 1;
        }
    }

    /// Starts a capture of `page` for `session`, or `None` when it is not allowed right now.
    #[must_use]
    pub fn capture(self: &Arc<Self>, session: SessionId, page: PageKey) -> Option<CaptureGuard> {
        let state = self.lock();
        if state.private.contains_key(&session) || state.holds.contains_key(&page) {
            return None;
        }
        let session_epoch = state.session_epochs.get(&session).copied().unwrap_or(0);
        let page_epoch = state.page_epochs.get(&page).copied().unwrap_or(0);
        drop(state);
        Some(CaptureGuard {
            privacy: Arc::clone(self),
            session,
            page,
            session_epoch,
            page_epoch,
        })
    }

    /// A control display opened. Until it closes, every session that starts is private.
    #[must_use]
    pub fn control_display_opened(self: &Arc<Self>) -> ControlDisplay {
        self.lock().control_displays += 1;
        ControlDisplay {
            privacy: Arc::clone(self),
            input_seen: tokio::sync::Mutex::new(false),
        }
    }

    /// A session started. While a control display is open it starts private (the decided taint
    /// rule).
    ///
    /// # Errors
    /// When the flag could not be committed.
    pub async fn session_started(
        &self,
        session: SessionId,
        at: EpochMillis,
    ) -> Result<(), PrivacyError> {
        if self.lock().control_displays == 0 {
            return Ok(());
        }
        self.make_private(session, PrivateReason::DisplayInput, at)
            .await
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One capture's claim. Check [`CaptureGuard::admit`] after the pixels arrive and before they
/// are stored or forwarded.
pub struct CaptureGuard {
    privacy: Arc<Privacy>,
    session: SessionId,
    page: PageKey,
    session_epoch: u64,
    page_epoch: u64,
}

impl CaptureGuard {
    /// Whether the pixels this capture produced may be kept: nothing about the session's or the
    /// page's privacy changed since the capture began.
    #[must_use]
    pub fn admit(&self) -> bool {
        let state = self.privacy.lock();
        !state.private.contains_key(&self.session)
            && !state.holds.contains_key(&self.page)
            && state
                .session_epochs
                .get(&self.session)
                .copied()
                .unwrap_or(0)
                == self.session_epoch
            && state.page_epochs.get(&self.page).copied().unwrap_or(0) == self.page_epoch
    }
}

/// An open control display. Dropping it closes it.
pub struct ControlDisplay {
    privacy: Arc<Privacy>,
    /// Held across the commits of the first input, so a second input waits for them. It turns
    /// true only after every commit succeeded; a failed or cancelled first input leaves it false
    /// and the next input tries again.
    input_seen: tokio::sync::Mutex<bool>,
}

impl ControlDisplay {
    /// An input event is about to be forwarded. Before the first one, every session live now
    /// becomes private, durably. Concurrent inputs wait for that, and later calls return at once.
    ///
    /// # Errors
    /// When a flag could not be committed. The caller drops the input; the next one retries.
    pub async fn first_input(
        &self,
        live: &[SessionId],
        at: EpochMillis,
    ) -> Result<(), PrivacyError> {
        let mut seen = self.input_seen.lock().await;
        if *seen {
            return Ok(());
        }
        for session in live {
            self.privacy
                .make_private(*session, PrivateReason::DisplayInput, at)
                .await?;
        }
        *seen = true;
        Ok(())
    }
}

impl Drop for ControlDisplay {
    fn drop(&mut self) {
        let mut state = self.privacy.lock();
        state.control_displays = state.control_displays.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{DocumentKey, PageKey, Privacy, PrivacyRecord, PrivacyStore, PrivateReason};
    use crate::{EpochMillis, SecretSet, SecretSetConfig, SessionId, TraceError};
    use futures_util::future::BoxFuture;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Store {
        committed: Mutex<Vec<(SessionId, PrivacyRecord)>>,
        fail: AtomicBool,
        /// When set, a commit waits for a permit: the test holds a commit open.
        gate: Option<tokio::sync::Semaphore>,
    }

    impl PrivacyStore for Store {
        fn persist_private(
            &self,
            session: SessionId,
            record: PrivacyRecord,
        ) -> BoxFuture<'_, Result<(), TraceError>> {
            Box::pin(async move {
                if let Some(gate) = &self.gate {
                    gate.acquire()
                        .await
                        .map_err(|_| TraceError::Closed)?
                        .forget();
                }
                if self.fail.load(Ordering::SeqCst) {
                    return Err(TraceError::Closed);
                }
                self.committed
                    .lock()
                    .map_err(|_| TraceError::Closed)?
                    .push((session, record));
                Ok(())
            })
        }
    }

    fn privacy() -> (Arc<Privacy>, Arc<Store>) {
        with_store(Store::default())
    }

    fn with_store(store: Store) -> (Arc<Privacy>, Arc<Store>) {
        let store = Arc::new(store);
        let privacy = Privacy::new(
            Arc::new(SecretSet::new(SecretSetConfig::default())),
            store.clone(),
            HashMap::new(),
        );
        (privacy, store)
    }

    fn page(name: &str) -> PageKey {
        PageKey(name.to_string())
    }

    #[tokio::test]
    async fn a_private_session_refuses_new_captures_and_commits_once() {
        let (privacy, store) = privacy();
        let session = SessionId::new();
        assert!(privacy.capture(session, page("t1")).is_some());
        let at = EpochMillis::from_millis(1_000);
        assert!(
            privacy
                .make_private(session, PrivateReason::SecretInput, at)
                .await
                .is_ok()
        );
        assert!(
            privacy
                .make_private(session, PrivateReason::Host, at)
                .await
                .is_ok()
        );
        assert!(privacy.capture(session, page("t1")).is_none());
        assert_eq!(
            privacy.is_private(session),
            Some(PrivacyRecord {
                since: at,
                reason: PrivateReason::SecretInput
            })
        );
        assert_eq!(store.committed.lock().map(|c| c.len()).unwrap_or(0), 1);
    }

    #[tokio::test]
    async fn a_capture_begun_before_the_session_went_private_is_not_admitted() {
        let (privacy, _) = privacy();
        let session = SessionId::new();
        let other = SessionId::new();
        let before = privacy.capture(session, page("t1"));
        let unrelated = privacy.capture(other, page("t2"));
        assert!(before.as_ref().is_some_and(|guard| guard.admit()));
        let _ = privacy
            .make_private(
                session,
                PrivateReason::Takeover,
                EpochMillis::from_millis(5),
            )
            .await;
        assert!(before.is_some_and(|guard| !guard.admit()));
        assert!(unrelated.is_some_and(|guard| guard.admit()));
    }

    #[tokio::test]
    async fn a_failed_commit_still_refuses_captures() {
        let (privacy, store) = privacy();
        store.fail.store(true, Ordering::SeqCst);
        let session = SessionId::new();
        let result = privacy
            .make_private(
                session,
                PrivateReason::SecretInput,
                EpochMillis::from_millis(1),
            )
            .await;
        assert!(result.is_err());
        assert!(privacy.capture(session, page("t")).is_none());
    }

    #[test]
    fn a_document_hold_blocks_its_page_until_a_new_document_commits() {
        let (privacy, _) = privacy();
        let session = SessionId::new();
        let started = privacy.capture(session, page("t1"));
        privacy.hold_document(page("t1"), DocumentKey("L1".into()));
        assert!(started.is_some_and(|guard| !guard.admit()));
        assert!(privacy.capture(session, page("t1")).is_none());
        assert!(privacy.capture(session, page("t2")).is_some());
        privacy.document_committed(&page("t1"), &DocumentKey("L1".into()));
        assert!(
            privacy.capture(session, page("t1")).is_none(),
            "same document"
        );
        privacy.document_committed(&page("t1"), &DocumentKey("L2".into()));
        assert!(privacy.capture(session, page("t1")).is_some());
    }

    #[tokio::test]
    async fn a_control_display_taints_live_sessions_and_sessions_that_start_while_open() {
        let (privacy, _) = privacy();
        let live = SessionId::new();
        let idle = SessionId::new();
        let at = EpochMillis::from_millis(10);
        let display = privacy.control_display_opened();
        let late = SessionId::new();
        assert!(privacy.session_started(late, at).await.is_ok());
        assert!(
            privacy.is_private(late).is_some(),
            "started while the display was open"
        );
        assert!(privacy.is_private(live).is_none(), "no input yet");
        assert!(display.first_input(&[live], at).await.is_ok());
        assert!(privacy.is_private(live).is_some());
        assert!(privacy.is_private(idle).is_none());
        drop(display);
        let after = SessionId::new();
        assert!(privacy.session_started(after, at).await.is_ok());
        assert!(
            privacy.is_private(after).is_none(),
            "the display had closed"
        );
    }

    #[test]
    fn flags_loaded_at_startup_hold() {
        let session = SessionId::new();
        let mut loaded = HashMap::new();
        loaded.insert(
            session,
            PrivacyRecord {
                since: EpochMillis::from_millis(1),
                reason: PrivateReason::Host,
            },
        );
        let privacy = Privacy::new(
            Arc::new(SecretSet::new(SecretSetConfig::default())),
            Arc::new(Store::default()),
            loaded,
        );
        assert!(privacy.capture(session, page("t")).is_none());
    }

    fn commits(store: &Store) -> usize {
        store.committed.lock().map(|c| c.len()).unwrap_or(0)
    }

    #[tokio::test]
    async fn a_flag_whose_commit_failed_is_committed_again_before_ok() {
        let (privacy, store) = privacy();
        let session = SessionId::new();
        store.fail.store(true, Ordering::SeqCst);
        let at = EpochMillis::from_millis(1);
        assert!(
            privacy
                .make_private(session, PrivateReason::SecretInput, at)
                .await
                .is_err()
        );
        store.fail.store(false, Ordering::SeqCst);
        assert!(
            privacy
                .make_private(session, PrivateReason::SecretInput, at)
                .await
                .is_ok()
        );
        assert_eq!(
            commits(&store),
            1,
            "the retry committed instead of trusting memory"
        );
        assert!(
            privacy
                .make_private(session, PrivateReason::Host, at)
                .await
                .is_ok()
        );
        assert_eq!(
            commits(&store),
            1,
            "a committed flag is not committed twice"
        );
    }

    #[tokio::test]
    async fn a_second_caller_waits_for_a_commit_in_flight() {
        let (privacy, store) = with_store(Store {
            gate: Some(tokio::sync::Semaphore::new(0)),
            ..Store::default()
        });
        let session = SessionId::new();
        let at = EpochMillis::from_millis(1);
        let first = tokio::spawn({
            let privacy = privacy.clone();
            async move {
                privacy
                    .make_private(session, PrivateReason::SecretInput, at)
                    .await
            }
        });
        tokio::task::yield_now().await;
        let second = privacy.make_private(session, PrivateReason::SecretInput, at);
        tokio::pin!(second);
        let early = tokio::time::timeout(std::time::Duration::from_millis(50), &mut second).await;
        assert!(
            early.is_err(),
            "the second caller returned before anything was on disk"
        );
        if let Some(gate) = &store.gate {
            gate.add_permits(2);
        }
        assert!(second.await.is_ok());
        assert!(first.await.is_ok_and(|result| result.is_ok()));
    }

    #[test]
    fn a_closed_page_never_readmits_a_capture_a_hold_invalidated() {
        let (privacy, _) = privacy();
        let session = SessionId::new();
        let started = privacy.capture(session, page("t1"));
        privacy.hold_document(page("t1"), DocumentKey("L1".into()));
        privacy.page_closed(&page("t1"));
        assert!(started.is_some_and(|guard| !guard.admit()));
    }

    #[tokio::test]
    async fn display_input_waits_for_its_commits_and_retries_after_a_failure() {
        let (privacy, store) = with_store(Store {
            gate: Some(tokio::sync::Semaphore::new(0)),
            ..Store::default()
        });
        let live = [SessionId::new(), SessionId::new()];
        let at = EpochMillis::from_millis(1);
        let display = Arc::new(privacy.control_display_opened());
        let first = tokio::spawn({
            let display = display.clone();
            async move { display.first_input(&live, at).await }
        });
        tokio::task::yield_now().await;
        let second = display.first_input(&live, at);
        tokio::pin!(second);
        let early = tokio::time::timeout(std::time::Duration::from_millis(50), &mut second).await;
        assert!(
            early.is_err(),
            "a second input was forwarded before the sessions were private"
        );
        if let Some(gate) = &store.gate {
            gate.add_permits(10);
        }
        assert!(first.await.is_ok_and(|result| result.is_ok()));
        assert!(second.await.is_ok());
        assert_eq!(commits(&store), 2);

        let (privacy, store) = privacy_failing();
        let display = privacy.control_display_opened();
        let session = SessionId::new();
        assert!(display.first_input(&[session], at).await.is_err());
        store.fail.store(false, Ordering::SeqCst);
        assert!(display.first_input(&[session], at).await.is_ok());
        assert_eq!(
            commits(&store),
            1,
            "the failed first input was retried, not skipped"
        );
    }

    fn privacy_failing() -> (Arc<Privacy>, Arc<Store>) {
        let (privacy, store) = privacy();
        store.fail.store(true, Ordering::SeqCst);
        (privacy, store)
    }
}
