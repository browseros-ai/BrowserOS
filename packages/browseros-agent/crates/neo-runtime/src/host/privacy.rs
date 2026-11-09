//! Neo: The host's synchronous privacy boundary. Taint has one add-only set shared by the emitter,
//! ledger, capture and artifact worker. A session's flag and touched-target union share a short
//! lock: a touch racing withhold is included on one side of that boundary, never missed.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use tokio::sync::watch;

use crate::host::SessionKey;
use crate::run::PageId;

/// The single source of truth for targets whose pixels no session may expose. Inserts commute;
/// a read never relies on the host task having consumed the corresponding tap event.
pub(super) struct Taint {
    targets: RwLock<HashSet<PageId>>,
    changed: watch::Sender<()>,
}

impl Default for Taint {
    fn default() -> Self {
        Self {
            targets: RwLock::default(),
            changed: watch::Sender::new(()),
        }
    }
}

impl Taint {
    pub(super) fn contains(&self, target: &PageId) -> bool {
        // A poisoned privacy lock fails closed; none of its guarded operations awaits or
        // executes caller code, so ordinary cancellation cannot poison it.
        self.targets
            .read()
            .map_or(true, |targets| targets.contains(target))
    }

    pub(super) fn insert(&self, target: PageId) -> bool {
        !self.extend([target]).is_empty()
    }

    pub(super) fn extend(&self, targets: impl IntoIterator<Item = PageId>) -> Vec<PageId> {
        let added = if let Ok(mut taint) = self.targets.write() {
            targets
                .into_iter()
                .filter(|target| taint.insert(target.clone()))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if !added.is_empty() {
            self.changed.send_replace(());
        }
        added
    }

    pub(super) fn changes(&self) -> watch::Receiver<()> {
        self.changed.subscribe()
    }
}

/// Session lookup happens at run creation and host control calls, never for every browser
/// call. Emitters hold their session's own handle, so unrelated runs do not share that lock.
#[derive(Default)]
pub(super) struct Privacy {
    pub(super) taint: Arc<Taint>,
    sessions: Mutex<HashMap<SessionKey, Arc<SessionPrivacy>>>,
}

impl Privacy {
    pub(super) fn session(&self, key: &SessionKey) -> Option<Arc<SessionPrivacy>> {
        let mut sessions = self.sessions.lock().ok()?;
        Some(Arc::clone(sessions.entry(key.clone()).or_insert_with(
            || {
                Arc::new(SessionPrivacy {
                    state: Mutex::default(),
                    taint: Arc::clone(&self.taint),
                })
            },
        )))
    }

    pub(super) fn restore(&self, key: &SessionKey, targets: &[PageId]) {
        self.taint.extend(targets.iter().cloned());
        if let Some(session) = self.session(key) {
            for target in targets {
                session.touch(target);
            }
            let _ = session.withhold();
        }
    }

    pub(super) fn forget(&self, key: &SessionKey) {
        if let Ok(mut sessions) = self.sessions.lock() {
            // A label with no recorded run can be reused after ledger eviction. Its durable
            // withheld flag is still monotonic, so eviction must not reset the emitter flag.
            if sessions
                .get(key)
                .is_some_and(|session| session.state.lock().is_ok_and(|state| !state.withheld))
            {
                sessions.remove(key);
            }
        }
    }
}

/// The emitter's flag and conservative touch set, including touches refused by a full tap.
/// The ledger separately keeps ordered touches for display; losing one must never lose privacy.
pub(super) struct SessionPrivacy {
    state: Mutex<SessionState>,
    taint: Arc<Taint>,
}

#[derive(Default)]
struct SessionState {
    withheld: bool,
    touched: HashSet<PageId>,
}

impl SessionPrivacy {
    pub(super) fn touch(&self, target: &PageId) {
        if let Ok(mut state) = self.state.lock() {
            state.touched.insert(target.clone());
            if state.withheld {
                self.taint.insert(target.clone());
            }
        } else {
            self.taint.insert(target.clone());
        }
    }

    pub(super) fn withhold(&self) -> Option<Vec<PageId>> {
        let mut state = self.state.lock().ok()?;
        state.withheld = true;
        self.taint.extend(state.touched.iter().cloned());
        Some(state.touched.iter().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eviction_cannot_reset_a_withheld_labels_emitter_flag() {
        let privacy = Privacy::default();
        let key = SessionKey::new("withheld-without-a-stored-run");
        let first = privacy.session(&key).unwrap();
        first.touch(&PageId::new("one".to_string()));
        first.withhold().unwrap();
        privacy.forget(&key);
        privacy
            .session(&key)
            .unwrap()
            .touch(&PageId::new("two".to_string()));
        assert!(privacy.taint.contains(&PageId::new("one".to_string())));
        assert!(privacy.taint.contains(&PageId::new("two".to_string())));
    }
}
