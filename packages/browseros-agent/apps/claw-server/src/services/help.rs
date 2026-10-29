//! In-memory registry of pending human-help requests, one per session. A blocked agent opens a
//! request and waits (re-calling the wait tool in bounded chunks); the cockpit surfaces it and a
//! human hands control back, which resolves the request.
//!
//! Lifecycle: an entry is removed only when the agent's own wait call consumes a terminal outcome
//! (resolved, timed out) or a hard cancel (cockpit Stop, session teardown) fires. The cockpit
//! snapshot is a pure read that hides a resolved or expired entry without removing it, so a poll
//! between the agent's bounded wait calls can never drop a hand-back note or turn a timeout into a
//! false resume. Orphans left by a crashed or disconnected agent are swept the next time any
//! session opens a request, bounding stale memory without a background task.

use crate::ids::SessionId;
use claw_api::models::{HelpRequest, HelpRequestKind};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, Notify, watch};

/// Who opened a request. A person who takes over a page from the working overlay opens one so the
/// agent waits for the hand back with `await_human_help`, the same way it waits for help it asked
/// for. Person requests stay inside the server: the cockpit never shows them, because the person is
/// already on the page and hands back there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpOrigin {
    Agent,
    Person,
}

/// What the agent said when it opened the request, plus the tab it was blocked on, captured at
/// request time so takeover targets the exact page even if the agent has several tabs open.
pub struct HelpOpenParams {
    pub request_id: String,
    pub reason: String,
    pub details: Option<String>,
    pub resume_hint: Option<String>,
    pub kind: Option<HelpRequestKind>,
    pub browser_tab_id: Option<i64>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub origin: HelpOrigin,
}

/// The outcome the agent observes after waiting a chunk.
#[derive(Debug, PartialEq, Eq)]
pub enum HelpWaitOutcome {
    Resolved {
        note: Option<String>,
    },
    Waiting {
        elapsed_seconds: i64,
    },
    /// A hard cancel (cockpit Stop or session teardown): the request is discarded.
    Cancelled,
    /// This wait call was cancelled by the client (its own timeout or a disconnect). The request
    /// is preserved so the agent's next wait call reattaches instead of losing the request.
    Interrupted,
    TimedOut,
}

pub struct HelpEntry {
    request_id: String,
    reason: String,
    details: Option<String>,
    resume_hint: Option<String>,
    kind: Option<HelpRequestKind>,
    browser_tab_id: Option<i64>,
    url: Option<String>,
    title: Option<String>,
    origin: HelpOrigin,
    requested_at_ms: i64,
    created: Instant,
    resolved: AtomicBool,
    note: StdMutex<Option<String>>,
    notify: Notify,
}

impl HelpEntry {
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    #[must_use]
    pub fn origin(&self) -> HelpOrigin {
        self.origin
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        !self.resolved.load(Ordering::Acquire)
    }

    fn mark_resolved(&self, note: Option<String>) {
        if let Ok(mut slot) = self.note.lock() {
            *slot = note;
        }
        self.resolved.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    fn take_note(&self) -> Option<String> {
        self.note.lock().ok().and_then(|slot| slot.clone())
    }
}

pub struct HelpRegistry {
    max_wait: Duration,
    entries: Mutex<HashMap<SessionId, Arc<HelpEntry>>>,
    /// Bumped whenever what the cockpit would show changes, so a listener can re-read instead of
    /// polling: a request opened, resolved, removed, or reaching its cap.
    changes: Arc<watch::Sender<u64>>,
}

impl HelpRegistry {
    #[must_use]
    pub fn new(max_wait: Duration) -> Self {
        Self {
            max_wait,
            entries: Mutex::new(HashMap::new()),
            changes: Arc::new(watch::Sender::new(0)),
        }
    }

    /// A receiver whose value changes whenever the open help requests may have changed.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn announce_change(&self) {
        self.changes
            .send_modify(|version| *version = version.wrapping_add(1));
    }

    #[must_use]
    pub fn max_wait(&self) -> Duration {
        self.max_wait
    }

