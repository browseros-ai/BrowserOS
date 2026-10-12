//! In-memory store of runs that parked waiting for the caller.
//!
//! A goal-driven run that needs a value it cannot generate stops and hands the
//! caller a handle. Until this existed the caller got a sentence and nothing
//! else, so answering meant starting a new run that observed, ranked and
//! decided from nothing.
//!
//! The handle is opaque and carries nothing. It arrives back as a tool argument
//! or, on the newer protocol revision, as the echoed `requestState`, and both
//! are client-supplied, so it is treated as attacker-controlled input: it is
//! looked up rather than parsed, it is refused for any session but the one that
//! parked it, it expires, and redeeming it removes it. Orphans left by a caller
//! that never answers are swept the next time anything parks, which bounds
//! stale memory without a background task, the way the human-help registry
//! does.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use browseros_decide::Run;
use tokio::sync::Mutex;
use ulid::Ulid;

use crate::ids::SessionId;

/// How long a parked run stays answerable.
///
/// Long enough for a caller to think, short enough that the page it parked on
/// has probably not moved. A longer window would mostly hand back runs whose
/// controls no longer exist, which the guard check then refuses anyway.
pub const CONTINUATION_TTL: Duration = Duration::from_secs(10 * 60);

/// How many parked runs one session may hold at once.
///
/// More than one, because a caller may legitimately pursue goals on two pages.
/// Bounded, because nothing else would stop a caller that parks and never
/// answers.
pub const PER_SESSION: usize = 4;

/// A run waiting for an answer, and what it was working on.
pub struct Parked {
    pub page: u32,
    pub run: Run,
}

/// Why a handle could not be redeemed.
///
/// Named rather than collapsed into one miss, because the three call for
/// different wording: an expired handle means start again, and a handle from
/// another session means the caller is holding someone else's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    Unknown,
    Expired,
    OtherSession,
    OtherPage,
}

struct Entry {
    session: SessionId,
    page: u32,
    run: Run,
    created: Instant,
}

pub struct ContinuationStore {
    entries: Mutex<HashMap<String, Entry>>,
    ttl: Duration,
    per_session: usize,
}

impl ContinuationStore {
    #[must_use]
    pub fn new(ttl: Duration, per_session: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl,
            per_session,
        }
    }

    /// Parks a run and returns the handle that answers it.
    pub async fn park(&self, session: &SessionId, page: u32, run: Run) -> String {
        let mut entries = self.entries.lock().await;
        let ttl = self.ttl;
        entries.retain(|_, entry| entry.created.elapsed() < ttl);
        self.evict_oldest_over_cap(&mut entries, session);
        let handle = format!("ctn-{}", Ulid::new());
        entries.insert(
            handle.clone(),
            Entry {
                session: session.clone(),
                page,
                run,
                created: Instant::now(),
            },
        );
        handle
    }

    /// Takes a parked run, if this session parked it on this page.
    ///
    /// Removes it either way it succeeds, so a handle is single use: the
    /// specification notes that bounding the replay window does not give single
    /// use, and that a server needing it has to enforce it itself.
    pub async fn redeem(
        &self,
        session: &SessionId,
        page: u32,
        handle: &str,
    ) -> Result<Parked, Refused> {
        let mut entries = self.entries.lock().await;
        let entry = entries.get(handle).ok_or(Refused::Unknown)?;
        if entry.created.elapsed() >= self.ttl {
            entries.remove(handle);
            return Err(Refused::Expired);
        }
        // Checked before removal, so a handle presented by the wrong session is
        // not destroyed for the session that owns it.
        if entry.session != *session {
            return Err(Refused::OtherSession);
        }
        if entry.page != page {
            return Err(Refused::OtherPage);
        }
        let entry = entries.remove(handle).ok_or(Refused::Unknown)?;
        Ok(Parked {
            page: entry.page,
            run: entry.run,
        })
    }

    /// Drops everything this session parked, for a session that has ended.
    pub async fn forget_session(&self, session: &SessionId) {
        let mut entries = self.entries.lock().await;
        entries.retain(|_, entry| entry.session != *session);
    }

    #[cfg(test)]
    async fn held(&self) -> usize {
        self.entries.lock().await.len()
    }

    fn evict_oldest_over_cap(&self, entries: &mut HashMap<String, Entry>, session: &SessionId) {
        loop {
            let mut mine: Vec<(&String, Instant)> = entries
                .iter()
                .filter(|(_, entry)| entry.session == *session)
                .map(|(handle, entry)| (handle, entry.created))
                .collect();
            if mine.len() < self.per_session {
                return;
            }
            mine.sort_by_key(|(_, created)| *created);
            let Some((oldest, _)) = mine.first() else {
                return;
            };
            let oldest = (*oldest).clone();
            entries.remove(&oldest);
        }
    }
}

