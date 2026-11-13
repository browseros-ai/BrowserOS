//! Host claims that exempt tabs from idle cleanup without restricting agent operations.
//!
//! Expiry uses Tokio's monotonic clock, while the host sees an epoch timestamp. No task wakes
//! for a keep. Reads expire it at its deadline and retain that end for an hour so the janitor
//! restarts the idle clock from the actual release, even when its next sweep comes later.

use crate::limits::ENDED_KEEP_RETENTION;
use browseros_core::TabId;
use neo_takeover::TabHold;
use neo_trace::EpochMillis;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::time::Instant;

/// A live keep as the host reads it. An absent expiry means it lasts until explicit cleanup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Keep {
    pub(crate) expires_at: Option<EpochMillis>,
}

/// One optional deadline, with its two clocks kept together so the wire and idle clock agree.
#[derive(Clone, Copy)]
struct Expiry {
    deadline: Instant,
    at: EpochMillis,
}

enum State {
    Active(Option<Expiry>),
    Ended(Instant),
}

/// In-memory, per-tab keeps owned by one service instance. Closing or losing a tab drops all
/// its state, and constructing a new service never restores a keep onto a reused tab number.
#[derive(Default)]
pub(crate) struct TabKeeps {
    entries: Mutex<HashMap<TabId, State>>,
}