    /// Open a request, or return the existing one for this session so a re-call reattaches without
    /// resetting the waiting timer. Also sweeps orphaned entries (past the cap, left by a crashed
    /// or disconnected agent) so stale memory cannot accumulate.
    ///
    /// An agent's request replaces a person's take over: the person is already on the page, and
    /// the agent's question is what they should see now.
    pub async fn open(&self, session: &SessionId, params: HelpOpenParams) -> Arc<HelpEntry> {
        let mut entries = self.entries.lock().await;
        let max_wait = self.max_wait;
        entries.retain(|_, entry| entry.created.elapsed() < max_wait);
        if let Some(existing) = entries.get(session)
            && !existing.resolved.load(Ordering::Acquire)
            && !(existing.origin == HelpOrigin::Person && params.origin == HelpOrigin::Agent)
        {
            return existing.clone();
        }
        let origin = params.origin;
        let entry = Arc::new(HelpEntry {
            request_id: params.request_id,
            reason: params.reason,
            details: params.details,
            resume_hint: params.resume_hint,
            kind: params.kind,
            browser_tab_id: params.browser_tab_id,
            url: params.url,
            title: params.title,
            origin: params.origin,
            requested_at_ms: now_ms(),
            created: Instant::now(),
            resolved: AtomicBool::new(false),
            note: StdMutex::new(None),
            notify: Notify::new(),
        });
        entries.insert(session.clone(), entry.clone());
        drop(entries);
        if origin == HelpOrigin::Person {
            return entry;
        }
        self.announce_change();
        // An expired request disappears from the snapshot without any call, so say so when the
        // cap passes; a listener re-reads and drops it.
        let changes = self.changes.clone();
        tokio::spawn(async move {
            tokio::time::sleep(max_wait).await;
            changes.send_modify(|version| *version = version.wrapping_add(1));
        });
        entry
    }

    pub async fn get(&self, session: &SessionId) -> Option<Arc<HelpEntry>> {
        self.entries.lock().await.get(session).cloned()
    }

    /// Hand control back to a waiting agent. Returns true when a pending request was found.
    pub async fn resolve(&self, session: &SessionId, note: Option<String>) -> bool {
        let entry = { self.entries.lock().await.get(session).cloned() };
        match entry {
            Some(entry) => {
                entry.mark_resolved(note);
                if entry.origin == HelpOrigin::Agent {
                    self.announce_change();
                }
                true
            }
            None => false,
        }
    }

    /// Remove the entry only if it is still the one this waiter owns, so a stale waiter from an
    /// already-resolved request cannot delete a newer request opened for the same session.
    async fn remove_if_current(&self, session: &SessionId, entry: &Arc<HelpEntry>) {
        let mut entries = self.entries.lock().await;
        if entries
            .get(session)
            .is_some_and(|current| Arc::ptr_eq(current, entry))
        {
            entries.remove(session);
            drop(entries);
            if entry.origin == HelpOrigin::Agent {
                self.announce_change();
            }
        }
    }

    /// The terminal outcome to hand the waiting agent if the request has resolved or outlived the
    /// cap, removing the entry as it is consumed. None while the request is still live.
    async fn terminal_state(
        &self,
        session: &SessionId,
        entry: &Arc<HelpEntry>,
    ) -> Option<HelpWaitOutcome> {
        if entry.resolved.load(Ordering::Acquire) {
            let note = entry.take_note();
            self.remove_if_current(session, entry).await;
            return Some(HelpWaitOutcome::Resolved { note });
        }
        if entry.created.elapsed() >= self.max_wait {
            self.remove_if_current(session, entry).await;
            return Some(HelpWaitOutcome::TimedOut);
        }
        None
    }

