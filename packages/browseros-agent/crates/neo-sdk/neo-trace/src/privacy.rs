//! Which pixels Neo may capture. A private tab gets no more screenshots, frames or live views.
//! Privacy is sticky and durable: a private tab stays private across Neo restarts, until it
//! closes.
//!
//! Ordering is the guarantee. Making a tab private sets the flag in memory and bumps its epoch
//! first, then commits the flag durably, and only then returns. Only after that does a
//! protected fill type or an owner's display open. Every capture holds a [`CaptureGuard`] taken
//! when it started, and calls [`CaptureGuard::admit`] once its pixels arrive:
//!
//! ```text
//!  make_private:  flag + epoch bump ──▶ durable commit ──▶ return ──▶ first keystroke
//!  a capture:     guard (epoch) ──▶ CDP capture ──▶ pixels arrive ──▶ admit? store : drop
//! ```
//!
//! Pixels that could show the secret are produced after the first keystroke, which comes after
//! the bump. They arrive after they are produced, so `admit` sees the new epoch and drops them.
//! Pixels a guard admits were produced before the bump. That is why visuals from before a tab
//! went private are kept, and why nothing here waits for in-flight captures.
//!
//! A flag is keyed by the browser's own tab id, and Chromium numbers tabs from 1 again after it
//! restarts. So a flag is forgotten when its tab closes ([`Privacy::tab_closed`]) and, when the
//! browser link connects, for every tab that is not open ([`Privacy::retain_tabs`]). A flag
//! that outlives its tab by mistake only over-protects the next tab with that id.
//!
//! The disk changes in the order memory does. Each flag and each delete is queued with the
//! [`PrivacyStore`] under the lock that changes memory, so when a close races a `make_private`
//! of the same tab, the disk ends where memory does. Queued after the lock is released, a
//! delete could land before the flag it deletes and leave that flag on disk. Worse, it could
//! land after a newer flag and erase it, and after a restart Neo would capture a tab that
//! memory had kept private.

use crate::{EpochMillis, SecretSet, TraceError};
use browseros_core::TabId;
use futures_util::future::BoxFuture;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Why a tab went private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivateReason {
    /// A secret was typed into the tab, e.g. by a protected fill.
    SecretInput,
    /// A takeover gave the tab to the owner, who may type anything.
    Takeover,
    /// A control display forwarded input while the tab was open, or the tab opened while one
    /// was open.
    DisplayInput,
    /// The host asked (`POST /v1/tabs/{tabId}/private`).
    Host,
    /// Neo's trace file broke and its flags may not all have been carried over
    /// ([`Salvage::flags`](crate::Salvage::flags)), so the app made every tab open at the
    /// browser's next listing private: any of them may have been.
    FlagsLost,
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
            Self::FlagsLost => "flags_lost",
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
            "flags_lost" => Some(Self::FlagsLost),
            _ => None,
        }
    }
}

/// When and why a tab went private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivacyRecord {
    /// No pixel at or after this time is captured.
    pub since: EpochMillis,
    /// Why.
    pub reason: PrivateReason,
}

/// Where private flags are made durable. The trace writer implements it, so a flag is committed
/// in order with the records around it.
///
/// The store orders each write when the method is called, not when its future is first polled.
/// [`Privacy`] calls while it holds its lock, so a call must not block, and the disk sees a
/// tab's flags and deletes in the order memory made them.
pub trait PrivacyStore: Send + Sync {
    /// Queues the commit of `tab`'s flag. The future resolves once it is on disk. A tab that
    /// already has a durable flag keeps its first one.
    fn persist_private(
        &self,
        tab: TabId,
        record: PrivacyRecord,
    ) -> BoxFuture<'_, Result<(), TraceError>>;

    /// Queues the delete of `tab`'s flag. The future resolves once the delete is on disk.
    fn forget_private(&self, tab: TabId) -> BoxFuture<'_, Result<(), TraceError>>;
}

