//! `Log` reporting on a page session, for a run listening to the page's console messages.
//!
//! Chromium reports what the browser itself logs (a `fetch` blocked by CORS, a frame refused by
//! `X-Frame-Options`, an intervention) as `Log.entryAdded`, not as a console API call. Upstream's
//! `crPage` enables `Log` on every page and emits each entry as a console message. Neo attaches
//! pages with inspector domains off (a site can time the inspector to detect it, see
//! `browseros_core::runtime_lease`), so `Log` rides on the `Runtime` lease a console listener or
//! wait already takes ([`ReportingLease`]): enabled after it, disabled before it, never on alone.
//! The tab's out-of-process frames report their own entries on their own sessions (a frame
//! refused for `X-Frame-Options`, measured); the target watcher leases `Log` there while a console
//! lease holds the tab ([`LogLeases::console_floor`], `target_watcher.rs` `FrameLog`), and the
//! console lease has the watcher take it off again before it lets `Runtime` go:
//!
//! ```text
//!  console listener or wait ── Runtime.enable ── Log.enable ── frames: Log.enable (watcher)
//!  its release              ── Log.disable ── frames: Log.disable (watcher) ── Runtime.disable
//! ```
//!
//! ```text
//!  acquire ── count 0 → 1 ── Log.enable    (Blink replays its stored entries before it replies)
//!  acquire ── count 1 → 2 ── nothing
//!  release ── count 2 → 1 ── nothing
//!  release ── count 1 → 0 ── Log.disable
//! ```
//!
//! As with core's `Runtime` leases, one turn lock per session puts each transition's command on
//! the wire in count order, and each span keeps the floor its opening acquirer named: the
//! watcher drops a replayed entry stamped before it ([`LogLeases::floor_ms`]).
//!
//! `addScriptTag` holds a plain [`LogLease`] for its insert, for the CSP reports that tell a
//! blocked tag (`page/add_tag.rs`); counted here, a console release cannot switch them off under
//! it. Entries only it asked for reach no listener: the watcher passes an entry on only while a
//! console lease holds the tab (`target_watcher.rs`, `log_entry`).

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::{CoreError, ProtocolSession, RuntimeLease, SessionId, TargetId};
use serde_json::json;
use tokio::runtime::Handle;
use tokio::sync::{Mutex as TurnLock, OwnedMutexGuard};

use crate::events::target_watcher::WatcherHandle;

/// Every page session's `Log` lease count, shared by all runs on one browser connection
/// (`EventHub::log_leases`).
#[derive(Default)]
pub(crate) struct LogLeases {
    sessions: Mutex<HashMap<SessionId, Arc<SessionLog>>>,
    /// The floor of each console lease that holds a tab, by the tab's target: while a tab has
    /// one, the watcher keeps `Log` on its out-of-process frames.
    consoles: Arc<Mutex<HashMap<TargetId, Vec<f64>>>>,
}

impl fmt::Debug for LogLeases {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogLeases").finish_non_exhaustive()
    }
}

/// One session's leases.
#[derive(Default)]
struct SessionLog {
    /// Held across each transition's CDP command, so commands reach the wire in count order.
    turn: Arc<TurnLock<()>>,
    held: Mutex<Held>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Held {
    count: usize,
    /// Whether the last `Log.enable` succeeded with no disable since. Apart from `count`: a
    /// failed enable leaves a count until its release runs, and the next acquirer must still
    /// enable rather than trust that count.
    on: bool,
    /// Milliseconds since the epoch: the floor the span's opening acquirer named.
    floor_ms: f64,
}

impl SessionLog {
    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Keeps `Log` enabled on one session until released. A page session's is held inside the
/// [`ReportingLease`] that orders its release before the `Runtime` lease's; an out-of-process
/// frame's by the watcher, while such a lease holds the frame's page.
#[must_use = "Log stays enabled only while the lease is held"]
pub(crate) struct LogLease {
    /// `None` once released.
    held: Option<Releasing>,
}

/// What releasing one lease needs: the session's entry, and an endpoint that still reaches the
/// session after the tab left the acquiring op's admission.
struct Releasing {
    entry: Arc<SessionLog>,
    session: ProtocolSession,
}

impl fmt::Debug for LogLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogLease")
            .field(
                "session",
                &self
                    .held
                    .as_ref()
                    .and_then(|held| held.session.session_id()),
            )
            .finish_non_exhaustive()
    }
}

