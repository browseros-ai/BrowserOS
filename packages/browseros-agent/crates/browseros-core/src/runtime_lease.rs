//! On-demand `Runtime` reporting for CDP target sessions (pages, OOPIFs, workers).
//!
//! Attaching a tab leaves `Runtime` off: while it is on, every console call the page makes
//! serializes its arguments for the inspector, and sites time that to detect DevTools and
//! blank or close themselves. A consumer that needs `Runtime`'s events on a session (a script
//! listening for console messages or page errors, an exposed binding, a worker's events) takes a
//! [`RuntimeLease`] for as long as it needs them:
//!
//! ```text
//!  acquire ── count 0 → 1 ── Runtime.enable    (the browser replays its stored history)
//!  acquire ── count 1 → 2 ── nothing
//!  drop    ── count 2 → 1 ── nothing
//!  drop    ── count 1 → 0 ── Runtime.disable
//! ```
//!
//! One turn lock per session orders every transition, and `PageSignals::harvest_console`'s own
//! enable + disable pair, so the wire order always matches the count: a harvest never switches
//! reporting off under a held lease, and a release never lands after a later acquire's enable.
//!
//! `Runtime.enable` replays the console messages and exceptions the browser stored before it.
//! Each lease span keeps the wall-clock floor its first acquirer named; consumers drop replayed
//! events stamped before it ([`RuntimeLeases::floor_ms`]), so a listener still sees only what
//! happened after it asked.

use crate::{CoreError, ProtocolSession, SessionId};
use serde_json::json;
use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};
use tokio::sync::{Mutex as TurnLock, OwnedMutexGuard};

/// Every session's lease count, shared by everything that drives one browser connection.
#[derive(Default)]
pub struct RuntimeLeases {
    sessions: Mutex<HashMap<SessionId, Arc<SessionLeases>>>,
}

impl fmt::Debug for RuntimeLeases {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeLeases").finish_non_exhaustive()
    }
}

/// One session's leases.
#[derive(Default)]
struct SessionLeases {
    /// Held across each transition's CDP command, so commands reach the wire in count order.
    turn: Arc<TurnLock<()>>,
    held: Mutex<Held>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Held {
    count: usize,
    /// Whether the last `Runtime.enable` succeeded with no disable since. Apart from `count`:
    /// a failed enable leaves a count until its release runs, and the next acquirer must still
    /// enable rather than trust that count.
    on: bool,
    /// Milliseconds since the epoch: the floor the span's opening acquirer named.
    floor_ms: f64,
}

impl SessionLeases {
    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Keeps `Runtime` enabled on one session until dropped. Dropping the last lease of a session
/// sends `Runtime.disable` from a spawned task, so a lease may drop anywhere, including when a
/// run's state is torn down; outside a Tokio runtime the disable is skipped.
#[must_use = "Runtime stays enabled only while the lease is held"]
pub struct RuntimeLease {
    entry: Arc<SessionLeases>,
    session: ProtocolSession,
}

impl fmt::Debug for RuntimeLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeLease")
            .field("session", &self.session.session_id())
            .finish_non_exhaustive()
    }
}

impl Drop for RuntimeLease {
    fn drop(&mut self) {
        let entry = Arc::clone(&self.entry);
        let session = self.session.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            drop(runtime.spawn(release(entry, session)));
        } else {
            let mut held = entry.held();
            held.count = held.count.saturating_sub(1);
        }
    }
}

async fn release(entry: Arc<SessionLeases>, session: ProtocolSession) {
    let _turn = Arc::clone(&entry.turn).lock_owned().await;
    let last = {
        let mut held = entry.held();
        held.count = held.count.saturating_sub(1);
        held.count == 0
    };
    if last {
        entry.held().on = false;
        // Best effort: a session that is gone has no reporting left to switch off.
        let _ = session.send_value("Runtime.disable", json!({})).await;
    }
}

/// Exclusive use of a session's `Runtime` toggle by a caller that holds no lease, granted only
/// while no lease is held (`PageSignals::harvest_console`'s enable + disable pair).
pub(crate) struct UnleasedTurn {
    _turn: OwnedMutexGuard<()>,
}

impl RuntimeLeases {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    fn entry(&self, session: &SessionId) -> Arc<SessionLeases> {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        // Sessions of closed targets leave idle entries behind; forget those as others come.
        sessions.retain(|id, entry| {
            id == session || Arc::strong_count(entry) > 1 || {
                let held = entry.held();
                held.count > 0
            }
        });
        Arc::clone(sessions.entry(session.clone()).or_default())
    }