/// Privacy could not be committed. Callers refuse the action: the fill is not sent, and the
/// control is not given.
#[derive(Debug, thiserror::Error)]
pub enum PrivacyError {
    /// The flag is set in memory (fail closed) but not on disk.
    #[error("the private flag could not be committed: {0}")]
    NotCommitted(#[source] TraceError),
}

/// A tab's flag. It refuses captures from the moment it is set; `committed` says whether it is
/// also on disk, which is what `make_private` waits for before it returns.
#[derive(Clone, Copy)]
struct PrivateFlag {
    record: PrivacyRecord,
    committed: bool,
}

#[derive(Default)]
struct State {
    private: HashMap<TabId, PrivateFlag>,
    /// Moves on each time a tab goes private and each time its flag is forgotten, which is what
    /// invalidates a guard. An entry is never removed: a missing epoch reads as 0, so a guard
    /// begun before the tab went private would match it again.
    epochs: HashMap<TabId, u64>,
    control_displays: usize,
}

impl State {
    fn epoch(&self, tab: TabId) -> u64 {
        self.epochs.get(&tab).copied().unwrap_or(0)
    }
}

/// Pixel privacy for the whole process, and the door to the secret set.
pub struct Privacy {
    secrets: Arc<SecretSet>,
    store: Arc<dyn PrivacyStore>,
    state: Mutex<State>,
}

impl Privacy {
    /// Starts from the flags the store loaded, so a restart keeps every private tab private.
    #[must_use]
    pub fn new(
        secrets: Arc<SecretSet>,
        store: Arc<dyn PrivacyStore>,
        loaded: HashMap<TabId, PrivacyRecord>,
    ) -> Arc<Self> {
        Arc::new(Self {
            secrets,
            store,
            state: Mutex::new(State {
                private: loaded
                    .into_iter()
                    .map(|(tab, record)| {
                        let flag = PrivateFlag {
                            record,
                            committed: true,
                        };
                        (tab, flag)
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

    /// Makes `tab` private, durably. Returns only once the flag is on disk, so a caller that
    /// gets `Ok` may type a secret. An already private tab keeps its first record. A flag that
    /// is set but not yet committed (another call is committing it, or an earlier commit
    /// failed) is committed again here rather than trusted. The commit is idempotent.
    ///
    /// # Errors
    /// When the flag could not be committed. It still holds in memory, so captures stay refused.
    pub async fn make_private(
        &self,
        tab: TabId,
        reason: PrivateReason,
        at: EpochMillis,
    ) -> Result<(), PrivacyError> {
        let (epoch, commit) = {
            let mut state = self.lock();
            let record = match state.private.get(&tab) {
                Some(flag) if flag.committed => return Ok(()),
                Some(flag) => flag.record,
                None => {
                    let record = PrivacyRecord { since: at, reason };
                    state.private.insert(
                        tab,
                        PrivateFlag {
                            record,
                            committed: false,
                        },
                    );
                    *state.epochs.entry(tab).or_default() += 1;
                    record
                }
            };
            // Called under the lock, so the commit is queued ahead of the delete of any close
            // that forgets this flag.
            (state.epoch(tab), self.store.persist_private(tab, record))
        };
        commit.await.map_err(PrivacyError::NotCommitted)?;
        let mut state = self.lock();
        // A moved epoch means a close forgot the flag this commit wrote. A flag set since then
        // is on disk only once its own commit is.
        if state.epoch(tab) == epoch
            && let Some(flag) = state.private.get_mut(&tab)
        {
            flag.committed = true;
        }
        Ok(())
    }

    /// The tab's privacy, when it is private.
    #[must_use]
    pub fn is_private(&self, tab: TabId) -> Option<PrivacyRecord> {
        self.lock().private.get(&tab).map(|flag| flag.record)
    }

    /// Starts a capture of `tab`, or `None` while the tab is private.
    #[must_use]
    pub fn capture(self: &Arc<Self>, tab: TabId) -> Option<CaptureGuard> {
        let state = self.lock();
        if state.private.contains_key(&tab) {
            return None;
        }
        let epoch = state.epoch(tab);
        drop(state);
        Some(CaptureGuard {
            privacy: Arc::clone(self),
            tab,
            epoch,
        })
    }

    /// A control display opened. Until it closes, every tab that opens is private.
    #[must_use]
    pub fn control_display_opened(self: &Arc<Self>) -> ControlDisplay {
        self.lock().control_displays += 1;
        ControlDisplay {
            privacy: Arc::clone(self),
            input_seen: tokio::sync::Mutex::new(false),
        }
    }

    /// A tab opened. While a control display is open it starts private (the decided taint
    /// rule): the owner may already be typing into it.
    ///
    /// # Errors
    /// When the flag could not be committed.
    pub async fn tab_opened(&self, tab: TabId, at: EpochMillis) -> Result<(), PrivacyError> {
        if self.lock().control_displays == 0 {
            return Ok(());
        }
        self.make_private(tab, PrivateReason::DisplayInput, at)
            .await
    }

    /// A tab closed, so its flag ends: first in memory, then on disk. Its epoch moves on and is
    /// kept, so a capture begun before the tab went private is still never admitted. A delete
    /// that fails is logged and leaves the row, which only over-protects a later tab with this
    /// id. A tab that was not private has nothing to forget.
    pub async fn tab_closed(&self, tab: TabId) {
        let forget = {
            let mut state = self.lock();
            if state.private.remove(&tab).is_none() {
                return;
            }
            *state.epochs.entry(tab).or_default() += 1;
            // Called under the lock, so the delete is queued after the commit of the flag it
            // deletes and before the commit of any flag set after it.
            self.store.forget_private(tab)
        };
        if let Err(error) = forget.await {
            tracing::warn!(%error, tab = tab.0, "a closed tab's private flag stays on disk");
        }
    }

    /// The browser link connected with `open` tabs. Every flag of a tab that is not open is
    /// forgotten as [`Privacy::tab_closed`] does: the tab closed while Neo was not watching,
    /// or the browser restarted and numbers its tabs from 1 again.
    pub async fn retain_tabs(&self, open: &[TabId]) {
        let stale = self
            .lock()
            .private
            .keys()
            .filter(|tab| !open.contains(tab))
            .copied()
            .collect::<Vec<_>>();
        for tab in stale {
            self.tab_closed(tab).await;
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One capture's claim. Check [`CaptureGuard::admit`] after the pixels arrive and before they
/// are stored or forwarded.
pub struct CaptureGuard {
    privacy: Arc<Privacy>,
    tab: TabId,
    epoch: u64,
}

impl CaptureGuard {
    /// Whether the pixels this capture produced may be kept: the tab's privacy did not change
    /// since the capture began.
    #[must_use]
    pub fn admit(&self) -> bool {
        let state = self.privacy.lock();
        !state.private.contains_key(&self.tab) && state.epoch(self.tab) == self.epoch
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
    /// An input event is about to be forwarded. Before the first one, every tab open now becomes
    /// private, durably: the owner can reach any of them from the one screen. Concurrent inputs
    /// wait for that, and later calls return at once.
    ///
    /// # Errors
    /// When a flag could not be committed. The caller drops the input; the next one retries.
    pub async fn first_input(
        &self,
        open_tabs: &[TabId],
        at: EpochMillis,
    ) -> Result<(), PrivacyError> {
        let mut seen = self.input_seen.lock().await;
        if *seen {
            return Ok(());
        }
        for tab in open_tabs {
            self.privacy
                .make_private(*tab, PrivateReason::DisplayInput, at)
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
    use super::{Privacy, PrivacyRecord, PrivacyStore, PrivateReason};
    use crate::{EpochMillis, SecretSet, SecretSetConfig, TraceError};
    use browseros_core::TabId;
    use futures_util::future::BoxFuture;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// A durable store in memory: `rows` is what a restart would load. It writes a row when the
    /// commit's future runs, not when the commit is called. No test in this module races a flag
    /// against its delete, so the difference does not matter here. `tests/trace_store.rs` races
    /// them on the real writer.
    #[derive(Default)]
    struct Store {
        rows: Mutex<HashMap<TabId, PrivacyRecord>>,
        commits: Mutex<usize>,
        fail: AtomicBool,
        /// When set, a commit waits for a permit: the test holds a commit open.
        gate: Option<tokio::sync::Semaphore>,
    }

    impl Store {
        fn rows(&self) -> HashMap<TabId, PrivacyRecord> {
            self.rows
                .lock()
                .map(|rows| rows.clone())
                .unwrap_or_default()
        }

        fn commits(&self) -> usize {
            self.commits.lock().map(|count| *count).unwrap_or(0)
        }
    }

    impl PrivacyStore for Store {
        fn persist_private(
            &self,
            tab: TabId,
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
                *self.commits.lock().map_err(|_| TraceError::Closed)? += 1;
                self.rows
                    .lock()
                    .map_err(|_| TraceError::Closed)?
                    .entry(tab)
                    .or_insert(record);
                Ok(())
            })
        }

        fn forget_private(&self, tab: TabId) -> BoxFuture<'_, Result<(), TraceError>> {
            Box::pin(async move {
                if self.fail.load(Ordering::SeqCst) {
                    return Err(TraceError::Closed);
                }
                self.rows
                    .lock()
                    .map_err(|_| TraceError::Closed)?
                    .remove(&tab);
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
            store.rows(),
        );
        (privacy, store)
    }

    /// What a Neo restart over the same store would start with.
    fn restarted(store: &Arc<Store>) -> Arc<Privacy> {
        Privacy::new(
            Arc::new(SecretSet::new(SecretSetConfig::default())),
            store.clone(),
            store.rows(),
        )
    }

    const AT: EpochMillis = EpochMillis::from_millis(1_000);

    #[tokio::test]
    async fn a_private_tab_refuses_new_captures_and_commits_once() {
        let (privacy, store) = privacy();
        let tab = TabId(1);
        assert!(privacy.capture(tab).is_some());
        assert!(
            privacy
                .make_private(tab, PrivateReason::SecretInput, AT)
                .await
                .is_ok()
        );
        assert!(
            privacy
                .make_private(tab, PrivateReason::Host, AT)
                .await
                .is_ok()
        );
        assert!(privacy.capture(tab).is_none());
        assert!(privacy.capture(TabId(2)).is_some(), "only that tab");
        assert_eq!(
            privacy.is_private(tab),
            Some(PrivacyRecord {
                since: AT,
                reason: PrivateReason::SecretInput
            })
        );
        assert_eq!(store.commits(), 1);
    }

    #[tokio::test]
    async fn a_capture_begun_before_the_tab_went_private_is_not_admitted() {
        let (privacy, _) = privacy();
        let before = privacy.capture(TabId(1));
        let unrelated = privacy.capture(TabId(2));
        assert!(before.as_ref().is_some_and(|guard| guard.admit()));
        let _ = privacy
            .make_private(TabId(1), PrivateReason::Takeover, AT)
            .await;
        assert!(before.is_some_and(|guard| !guard.admit()));
        assert!(unrelated.is_some_and(|guard| guard.admit()));
    }

    #[tokio::test]
    async fn a_failed_commit_still_refuses_captures() {
        let (privacy, store) = privacy();
        store.fail.store(true, Ordering::SeqCst);
        let result = privacy
            .make_private(TabId(1), PrivateReason::SecretInput, AT)
            .await;
        assert!(result.is_err());
        assert!(privacy.capture(TabId(1)).is_none());
    }

    #[tokio::test]
    async fn a_private_tab_stays_private_after_a_restart() {
        let (privacy, store) = privacy();
        let _ = privacy
            .make_private(TabId(4), PrivateReason::Host, AT)
            .await;
        drop(privacy);
        let privacy = restarted(&store);
        assert!(privacy.capture(TabId(4)).is_none());
        assert_eq!(
            privacy.is_private(TabId(4)).map(|record| record.reason),
            Some(PrivateReason::Host)
        );
    }

    #[tokio::test]
    async fn a_tab_that_opens_while_a_control_display_is_open_starts_private() {
        let (privacy, store) = privacy();
        let display = privacy.control_display_opened();
        assert!(privacy.tab_opened(TabId(7), AT).await.is_ok());
        assert_eq!(
            privacy.is_private(TabId(7)).map(|record| record.reason),
            Some(PrivateReason::DisplayInput)
        );
        assert!(store.rows().contains_key(&TabId(7)), "durably");
        drop(display);
        assert!(privacy.tab_opened(TabId(8), AT).await.is_ok());
        assert!(
            privacy.is_private(TabId(8)).is_none(),
            "the display had closed"
        );
    }

    #[tokio::test]
    async fn the_first_display_input_makes_every_open_tab_private() {
        let (privacy, _) = privacy();
        let display = privacy.control_display_opened();
        assert!(privacy.is_private(TabId(1)).is_none(), "no input yet");
        assert!(display.first_input(&[TabId(1), TabId(2)], AT).await.is_ok());
        assert!(privacy.is_private(TabId(1)).is_some());
        assert!(privacy.is_private(TabId(2)).is_some());
        assert!(
            privacy.is_private(TabId(3)).is_none(),
            "a tab that was not open"
        );
        assert!(display.first_input(&[TabId(3)], AT).await.is_ok());
        assert!(
            privacy.is_private(TabId(3)).is_none(),
            "only the first input taints"
        );
    }

    #[tokio::test]
    async fn closing_a_private_tab_forgets_its_flag_durably() {
        let (privacy, store) = privacy();
        let tab = TabId(3);
        let _ = privacy
            .make_private(tab, PrivateReason::SecretInput, AT)
            .await;
        privacy.tab_closed(tab).await;
        assert!(privacy.is_private(tab).is_none());
        assert!(store.rows().is_empty());
        assert!(restarted(&store).is_private(tab).is_none());
    }

    #[tokio::test]
    async fn a_failed_forget_leaves_the_durable_flag() {
        let (privacy, store) = privacy();
        let tab = TabId(3);
        let _ = privacy.make_private(tab, PrivateReason::Host, AT).await;
        store.fail.store(true, Ordering::SeqCst);
        privacy.tab_closed(tab).await;
        assert!(privacy.is_private(tab).is_none(), "forgotten in memory");
        assert!(
            restarted(&store).is_private(tab).is_some(),
            "a stale flag over-protects; it never fails open"
        );
    }

    #[tokio::test]
    async fn a_closed_tab_never_readmits_a_capture_begun_before_it_went_private() {
        let (privacy, _) = privacy();
        let tab = TabId(5);
        let started = privacy.capture(tab);
        let _ = privacy.make_private(tab, PrivateReason::Host, AT).await;
        privacy.tab_closed(tab).await;
        assert!(started.is_some_and(|guard| !guard.admit()));
    }

    #[tokio::test]
    async fn retain_tabs_forgets_the_flags_of_tabs_that_are_not_open() {
        let (privacy, store) = privacy();
        for tab in [TabId(1), TabId(2), TabId(3)] {
            let _ = privacy.make_private(tab, PrivateReason::Host, AT).await;
        }
        privacy.retain_tabs(&[TabId(2), TabId(9)]).await;
        assert!(privacy.is_private(TabId(1)).is_none());
        assert!(privacy.is_private(TabId(2)).is_some());
        assert!(privacy.is_private(TabId(3)).is_none());
        assert_eq!(store.rows().keys().copied().collect::<Vec<_>>(), [TabId(2)]);
    }

    #[tokio::test]
    async fn a_flag_whose_commit_failed_is_committed_again_before_ok() {
        let (privacy, store) = privacy();
        let tab = TabId(1);
        store.fail.store(true, Ordering::SeqCst);
        assert!(
            privacy
                .make_private(tab, PrivateReason::SecretInput, AT)
                .await
                .is_err()
        );
        store.fail.store(false, Ordering::SeqCst);
        assert!(
            privacy
                .make_private(tab, PrivateReason::SecretInput, AT)
                .await
                .is_ok()
        );
        assert_eq!(
            store.commits(),
            1,
            "the retry committed instead of trusting memory"
        );
        assert!(
            privacy
                .make_private(tab, PrivateReason::Host, AT)
                .await
                .is_ok()
        );
        assert_eq!(
            store.commits(),
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
        let tab = TabId(1);
        let first = tokio::spawn({
            let privacy = privacy.clone();
            async move {
                privacy
                    .make_private(tab, PrivateReason::SecretInput, AT)
                    .await
            }
        });
        tokio::task::yield_now().await;
        let second = privacy.make_private(tab, PrivateReason::SecretInput, AT);
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

    #[tokio::test]
    async fn display_input_waits_for_its_commits_and_retries_after_a_failure() {
        let (privacy, store) = with_store(Store {
            gate: Some(tokio::sync::Semaphore::new(0)),
            ..Store::default()
        });
        let open = [TabId(1), TabId(2)];
        let display = Arc::new(privacy.control_display_opened());
        let first = tokio::spawn({
            let display = display.clone();
            async move { display.first_input(&open, AT).await }
        });
        tokio::task::yield_now().await;
        let second = display.first_input(&open, AT);
        tokio::pin!(second);
        let early = tokio::time::timeout(std::time::Duration::from_millis(50), &mut second).await;
        assert!(
            early.is_err(),
            "a second input was forwarded before the tabs were private"
        );
        if let Some(gate) = &store.gate {
            gate.add_permits(10);
        }
        assert!(first.await.is_ok_and(|result| result.is_ok()));
        assert!(second.await.is_ok());
        assert_eq!(store.commits(), 2);

        let (privacy, store) = with_store(Store::default());
        store.fail.store(true, Ordering::SeqCst);
        let display = privacy.control_display_opened();
        assert!(display.first_input(&[TabId(1)], AT).await.is_err());
        store.fail.store(false, Ordering::SeqCst);
        assert!(display.first_input(&[TabId(1)], AT).await.is_ok());
        assert_eq!(
            store.commits(),
            1,
            "the failed first input was retried, not skipped"
        );
    }
}
