//! In-memory registry of pending human-help requests, one per session. A blocked agent opens a
//! request and waits (re-calling the wait tool in bounded chunks); the cockpit surfaces it and a
//! human hands control back, which resolves the request. Entries are ephemeral: resolved ones are
//! hidden from the cockpit and reaped, and any entry older than `max_wait` is reaped so a crashed
//! or non-looping agent cannot leave a stale request behind.

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
use tokio::sync::{Mutex, Notify};

/// What the agent said when it opened the request.
pub struct HelpOpenParams {
    pub request_id: String,
    pub reason: String,
    pub details: Option<String>,
    pub resume_hint: Option<String>,
    pub kind: Option<HelpRequestKind>,
}

/// The outcome the agent observes after waiting a chunk.
#[derive(Debug, PartialEq, Eq)]
pub enum HelpWaitOutcome {
    Resolved { note: Option<String> },
    Waiting { elapsed_seconds: i64 },
    Cancelled,
    TimedOut,
}

pub struct HelpEntry {
    request_id: String,
    reason: String,
    details: Option<String>,
    resume_hint: Option<String>,
    kind: Option<HelpRequestKind>,
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
}

impl HelpRegistry {
    #[must_use]
    pub fn new(max_wait: Duration) -> Self {
        Self {
            max_wait,
            entries: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn max_wait(&self) -> Duration {
        self.max_wait
    }

    /// Open a request, or return the existing one for this session so a re-call reattaches without
    /// resetting the waiting timer.
    pub async fn open(&self, session: &SessionId, params: HelpOpenParams) -> Arc<HelpEntry> {
        let mut entries = self.entries.lock().await;
        if let Some(existing) = entries.get(session)
            && !existing.resolved.load(Ordering::Acquire)
        {
            return existing.clone();
        }
        let entry = Arc::new(HelpEntry {
            request_id: params.request_id,
            reason: params.reason,
            details: params.details,
            resume_hint: params.resume_hint,
            kind: params.kind,
            requested_at_ms: now_ms(),
            created: Instant::now(),
            resolved: AtomicBool::new(false),
            note: StdMutex::new(None),
            notify: Notify::new(),
        });
        entries.insert(session.clone(), entry.clone());
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
                true
            }
            None => false,
        }
    }

    async fn remove(&self, session: &SessionId) {
        self.entries.lock().await.remove(session);
    }

    /// Wait up to `chunk` for this session's request to resolve. Honors cancellation and the
    /// overall `max_wait` cap, and removes the entry on any terminal outcome.
    pub async fn wait_chunk(
        &self,
        session: &SessionId,
        entry: &Arc<HelpEntry>,
        cancel: &tokio_util::sync::CancellationToken,
        chunk: Duration,
    ) -> HelpWaitOutcome {
        if entry.resolved.load(Ordering::Acquire) {
            let note = entry.take_note();
            self.remove(session).await;
            return HelpWaitOutcome::Resolved { note };
        }
        tokio::select! {
            () = entry.notify.notified() => {
                if entry.resolved.load(Ordering::Acquire) {
                    let note = entry.take_note();
                    self.remove(session).await;
                    HelpWaitOutcome::Resolved { note }
                } else {
                    HelpWaitOutcome::Waiting { elapsed_seconds: entry.created.elapsed().as_secs() as i64 }
                }
            }
            () = cancel.cancelled() => {
                self.remove(session).await;
                HelpWaitOutcome::Cancelled
            }
            () = tokio::time::sleep(chunk) => {
                if entry.created.elapsed() >= self.max_wait {
                    self.remove(session).await;
                    HelpWaitOutcome::TimedOut
                } else {
                    HelpWaitOutcome::Waiting { elapsed_seconds: entry.created.elapsed().as_secs() as i64 }
                }
            }
        }
    }

    /// Build the cockpit wire view for a session, or None when there is nothing to show. Reaps a
    /// resolved or expired entry so a stale request never lingers in the snapshot.
    pub async fn snapshot(
        &self,
        session: &SessionId,
        browser_tab_id: Option<i64>,
        url: Option<String>,
        title: Option<String>,
    ) -> Option<HelpRequest> {
        let mut entries = self.entries.lock().await;
        let entry = entries.get(session)?.clone();
        if entry.resolved.load(Ordering::Acquire) || entry.created.elapsed() >= self.max_wait {
            entries.remove(session);
            return None;
        }
        let tab = browser_tab_id?;
        let mut wire = HelpRequest::new(
            entry.request_id.clone(),
            entry.reason.clone(),
            tab,
            entry.requested_at_ms,
        );
        wire.details = entry.details.clone();
        wire.resume_hint = entry.resume_hint.clone();
        wire.kind = entry.kind;
        wire.url = url;
        wire.title = title;
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

    fn params(id: &str) -> HelpOpenParams {
        HelpOpenParams {
            request_id: id.to_string(),
            reason: "Enter the code".to_string(),
            details: None,
            resume_hint: Some("resume at 15".to_string()),
            kind: Some(HelpRequestKind::Login),
        }
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
        let outcome = registry
            .wait_chunk(
                &session,
                &entry,
                &CancellationToken::new(),
                Duration::from_secs(1),
            )
            .await;
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
        let outcome = registry
            .wait_chunk(
                &session,
                &entry,
                &CancellationToken::new(),
                Duration::from_millis(20),
            )
            .await;
        assert!(matches!(outcome, HelpWaitOutcome::Waiting { .. }));
        assert!(registry.get(&session).await.is_some());
    }

    #[tokio::test]
    async fn cancel_returns_cancelled_and_clears_the_entry() {
        let registry = HelpRegistry::new(Duration::from_secs(60));
        let session = SessionId::new("s1");
        let entry = registry.open(&session, params("req-1")).await;
        let cancel = CancellationToken::new();
        cancel.cancel();
        let outcome = registry
            .wait_chunk(&session, &entry, &cancel, Duration::from_secs(1))
            .await;
        assert_eq!(outcome, HelpWaitOutcome::Cancelled);
        assert!(registry.get(&session).await.is_none());
    }

    #[tokio::test]
    async fn chunk_past_the_cap_times_out_and_clears_the_entry() {
        let registry = HelpRegistry::new(Duration::from_millis(1));
        let session = SessionId::new("s1");
        let entry = registry.open(&session, params("req-1")).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        let outcome = registry
            .wait_chunk(
                &session,
                &entry,
                &CancellationToken::new(),
                Duration::from_millis(5),
            )
            .await;
        assert_eq!(outcome, HelpWaitOutcome::TimedOut);
        assert!(registry.get(&session).await.is_none());
    }

    #[tokio::test]
    async fn snapshot_shows_the_request_and_reaps_when_resolved_or_tabless() {
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
        // No owned tab to take over -> nothing to show.
        assert!(
            registry
                .snapshot(&session, None, None, None)
                .await
                .is_none()
        );
        // Resolved -> hidden and reaped.
        registry.resolve(&session, None).await;
        assert!(
            registry
                .snapshot(&session, Some(42), None, None)
                .await
                .is_none()
        );
        assert!(registry.get(&session).await.is_none());
    }

    #[tokio::test]
    async fn snapshot_reaps_an_entry_past_the_cap_without_any_wait_call() {
        let registry = HelpRegistry::new(Duration::from_millis(1));
        let session = SessionId::new("s1");
        registry.open(&session, params("req-1")).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        // A crashed or non-looping agent never re-waits; the snapshot still reaps it.
        assert!(
            registry
                .snapshot(&session, Some(42), None, None)
                .await
                .is_none()
        );
        assert!(registry.get(&session).await.is_none());
    }
}