impl TabKeeps {
    pub(crate) fn set(&self, tab: TabId, ttl: Option<Duration>) -> Keep {
        let expiry = ttl.map(|ttl| Expiry {
            deadline: Instant::now() + ttl,
            at: EpochMillis::now().plus_millis(i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX)),
        });
        self.table().insert(tab, State::Active(expiry));
        Keep {
            expires_at: expiry.map(|expiry| expiry.at),
        }
    }

    pub(crate) fn get(&self, tab: TabId) -> Option<Keep> {
        match self.table().get(&tab)? {
            State::Active(expiry) => Some(Keep {
                expires_at: expiry.map(|expiry| expiry.at),
            }),
            State::Ended(_) => None,
        }
    }

    pub(crate) fn delete(&self, tab: TabId) {
        if let Some(state @ State::Active(_)) = self.table().get_mut(&tab) {
            *state = State::Ended(Instant::now());
        }
    }

    pub(crate) fn hold(&self, tab: TabId) -> Option<TabHold> {
        self.table().get(&tab).map(|state| match state {
            State::Active(_) => TabHold::Now,
            State::Ended(at) => TabHold::Until(*at),
        })
    }

    pub(crate) fn tab_closed(&self, tab: TabId) {
        self.table().remove(&tab);
    }

    pub(crate) fn retain_tabs(&self, open: &[TabId]) {
        self.table().retain(|tab, _| open.contains(tab));
    }

    /// One short synchronous lock, never held over browser I/O. Every access also forgets old
    /// releases, so lazy expiry does not retain historical tabs for the server's lifetime.
    fn table(&self) -> MutexGuard<'_, HashMap<TabId, State>> {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        entries.retain(|_, state| {
            if let State::Active(Some(expiry)) = state
                && expiry.deadline <= now
            {
                *state = State::Ended(expiry.deadline);
            }
            match state {
                State::Ended(at) => now.duration_since(*at) < ENDED_KEEP_RETENTION,
                State::Active(_) => true,
            }
        });
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::TabKeeps;
    use browseros_core::TabId;
    use neo_takeover::TabHold;
    use neo_trace::EpochMillis;
    use std::time::Duration;
    use tokio::time::{Instant, advance};

    #[tokio::test(start_paused = true)]
    async fn an_unlimited_keep_holds_until_deleted_once() {
        let keeps = TabKeeps::default();
        let tab = TabId(3);
        assert_eq!(keeps.get(tab), None);
        assert_eq!(keeps.hold(tab), None);
        keeps.delete(tab);
        assert_eq!(keeps.hold(tab), None);

        assert_eq!(keeps.set(tab, None).expires_at, None);
        advance(Duration::from_secs(60 * 60 * 24 * 31)).await;
        assert!(keeps.get(tab).is_some());
        assert_eq!(keeps.hold(tab), Some(TabHold::Now));
        let ended = Instant::now();
        keeps.delete(tab);
        assert_eq!(keeps.get(tab), None);
        assert_eq!(keeps.hold(tab), Some(TabHold::Until(ended)));
        advance(Duration::from_secs(10)).await;
        keeps.delete(tab);
        assert_eq!(keeps.hold(tab), Some(TabHold::Until(ended)));
    }

    #[tokio::test(start_paused = true)]
    async fn expiry_is_exact_even_when_the_first_read_is_late() {
        let keeps = TabKeeps::default();
        let tab = TabId(3);
        let before = EpochMillis::now().millis();
        let deadline = Instant::now() + Duration::from_secs(60);
        let keep = keeps.set(tab, Some(Duration::from_secs(60)));
        let after = EpochMillis::now().millis();
        assert!(
            keep.expires_at
                .is_some_and(|at| { (before + 60_000..=after + 60_000).contains(&at.millis()) })
        );
        advance(Duration::from_secs(59)).await;
        assert_eq!(keeps.get(tab), Some(keep));
        assert_eq!(keeps.hold(tab), Some(TabHold::Now));
        advance(Duration::from_secs(21)).await;
        assert_eq!(keeps.get(tab), None);
        assert_eq!(keeps.hold(tab), Some(TabHold::Until(deadline)));
        keeps.delete(tab);
        assert_eq!(keeps.hold(tab), Some(TabHold::Until(deadline)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_repeat_put_replaces_the_deadline_in_both_directions() {
        let keeps = TabKeeps::default();
        let tab = TabId(3);
        keeps.set(tab, Some(Duration::from_secs(1)));
        keeps.set(tab, None);
        advance(Duration::from_secs(2)).await;
        assert_eq!(keeps.hold(tab), Some(TabHold::Now));
        let deadline = Instant::now() + Duration::from_secs(1);
        keeps.set(tab, Some(Duration::from_secs(1)));
        advance(Duration::from_secs(1)).await;
        assert_eq!(keeps.hold(tab), Some(TabHold::Until(deadline)));
        keeps.set(tab, Some(Duration::from_secs(2)));
        keeps.set(tab, Some(Duration::from_secs(4)));
        advance(Duration::from_secs(2)).await;
        assert_eq!(keeps.hold(tab), Some(TabHold::Now));
    }

    #[tokio::test(start_paused = true)]
    async fn ended_keeps_are_forgotten_one_hour_after_the_actual_end() {
        let keeps = TabKeeps::default();
        let tab = TabId(3);
        let deadline = Instant::now() + Duration::from_secs(1);
        keeps.set(tab, Some(Duration::from_secs(1)));
        advance(Duration::from_secs(3600)).await;
        assert_eq!(keeps.hold(tab), Some(TabHold::Until(deadline)));
        advance(Duration::from_secs(1)).await;
        assert_eq!(keeps.hold(tab), None);

        keeps.set(tab, None);
        keeps.delete(tab);
        advance(Duration::from_secs(3600)).await;
        assert_eq!(keeps.hold(tab), None);
    }

    #[tokio::test(start_paused = true)]
    async fn closed_or_unlisted_tabs_lose_keeps_and_release_history() {
        let keeps = TabKeeps::default();
        for tab in [3, 5, 7] {
            keeps.set(TabId(tab), None);
        }
        keeps.delete(TabId(7));
        keeps.tab_closed(TabId(3));
        assert_eq!(keeps.hold(TabId(3)), None);
        keeps.retain_tabs(&[TabId(5)]);
        assert_eq!(keeps.hold(TabId(5)), Some(TabHold::Now));
        assert_eq!(keeps.hold(TabId(7)), None);
        assert_eq!(TabKeeps::default().hold(TabId(5)), None);
    }
}