    /// Enables `Runtime` on `session` unless a lease already holds it on, and holds it on until
    /// the returned lease drops. `floor_ms` (milliseconds since the epoch) is when the caller
    /// started to want events: replays stamped before it are history ([`Self::floor_ms`]). It
    /// applies only to the acquire that opens a span; within a span the browser replays nothing.
    ///
    /// Cancel-safe: the count is taken before `Runtime.enable` is sent, so a caller dropped
    /// mid-enable releases through the lease it never received.
    pub async fn acquire(
        &self,
        session: &ProtocolSession,
        floor_ms: f64,
    ) -> Result<RuntimeLease, CoreError> {
        let Some(id) = session.session_id() else {
            return Err(CoreError::Message(
                "a Runtime lease needs a target session".to_owned(),
            ));
        };
        let entry = self.entry(id);
        let turn = Arc::clone(&entry.turn).lock_owned().await;
        let opens = {
            let mut held = entry.held();
            held.count += 1;
            // Before the enable: the browser replays its history ahead of the reply.
            if !held.on {
                held.floor_ms = floor_ms;
            }
            !held.on
        };
        // The release must reach the session even after the tab left this caller's admission
        // (moved to another window, say), or reporting would stay on for good.
        let lease = RuntimeLease {
            entry,
            session: session.cancellation_endpoint(),
        };
        if opens {
            session.send_value("Runtime.enable", json!({})).await?;
            lease.entry.held().on = true;
        }
        drop(turn);
        Ok(lease)
    }

    /// Whether any lease holds `Runtime` on for `session`.
    #[must_use]
    pub fn is_held(&self, session: &SessionId) -> bool {
        self.floor_ms(session).is_some()
    }

    /// The current span's floor on `session`, while some lease is held: a `Runtime` event
    /// stamped (`timestamp`, milliseconds since the epoch) before it is the browser's replay
    /// of what happened before anyone asked, and must not reach a listener.
    #[must_use]
    pub fn floor_ms(&self, session: &SessionId) -> Option<f64> {
        let sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let held = *sessions.get(session)?.held();
        (held.count > 0).then_some(held.floor_ms)
    }

    /// The session's turn for a caller that toggles `Runtime` itself, or `None` while a lease
    /// holds it on (reporting is live then, and must stay on).
    pub(crate) async fn unleased_turn(&self, session: &SessionId) -> Option<UnleasedTurn> {
        let entry = self.entry(session);
        let turn = Arc::clone(&entry.turn).lock_owned().await;
        (entry.held().count == 0).then_some(UnleasedTurn { _turn: turn })
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeLeases;
    use crate::{ProtocolSession, SessionId, testing::TestConnection};
    use serde_json::json;

    fn runtime_calls(connection: &TestConnection) -> Vec<String> {
        connection
            .calls()
            .unwrap_or_default()
            .into_iter()
            .filter(|call| call.method.starts_with("Runtime."))
            .map(|call| call.method)
            .collect()
    }

    #[tokio::test]
    async fn the_first_lease_enables_and_the_last_release_disables()
    -> Result<(), Box<dyn std::error::Error>> {
        let connection = TestConnection::new(Vec::<(&'static str, serde_json::Value)>::new());
        connection.always("Runtime.enable", json!({}))?;
        connection.always("Runtime.disable", json!({}))?;
        let session = ProtocolSession::for_session(connection.clone(), SessionId::from("s-1"));
        let leases = RuntimeLeases::new();
        let id = SessionId::from("s-1");

        let first = leases.acquire(&session, 10.0).await?;
        let second = leases.acquire(&session, 20.0).await?;
        assert_eq!(runtime_calls(&connection), vec!["Runtime.enable"]);
        assert_eq!(
            leases.floor_ms(&id),
            Some(10.0),
            "the span keeps its opener's floor"
        );
        assert!(
            leases.unleased_turn(&id).await.is_none(),
            "a harvest must wait it out"
        );

        drop(first);
        tokio::task::yield_now().await;
        assert!(leases.is_held(&id));
        drop(second);
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            runtime_calls(&connection),
            vec!["Runtime.enable", "Runtime.disable"]
        );
        assert!(!leases.is_held(&id));
        assert!(leases.unleased_turn(&id).await.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn an_acquire_after_a_failed_enable_enables_again()
    -> Result<(), Box<dyn std::error::Error>> {
        let connection = TestConnection::new([
            ("Runtime.enable", crate::testing::cdp_error("busy")),
            ("Runtime.enable", json!({})),
        ]);
        connection.always("Runtime.disable", json!({}))?;
        let session = ProtocolSession::for_session(connection.clone(), SessionId::from("s-2"));
        let leases = RuntimeLeases::new();

        assert!(leases.acquire(&session, 1.0).await.is_err());
        // Its release is still queued; this acquire must not count on the failed enable.
        let lease = leases.acquire(&session, 2.0).await?;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            runtime_calls(&connection),
            vec!["Runtime.enable", "Runtime.enable"]
        );
        assert!(leases.is_held(&SessionId::from("s-2")));
        drop(lease);
        Ok(())
    }
}
