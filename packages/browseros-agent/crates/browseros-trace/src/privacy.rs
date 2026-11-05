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

#[derive(Default)]
struct State {
    private: HashMap<SessionId, PrivacyRecord>,
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
                private: loaded,
                ..State::default()
            }),
        })
    }

    /// The process-wide secret set this privacy masks text with.
    #[must_use]
    pub fn secrets(&self) -> &Arc<SecretSet> {
        &self.secrets
    }

    /// Makes `session` private, durably. Idempotent: an already private session keeps its first
    /// record.
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
            if state.private.contains_key(&session) {
                return Ok(());
            }
            let record = PrivacyRecord { since: at, reason };
            state.private.insert(session, record);
            *state.session_epochs.entry(session).or_default() += 1;
            record
        };
        self.store
            .persist_private(session, record)
            .await
            .map_err(PrivacyError::NotCommitted)
    }

    /// The session's privacy, when it is private.
    #[must_use]
    pub fn is_private(&self, session: SessionId) -> Option<PrivacyRecord> {
        self.lock().private.get(&session).copied()
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

    /// A page closed: forget its hold and epoch.
    pub fn page_closed(&self, page: &PageKey) {
        let mut state = self.lock();
        state.holds.remove(page);
        state.page_epochs.remove(page);
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
            input_seen: Mutex::new(false),
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
    input_seen: Mutex<bool>,
}

impl ControlDisplay {
    /// The display's first input event is about to be forwarded. Every session live now
    /// becomes private first. Later calls do nothing.
    ///
    /// # Errors
    /// When a flag could not be committed. The caller drops the input.
    pub async fn first_input(
        &self,
        live: &[SessionId],
        at: EpochMillis,
    ) -> Result<(), PrivacyError> {
        {
            let mut seen = self
                .input_seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if *seen {
                return Ok(());
            }
            *seen = true;
        }
        for session in live {
            if let Err(error) = self
                .privacy
                .make_private(*session, PrivateReason::DisplayInput, at)
                .await
            {
                // Retry on the next input: the flags set in memory still refuse captures.
                *self
                    .input_seen
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = false;
                return Err(error);
            }
        }
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
    }

    impl PrivacyStore for Store {
        fn persist_private(
            &self,
            session: SessionId,
            record: PrivacyRecord,
        ) -> BoxFuture<'_, Result<(), TraceError>> {
            Box::pin(async move {
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
        let store = Arc::new(Store::default());
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
}