    /// Wait up to `chunk` for this session's request to resolve. `terminal_cancel` (cockpit Stop or
    /// session teardown) discards the request; `call_cancel` (this wait call's own cancellation)
    /// leaves it in place so the agent's next call reattaches.
    pub async fn wait_chunk(
        &self,
        session: &SessionId,
        entry: &Arc<HelpEntry>,
        chunk: Duration,
        terminal_cancel: &tokio_util::sync::CancellationToken,
        call_cancel: &tokio_util::sync::CancellationToken,
    ) -> HelpWaitOutcome {
        if let Some(outcome) = self.terminal_state(session, entry).await {
            return outcome;
        }
        let waiting = || HelpWaitOutcome::Waiting {
            elapsed_seconds: entry.created.elapsed().as_secs() as i64,
        };
        tokio::select! {
            () = entry.notify.notified() => {
                self.terminal_state(session, entry).await.unwrap_or_else(waiting)
            }
            () = terminal_cancel.cancelled() => {
                self.remove_if_current(session, entry).await;
                HelpWaitOutcome::Cancelled
            }
            () = call_cancel.cancelled() => HelpWaitOutcome::Interrupted,
            () = tokio::time::sleep(chunk) => {
                self.terminal_state(session, entry).await.unwrap_or_else(waiting)
            }
        }
    }