impl Default for ContinuationStore {
    fn default() -> Self {
        Self::new(CONTINUATION_TTL, PER_SESSION)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browseros_decide::Budget;

    fn run() -> Run {
        Run::new(
            "sort by price",
            Budget {
                max_steps: 12,
                max_seconds: 30,
            },
        )
    }

    fn session(name: &str) -> SessionId {
        SessionId::new(name.to_string())
    }

    #[tokio::test]
    async fn a_parked_run_comes_back_once() -> anyhow::Result<()> {
        let store = ContinuationStore::default();
        let handle = store.park(&session("a"), 3, run()).await;

        let parked = store.redeem(&session("a"), 3, &handle).await;
        assert!(parked.is_ok());
        assert_eq!(
            store.redeem(&session("a"), 3, &handle).await.err(),
            Some(Refused::Unknown),
            "a handle is single use: bounding the replay window is not the same thing"
        );
        Ok(())
    }

    /// The handle arrives as client-supplied input, so it cannot be the only
    /// thing that decides whose run is continued.
    #[tokio::test]
    async fn a_handle_from_another_session_is_refused_and_left_alone() {
        let store = ContinuationStore::default();
        let handle = store.park(&session("a"), 3, run()).await;

        assert_eq!(
            store.redeem(&session("b"), 3, &handle).await.err(),
            Some(Refused::OtherSession)
        );
        assert!(
            store.redeem(&session("a"), 3, &handle).await.is_ok(),
            "and the session that owns it can still answer"
        );
    }

    /// A run is bound to the page it parked on. Continuing it somewhere else
    /// would carry a trail and a set of references that describe a page nobody
    /// is looking at.
    #[tokio::test]
    async fn a_handle_for_another_page_is_refused() {
        let store = ContinuationStore::default();
        let handle = store.park(&session("a"), 3, run()).await;

        assert_eq!(
            store.redeem(&session("a"), 9, &handle).await.err(),
            Some(Refused::OtherPage)
        );
    }

    #[tokio::test]
    async fn an_expired_handle_is_refused_and_dropped() {
        let store = ContinuationStore::new(Duration::ZERO, PER_SESSION);
        let handle = store.park(&session("a"), 3, run()).await;

        assert_eq!(
            store.redeem(&session("a"), 3, &handle).await.err(),
            Some(Refused::Expired)
        );
        assert_eq!(store.held().await, 0);
    }

    /// Nothing else stops a caller that parks and never answers.
    #[tokio::test]
    async fn the_per_session_cap_evicts_the_oldest() {
        let store = ContinuationStore::new(CONTINUATION_TTL, 2);
        let first = store.park(&session("a"), 3, run()).await;
        let second = store.park(&session("a"), 3, run()).await;
        let third = store.park(&session("a"), 3, run()).await;

        assert_eq!(
            store.redeem(&session("a"), 3, &first).await.err(),
            Some(Refused::Unknown),
            "the oldest went"
        );
        assert!(store.redeem(&session("a"), 3, &second).await.is_ok());
        assert!(store.redeem(&session("a"), 3, &third).await.is_ok());
    }

    /// One session filling its cap must not evict another's.
    #[tokio::test]
    async fn the_cap_is_per_session() {
        let store = ContinuationStore::new(CONTINUATION_TTL, 1);
        let mine = store.park(&session("a"), 3, run()).await;
        let theirs = store.park(&session("b"), 3, run()).await;

        assert!(store.redeem(&session("a"), 3, &mine).await.is_ok());
        assert!(store.redeem(&session("b"), 3, &theirs).await.is_ok());
    }

    #[tokio::test]
    async fn anything_a_client_invents_is_just_a_miss() {
        let store = ContinuationStore::default();

        for nonsense in ["", "ctn-", "../../etc/passwd", "ctn-01J0000000000000000000"] {
            assert_eq!(
                store.redeem(&session("a"), 3, nonsense).await.err(),
                Some(Refused::Unknown),
                "{nonsense} is not a handle"
            );
        }
    }

    #[tokio::test]
    async fn a_session_that_ended_leaves_nothing_behind() {
        let store = ContinuationStore::default();
        store.park(&session("a"), 3, run()).await;
        store.park(&session("b"), 3, run()).await;

        store.forget_session(&session("a")).await;

        assert_eq!(store.held().await, 1);
    }
}