impl LogLease {
    /// Gives the lease back, switching `Log` off when it was the session's last. Awaited, so
    /// the caller knows the `Log.disable` went before what it does next.
    pub(crate) async fn release(mut self) {
        if let Some(held) = self.held.take() {
            held.release().await;
        }
    }
}

impl Drop for LogLease {
    /// A lease dropped unreleased (an acquire cancelled mid-enable) releases from a spawned
    /// task; outside a Tokio runtime only the count is given back.
    fn drop(&mut self) {
        let Some(held) = self.held.take() else {
            return;
        };
        if let Ok(runtime) = Handle::try_current() {
            drop(runtime.spawn(held.release()));
        } else {
            let mut count = held.entry.held();
            count.count = count.count.saturating_sub(1);
        }
    }
}

impl Releasing {
    async fn release(self) {
        let _turn = Arc::clone(&self.entry.turn).lock_owned().await;
        let last = {
            let mut held = self.entry.held();
            held.count = held.count.saturating_sub(1);
            held.count == 0
        };
        if last {
            self.entry.held().on = false;
            // Best effort: a session that is gone has no reporting left to switch off.
            let _ = self.session.send_value("Log.disable", json!({})).await;
        }
    }
}

impl LogLeases {
    fn entry(&self, session: &SessionId) -> Arc<SessionLog> {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        // Sessions of closed targets leave idle entries behind; forget those as others come.
        sessions.retain(|id, entry| {
            id == session || Arc::strong_count(entry) > 1 || entry.held().count > 0
        });
        Arc::clone(sessions.entry(session.clone()).or_default())
    }

    /// Enables `Log` on `session` unless a lease already holds it on, and holds it on until the
    /// returned lease is released. `floor_ms` (milliseconds since the epoch) is when the caller
    /// started to want entries; it applies only to the acquire that opens a span, since within a
    /// span the browser replays nothing.
    ///
    /// Cancel-safe: the count is taken before `Log.enable` is sent, so a caller dropped
    /// mid-enable releases through the lease it never received.
    pub(crate) async fn acquire(
        &self,
        session: &ProtocolSession,
        floor_ms: f64,
    ) -> Result<LogLease, CoreError> {
        let (lease, opening) = self.count(session, floor_ms).await?;
        if let Some(turn) = opening {
            session.send_value("Log.enable", json!({})).await?;
            turn.opened();
        }
        Ok(lease)
    }

    /// [`Self::acquire`] on a console listener's or wait's page session, on top of its `Runtime`
    /// lease, holding tab `tab` for its frames from `floor_ms` on. The pair is one
    /// [`ReportingLease`] before `Log.enable` is sent, so even a cancelled acquire releases `Log`,
    /// the frames' `Log` and then `Runtime`, in that order. A refused enable answers the lease
    /// without page `Log`, and why.
    pub(crate) async fn acquire_console(
        &self,
        runtime: RuntimeLease,
        session: &ProtocolSession,
        tab: TargetId,
        floor_ms: f64,
        watcher: WatcherHandle,
    ) -> Result<ReportingLease, Box<LogRefused>> {
        let (log, opening) = match self.count(session, floor_ms).await {
            Ok(counted) => counted,
            Err(error) => {
                return Err(Box::new(LogRefused {
                    lease: runtime.into(),
                    error,
                }));
            }
        };
        lock(&self.consoles)
            .entry(tab.clone())
            .or_default()
            .push(floor_ms);
        let mut lease = ReportingLease {
            log: Some(log),
            runtime: Some(runtime),
            console: Some(ConsoleHold {
                consoles: Arc::clone(&self.consoles),
                tab,
                floor_ms,
                watcher,
            }),
        };
        if let Some(turn) = opening {
            if let Err(error) = session.send_value("Log.enable", json!({})).await {
                // The release takes the turn this acquire still holds.
                drop(turn);
                if let Some(log) = lease.log.take() {
                    log.release().await;
                }
                return Err(Box::new(LogRefused { lease, error }));
            }
            turn.opened();
        }
        Ok(lease)
    }