    /// Build the cockpit wire view for a session, or None when there is nothing to show. A pure
    /// read: a resolved or expired entry is hidden but left in place for the agent's own wait call
    /// to consume, so a poll between wait calls cannot drop a hand-back note or mask a timeout.
    ///
    /// The tab pinned at request time wins; the `fallback_*` values (the poll-time most-active owned
    /// tab) only fill in when nothing was captured then, so takeover targets the blocked page even
    /// when the agent has several tabs open.
    pub async fn snapshot(
        &self,
        session: &SessionId,
        fallback_tab_id: Option<i64>,
        fallback_url: Option<String>,
        fallback_title: Option<String>,
    ) -> Option<HelpRequest> {
        let entries = self.entries.lock().await;
        let entry = entries.get(session)?.clone();
        if entry.origin == HelpOrigin::Person
            || entry.resolved.load(Ordering::Acquire)
            || entry.created.elapsed() >= self.max_wait
        {
            return None;
        }
        let tab = entry.browser_tab_id.or(fallback_tab_id)?;
        let mut wire = HelpRequest::new(
            entry.request_id.clone(),
            entry.reason.clone(),
            tab,
            entry.requested_at_ms,
        );
        wire.details = entry.details.clone();
        wire.resume_hint = entry.resume_hint.clone();
        wire.kind = entry.kind;
        wire.url = entry.url.clone().or(fallback_url);
        wire.title = entry.title.clone().or(fallback_title);
        Some(wire)
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn listeners_hear_open_resolve_and_removal() {
        let registry = HelpRegistry::new(Duration::from_secs(60));
        let session = SessionId::new("s1".to_string());
        let mut changes = registry.subscribe();
        let start = *changes.borrow_and_update();

        let entry = registry.open(&session, params("r1")).await;
        assert!(changes.has_changed().is_ok_and(|changed| changed));
        assert_eq!(*changes.borrow_and_update(), start + 1);

        assert!(registry.resolve(&session, None).await);
        assert_eq!(*changes.borrow_and_update(), start + 2);

        let outcome = wait(&registry, &session, &entry, Duration::from_millis(10)).await;
        assert_eq!(outcome, HelpWaitOutcome::Resolved { note: None });
        assert_eq!(*changes.borrow_and_update(), start + 3);
    }

    #[tokio::test(start_paused = true)]
    async fn listeners_hear_a_request_reach_its_cap() {
        let registry = HelpRegistry::new(Duration::from_secs(60));
        let session = SessionId::new("s1".to_string());
        let mut changes = registry.subscribe();
        registry.open(&session, params("r1")).await;
        changes.borrow_and_update();

        tokio::time::sleep(Duration::from_secs(61)).await;
        assert!(changes.has_changed().is_ok_and(|changed| changed));
    }

    #[tokio::test]
    async fn an_agent_request_replaces_a_take_over_but_not_the_other_way_round() {
        let registry = HelpRegistry::new(Duration::from_secs(60));
        let session = SessionId::new("s1".to_string());
        let person = HelpOpenParams {
            origin: HelpOrigin::Person,
            ..params("takeover")
        };
        registry.open(&session, person).await;
        assert_eq!(registry.snapshot(&session, Some(1), None, None).await, None);
        let agent = registry.open(&session, params("help")).await;
        assert_eq!(agent.request_id(), "help");
        assert!(
            registry
                .snapshot(&session, Some(1), None, None)
                .await
                .is_some()
        );
        let kept = registry
            .open(
                &session,
                HelpOpenParams {
                    origin: HelpOrigin::Person,
                    ..params("takeover-2")
                },
            )
            .await;
        assert_eq!(kept.request_id(), "help");
    }

    fn params(id: &str) -> HelpOpenParams {
        HelpOpenParams {
            request_id: id.to_string(),
            reason: "Enter the code".to_string(),
            details: None,
            resume_hint: Some("resume at 15".to_string()),
            kind: Some(HelpRequestKind::Login),
            browser_tab_id: None,
            url: None,
            title: None,
            origin: HelpOrigin::Agent,
        }
    }

    /// Wait a chunk with neither cancel token armed, the common case in tests.
    async fn wait(
        registry: &HelpRegistry,
        session: &SessionId,
        entry: &Arc<HelpEntry>,
        chunk: Duration,
    ) -> HelpWaitOutcome {
        registry
            .wait_chunk(
                session,
                entry,
                chunk,
                &CancellationToken::new(),
                &CancellationToken::new(),
            )
            .await
    }

    #[tokio::test]
    async fn open_reattaches_without_resetting_the_request() {
        let registry = HelpRegistry::new(Duration::from_secs(60));
        let session = SessionId::new("s1");
        let first = registry.open(&session, params("req-1")).await;
        let second = registry.open(&session, params("req-2")).await;
        assert_eq!(first.request_id(), "req-1");
        assert_eq!(second.request_id(), "req-1");
    }

    #[tokio::test]
    async fn resolve_makes_the_wait_return_resolved_with_the_note() {
        let registry = HelpRegistry::new(Duration::from_secs(60));
        let session = SessionId::new("s1");
        let entry = registry.open(&session, params("req-1")).await;
        assert!(
            registry
                .resolve(&session, Some("2fa done".to_string()))
                .await
        );
        let outcome = wait(&registry, &session, &entry, Duration::from_secs(1)).await;
        assert_eq!(
            outcome,
            HelpWaitOutcome::Resolved {
                note: Some("2fa done".to_string())
            }
        );
        assert!(registry.get(&session).await.is_none());
    }

    #[tokio::test]
    async fn chunk_returns_waiting_and_keeps_the_entry() {
        let registry = HelpRegistry::new(Duration::from_secs(60));
        let session = SessionId::new("s1");
        let entry = registry.open(&session, params("req-1")).await;
        let outcome = wait(&registry, &session, &entry, Duration::from_millis(20)).await;
        assert!(matches!(outcome, HelpWaitOutcome::Waiting { .. }));
        assert!(registry.get(&session).await.is_some());
    }

    #[tokio::test]
    async fn terminal_cancel_returns_cancelled_and_clears_the_entry() {
        let registry = HelpRegistry::new(Duration::from_secs(60));
        let session = SessionId::new("s1");
        let entry = registry.open(&session, params("req-1")).await;
        let terminal = CancellationToken::new();
        terminal.cancel();
        let outcome = registry
            .wait_chunk(
                &session,
                &entry,
                Duration::from_secs(1),
                &terminal,
                &CancellationToken::new(),
            )
            .await;
        assert_eq!(outcome, HelpWaitOutcome::Cancelled);
        assert!(registry.get(&session).await.is_none());
    }

    #[tokio::test]
    async fn per_call_cancel_interrupts_but_keeps_the_request() {
        let registry = HelpRegistry::new(Duration::from_secs(60));
        let session = SessionId::new("s1");
        let entry = registry.open(&session, params("req-1")).await;
        let call = CancellationToken::new();
        call.cancel();
        let outcome = registry
            .wait_chunk(
                &session,
                &entry,
                Duration::from_secs(1),
                &CancellationToken::new(),
                &call,
            )
            .await;
        assert_eq!(outcome, HelpWaitOutcome::Interrupted);
        // The request survives so the agent's next call reattaches.
        assert!(registry.get(&session).await.is_some());
    }

    #[tokio::test]
    async fn chunk_past_the_cap_times_out_and_clears_the_entry() {
        let registry = HelpRegistry::new(Duration::from_millis(1));
        let session = SessionId::new("s1");
        let entry = registry.open(&session, params("req-1")).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        let outcome = wait(&registry, &session, &entry, Duration::from_millis(5)).await;
        assert_eq!(outcome, HelpWaitOutcome::TimedOut);
        assert!(registry.get(&session).await.is_none());
    }

    #[tokio::test]
    async fn a_stale_waiter_does_not_delete_a_newer_request() {
        let registry = HelpRegistry::new(Duration::from_secs(60));
        let session = SessionId::new("s1");
        let first = registry.open(&session, params("req-1")).await;
        // First request is resolved and consumed by its waiter, which removes it.
        registry.resolve(&session, None).await;
        wait(&registry, &session, &first, Duration::from_secs(1)).await;
        // A new request opens for the same session.
        let second = registry.open(&session, params("req-2")).await;
        assert_eq!(second.request_id(), "req-2");
        // A late cleanup from the first waiter must not delete the second.
        registry.remove_if_current(&session, &first).await;
        let Some(current) = registry.get(&session).await else {
            panic!("the newer request must survive");
        };
        assert_eq!(current.request_id(), "req-2");
    }

    #[tokio::test]
    async fn snapshot_shows_then_hides_without_removing_for_the_agent() {
        let registry = HelpRegistry::new(Duration::from_secs(60));
        let session = SessionId::new("s1");
        registry.open(&session, params("req-1")).await;
        let Some(wire) = registry
            .snapshot(&session, Some(42), Some("https://x".to_string()), None)
            .await
        else {
            panic!("a pending request");
        };
        assert_eq!(wire.browser_tab_id, 42);
        assert_eq!(wire.resume_hint.as_deref(), Some("resume at 15"));
        // No owned tab to take over -> nothing to show, entry untouched.
        assert!(
            registry
                .snapshot(&session, None, None, None)
                .await
                .is_none()
        );
        // Resolved -> hidden from the cockpit but kept so the agent's wait consumes the note.
        registry
            .resolve(&session, Some("2fa done".to_string()))
            .await;
        assert!(
            registry
                .snapshot(&session, Some(42), None, None)
                .await
                .is_none()
        );
        assert!(registry.get(&session).await.is_some());
    }

    #[tokio::test]
    async fn snapshot_prefers_the_tab_pinned_at_request_time() {
        let registry = HelpRegistry::new(Duration::from_secs(60));
        let session = SessionId::new("s1");
        let mut pinned = params("req-1");
        pinned.browser_tab_id = Some(7);
        pinned.url = Some("https://blocked".to_string());
        registry.open(&session, pinned).await;
        // The poll-time fallback points at a different, more-active tab; the pinned tab wins.
        let Some(wire) = registry
            .snapshot(&session, Some(42), Some("https://other".to_string()), None)
            .await
        else {
            panic!("a pending request");
        };
        assert_eq!(wire.browser_tab_id, 7);
        assert_eq!(wire.url.as_deref(), Some("https://blocked"));
    }

    #[tokio::test]
    async fn open_sweeps_orphaned_entries_past_the_cap() {
        let registry = HelpRegistry::new(Duration::from_millis(1));
        let orphan = SessionId::new("ended");
        registry.open(&orphan, params("req-1")).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        // A later request from any session is the independent expiry path that clears the orphan.
        registry
            .open(&SessionId::new("fresh"), params("req-2"))
            .await;
        assert!(registry.get(&orphan).await.is_none());
    }
}