    /// Takes one count on `session`'s `Log` under its turn, and the turn itself when this count
    /// opens a span, whose `Log.enable` the caller sends and confirms ([`Opening::opened`]).
    async fn count(
        &self,
        session: &ProtocolSession,
        floor_ms: f64,
    ) -> Result<(LogLease, Option<Opening>), CoreError> {
        let Some(id) = session.session_id() else {
            return Err(CoreError::Message(
                "a Log lease needs a target session".to_owned(),
            ));
        };
        let entry = self.entry(id);
        let turn = Arc::clone(&entry.turn).lock_owned().await;
        let opens = {
            let mut held = entry.held();
            held.count += 1;
            // Before the enable: the browser replays its stored entries ahead of the reply.
            if !held.on {
                held.floor_ms = floor_ms;
            }
            !held.on
        };
        let lease = LogLease {
            held: Some(Releasing {
                entry: Arc::clone(&entry),
                session: session.cancellation_endpoint(),
            }),
        };
        Ok((lease, opens.then_some(Opening { entry, _turn: turn })))
    }

    /// The earliest floor of the console leases that hold tab `tab` (the target of its page), or
    /// `None` while none does: how far back a frame's `Log.enable` replay still counts.
    pub(crate) fn console_floor(&self, tab: &TargetId) -> Option<f64> {
        lock(&self.consoles)
            .get(tab)?
            .iter()
            .copied()
            .reduce(f64::min)
    }

    /// The current span's floor on `session`, while some lease is held: an entry stamped
    /// (`timestamp`, milliseconds since the epoch) before it is the browser's replay of what
    /// it logged before anyone asked. `None` while no lease holds `Log` on: no run asked.
    pub(crate) fn floor_ms(&self, session: &SessionId) -> Option<f64> {
        let sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let held = *sessions.get(session)?.held();
        (held.count > 0).then_some(held.floor_ms)
    }
}

/// The turn of a session whose count just opened a span, held until its `Log.enable` replied.
struct Opening {
    entry: Arc<SessionLog>,
    _turn: OwnedMutexGuard<()>,
}

impl Opening {
    /// The enable replied: the span is on, and the turn goes.
    fn opened(self) {
        self.entry.held().on = true;
    }
}

/// A console lease whose page refused `Log.enable`: the lease without page `Log`, and why.
pub(crate) struct LogRefused {
    pub(crate) lease: ReportingLease,
    pub(crate) error: CoreError,
}

/// The reporting one listener or wait keeps on a page session: `Runtime`, and for console
/// messages `Log` on top of it, on the page and its frames. Dropping it disables page `Log`, has
/// the watcher take the frames' `Log` off, and only then lets `Runtime` go, in one spawned task,
/// so `Log` is never on without the `Runtime` lease that brought it.
#[derive(Debug)]
#[must_use = "Runtime and Log stay enabled only while the lease is held"]
pub(crate) struct ReportingLease {
    log: Option<LogLease>,
    runtime: Option<RuntimeLease>,
    console: Option<ConsoleHold>,
}

/// A console lease's hold on its tab's frames ([`LogLeases::console_floor`]).
struct ConsoleHold {
    consoles: Arc<Mutex<HashMap<TargetId, Vec<f64>>>>,
    tab: TargetId,
    floor_ms: f64,
    /// Reconciles the frames' `Log` once this hold is gone (`WatcherHandle::synchronize`).
    watcher: WatcherHandle,
}

impl fmt::Debug for ConsoleHold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleHold")
            .field("tab", &self.tab)
            .finish_non_exhaustive()
    }
}

impl ConsoleHold {
    /// Takes this hold's floor off its tab.
    fn release(&self) {
        let mut consoles = lock(&self.consoles);
        if let Some(floors) = consoles.get_mut(&self.tab) {
            let mine = self.floor_ms.to_bits();
            if let Some(at) = floors.iter().position(|floor| floor.to_bits() == mine) {
                floors.swap_remove(at);
            }
            if floors.is_empty() {
                consoles.remove(&self.tab);
            }
        }
    }
}

impl From<RuntimeLease> for ReportingLease {
    fn from(runtime: RuntimeLease) -> Self {
        Self {
            log: None,
            runtime: Some(runtime),
            console: None,
        }
    }
}

impl Drop for ReportingLease {
    fn drop(&mut self) {
        let (log, runtime, console) = (self.log.take(), self.runtime.take(), self.console.take());
        if log.is_none() && console.is_none() {
            return;
        }
        let Ok(handle) = Handle::try_current() else {
            // Outside a runtime the drops only give their counts back.
            if let Some(console) = console {
                console.release();
            }
            return;
        };
        drop(handle.spawn(async move {
            if let Some(log) = log {
                log.release().await;
            }
            if let Some(console) = console {
                console.release();
                // Bounded: the watcher answers within 2 s, or is gone.
                console.watcher.synchronize().await;
            }
            // Its own drop sends `Runtime.disable`, after every `Log.disable` went.
            drop(runtime);
        }));
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use browseros_core::testing::TestConnection;
    use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection, SessionId, TargetId};
    use serde_json::json;
    use tokio::task;

    use super::LogLeases;
    use crate::events::WatcherHandle;

    /// Sends `Log.enable` on the first lease and `Log.disable` after the last, and keeps the
    /// span's first floor. A console lease holds its tab for the frames at its own floor, and
    /// its release takes `Log` and the tab's hold off before it lets `Runtime` go.
    #[tokio::test]
    async fn tests_that_log_follows_its_lease_count_and_goes_off_before_runtime()
    -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([("Browser.getVersion", json!({}))]);
        for method in [
            "Log.enable",
            "Log.disable",
            "Runtime.enable",
            "Runtime.disable",
        ] {
            connection.always(method, json!({}))?;
        }
        let browser = BrowserSession::new(
            Arc::clone(&connection) as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        );
        let id = SessionId::from("page");
        let session = browser.protocol_session(id.clone());
        let logs = LogLeases::default();
        assert_eq!(logs.floor_ms(&id), None, "nothing asked");

        let first = logs.acquire(&session, 1_000.0).await?;
        let second = logs.acquire(&session, 2_000.0).await?;
        assert_eq!(logs.floor_ms(&id), Some(1_000.0), "the span's first floor");
        first.release().await;
        assert_eq!(
            logs.floor_ms(&id),
            Some(1_000.0),
            "one lease still holds it"
        );
        second.release().await;
        assert_eq!(logs.floor_ms(&id), None, "the last release ends the span");

        let runtime = browser
            .page_signals
            .runtime_leases()
            .acquire(&session, 3_000.0)
            .await?;
        let tab = TargetId("tab".to_owned());
        let console = logs
            .acquire_console(
                runtime,
                &session,
                tab.clone(),
                3_000.0,
                WatcherHandle::detached(),
            )
            .await
            .map_err(|refused| refused.error)?;
        assert_eq!(logs.console_floor(&tab), Some(3_000.0), "the frames' floor");
        drop(console);
        for _ in 0..50 {
            task::yield_now().await;
        }
        let sent: Vec<String> = connection
            .calls()?
            .into_iter()
            .filter(|call| call.session.as_ref() == Some(&id))
            .map(|call| call.method)
            .collect();
        assert_eq!(
            sent,
            [
                "Log.enable",
                "Log.disable",
                "Runtime.enable",
                "Log.enable",
                "Log.disable",
                "Runtime.disable",
            ],
            "one enable per span, and Log off before Runtime"
        );
        assert_eq!(
            logs.console_floor(&tab),
            None,
            "the tab's frames are let go"
        );
        Ok(())
    }
}
