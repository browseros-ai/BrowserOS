//! Keeps the agent off a tab while its owner has it (design D13's lock).
//!
//! Three kinds of caller reach the lock table, at three costs:
//! - the dispatcher's lock policy asks [`Locks::check`] about every agent operation, at both of
//!   its policy stages, and [`Locks::check_at`] answers the same question for a past instant,
//!   such as when a page event the agent reads was recorded. Both are synchronous: a pure fold
//!   (`lock_decision`) over the locks, under a `std::sync::Mutex` that no code holds across an
//!   await. The agent's hot path never waits on I/O, or on a host that is taking a lock;
//! - the host's routes take, read and release locks;
//! - tab cleanup asks [`Locks::is_locked`], and never closes a locked tab.
//!
//! **Deadlines end a lock, not timers.** Each held lock keeps the instant its TTL ends, and every
//! read compares against it. A lock past its deadline refuses nothing even while its timer has
//! not run (a busy runtime, a stalled task), so a late timer can never extend a lock, and a host
//! that crashed cannot keep a tab longer than the 20-minute cap. The per-lock timer only
//! announces the expiry; any read that finds a passed deadline first ends the lock itself.
//!
//! **Taking a lock settles the tab's in-flight writes**, so the agent's last click does not land
//! under the owner's finger:
//! 1. the lock goes into the table, and refuses from that moment. The dispatcher checks every
//!    operation again at each attempt, so a write it has not sent yet is refused;
//! 2. the trace sink is flushed, so every operation start queued before the lock is committed;
//! 3. the store's open writes on the tab are read every 50 ms for up to 2 s.
//!
//! The store holds every start and end, at most one writer batch behind, so no task follows the
//! trace broadcast and a lagged receiver loses nothing (decision 27). Without the flush, a click
//! that began a moment before the lock could still sit in the writer's queue: the first read
//! would find nothing open, and the lock would report `settled` while that click was in flight.
//!
//! Each lock's life (the settle, the announcement, then the wait for its deadline) runs in a
//! task of its own, so a caller that stops waiting, such as a host that hung up, still leaves
//! the lock settled and announced. State changes are broadcast while the table's mutex is held,
//! so a lock is never announced as taken after it was announced as ended.

use crate::limits::{
    KEEP_ENDED_LOCK, LOCK_CHANGES, MAX_LOCK_TTL, MIN_LOCK_TTL, SETTLE_BUDGET, SETTLE_POLL,
};
use crate::{LockId, PartError, TabHold, TakeoverId, lock_decision};
use browseros_core::TabId;
use neo_trace::{Effect, EpochMillis, TraceSink, TraceStore};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, oneshot};
use tokio::time::Instant;

/// Whether a lock also refuses the agent's reads of its tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reads {
    /// Every operation on the tab is refused. The default: inside `run`, a read can be
    /// `inputValue()` on the field the owner is typing a password into.
    #[default]
    Block,
    /// Only writes are refused (zuzu's old behavior).
    Allow,
}

/// How long a lock holds before it ends itself: 1 s to 20 minutes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockTtl(Duration);

impl LockTtl {
    /// The shortest TTL.
    pub const MIN: Duration = MIN_LOCK_TTL;
    /// The longest TTL, a fixed cap.
    pub const MAX: Duration = MAX_LOCK_TTL;

    /// `None` outside [`Self::MIN`] to [`Self::MAX`].
    #[must_use]
    pub fn new(ttl: Duration) -> Option<Self> {
        (Self::MIN..=Self::MAX).contains(&ttl).then_some(Self(ttl))
    }

    /// The duration.
    #[must_use]
    pub fn get(self) -> Duration {
        self.0
    }
}

/// What an agent operation targets, as the dispatcher's adapter knows it. The adapter has to
/// pick one, so a page it could not map to a tab cannot slip past a lock as "no page".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockTarget {
    /// The operation touches no page, such as `ctx.host.call`. Never refused.
    NoPage,
    /// The operation targets this tab.
    Tab(TabId),
    /// The operation has a page whose tab the adapter does not know. While any lock is held it
    /// is refused as if it were on that lock's tab. Unused once the skeleton's `PolicyCheck`
    /// carries the tab.
    UnmappedPage,
}

/// What a lock a request names is on the tab it names ([`Locks::named`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedLock {
    /// The host took it (`POST /v1/locks`), and it holds the tab: a fill naming it passes it.
    HostHeld,
    /// A takeover took it, and it holds the tab: the owner's, which a fill never passes.
    TakeoverHeld,
    /// No lock with that id holds that tab now: released, expired, or on another tab.
    NotHeld,
}

/// Who may end a lock before its TTL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockBinding {
    /// The host took it directly (`POST /v1/locks`); the host releases it.
    Host,
    /// A takeover took it. Only that takeover's release ends it early, after the owner's
    /// display stops and the fit is restored (decision 29).
    Takeover(TakeoverId),
}

/// A lock as the host sees it: held, or ended with a time and a cause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock {
    /// Its id.
    pub id: LockId,
    /// The tab it keeps the agent off.
    pub tab: TabId,
    /// Whether it refuses reads too.
    pub reads: Reads,
    /// Who may end it early.
    pub binding: LockBinding,
    /// Whether the tab's in-flight writes finished within 2 s of locking. False while that is
    /// unknown.
    pub settled: bool,
    /// When it was taken.
    pub created_at: EpochMillis,
    /// When its TTL ends it.
    pub expires_at: EpochMillis,
    /// Present once it ended.
    pub ended: Option<LockEnded>,
}

/// When and why a lock ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockEnded {
    /// When it stopped refusing. For an expired lock that is its `expires_at`, however late its
    /// timer ran.
    pub at: EpochMillis,
    /// Why.
    pub cause: LockEnd,
}

/// Why a lock ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockEnd {
    /// Its binding released it: the host, or its takeover.
    Released,
    /// Its TTL passed.
    Expired,
    /// Its tab closed. The browser may reuse tab numbers after a restart, so a lock never
    /// outlives its tab.
    TabClosed,
}

/// The agent's operation was refused because the owner has control of its tab. The app's lock
/// policy turns it into `host / owner_has_control`; the dispatcher reports it `not_sent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("The owner has control of this tab. Nothing was done. Wait for them to give control back.")]
pub struct OwnerHasControl {
    /// The lock that refused it: the oldest, when several would.
    pub lock: LockId,
}

/// A held lock and the instant its TTL ends. Tokio's clock, so paused-time tests drive it.
pub(crate) struct Held {
    pub(crate) lock: Lock,
    pub(crate) deadline: Instant,
}

/// Held locks by id, so the oldest comes first (ids are time-ordered), and ended locks in the
/// order they ended.
#[derive(Default)]
struct LockTable {
    held: BTreeMap<LockId, Held>,
    ended: VecDeque<(Instant, Lock)>,
}

impl LockTable {
    fn get(&self, id: LockId) -> Option<&Lock> {
        self.held.get(&id).map(|held| &held.lock).or_else(|| {
            self.ended
                .iter()
                .map(|(_, lock)| lock)
                .find(|lock| lock.id == id)
        })
    }

    fn get_mut(&mut self, id: LockId) -> Option<&mut Lock> {
        match self.held.get_mut(&id) {
            Some(held) => Some(&mut held.lock),
            None => self
                .ended
                .iter_mut()
                .map(|(_, lock)| lock)
                .find(|lock| lock.id == id),
        }
    }

    /// Moves `id` to the ended list, if it is held, and returns it as it ended.
    fn end(&mut self, id: LockId, ended: LockEnded, now: Instant) -> Option<Lock> {
        let mut lock = self.held.remove(&id)?.lock;
        lock.ended = Some(ended);
        self.ended.push_back((now, lock.clone()));
        Some(lock)
    }

    /// Ends every held lock that `matches`, with `cause`, and returns them as they ended.
    fn end_where(
        &mut self,
        cause: LockEnd,
        now: Instant,
        matches: impl Fn(&Held) -> bool,
    ) -> Vec<Lock> {
        let ids = self
            .held
            .iter()
            .filter(|(_, held)| matches(held))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        ids.into_iter()
            .filter_map(|id| {
                let at = match cause {
                    LockEnd::Expired => self.held.get(&id)?.lock.expires_at,
                    LockEnd::Released | LockEnd::TabClosed => EpochMillis::now(),
                };
                self.end(id, LockEnded { at, cause }, now)
            })
            .collect()
    }
}

/// Every lock in the process. Locks live in memory only and vanish with the process, which is
/// safe: the agent's operations run in the same process.
pub struct Locks {
    store: TraceStore,
    sink: TraceSink,
    table: Mutex<LockTable>,
    changes: broadcast::Sender<Lock>,
}

impl Locks {
    /// No locks. `sink` and `store` are the trace writer's: taking a lock flushes the one and
    /// reads the tab's open writes from the other.
    #[must_use]
    pub fn new(store: TraceStore, sink: TraceSink) -> Arc<Self> {
        Arc::new(Self {
            store,
            sink,
            table: Mutex::default(),
            changes: broadcast::channel(LOCK_CHANGES).0,
        })
    }

    /// Locks `tab`: refuses the agent from now on, waits up to 2 s for the tab's in-flight
    /// writes, and returns the lock with `settled` known. The lock may have ended by then (a
    /// TTL under 2 s, a release, the tab closing), and is returned as it stands.
    ///
    /// The tab is not checked here; the caller answers for a tab that is not open first.
    pub async fn lock(
        self: &Arc<Self>,
        tab: TabId,
        reads: Reads,
        ttl: LockTtl,
        binding: LockBinding,
    ) -> Lock {
        let created_at = EpochMillis::now();
        let lock = Lock {
            id: LockId::new(),
            tab,
            reads,
            binding,
            settled: false,
            created_at,
            expires_at: created_at.plus_millis(millis(ttl.get())),
            ended: None,
        };
        let deadline = Instant::now() + ttl.get();
        self.table().held.insert(
            lock.id,
            Held {
                lock: lock.clone(),
                deadline,
            },
        );
        let (taken, settled) = oneshot::channel();
        tokio::spawn(life(Arc::clone(self), lock.id, tab, deadline, taken));
        settled.await.unwrap_or(lock)
    }

    /// Ends a lock the host took. Idempotent: an ended lock is returned as it ended.
    ///
    /// # Errors
    /// `NotFound` for an id no lock has (or one that ended over an hour ago); `TakeoverLock`
    /// for a takeover's lock, held or ended.
    pub fn unlock_host(&self, id: LockId) -> Result<Lock, PartError> {
        self.unlock(id, LockBinding::Host)
    }

    /// Ends the lock `takeover` took, as the last step of its release. Idempotent, as
    /// [`Self::unlock_host`] is.
    ///
    /// # Errors
    /// `NotFound` for an id no lock has (or one that ended over an hour ago), or a host's lock;
    /// `TakeoverLock` for another takeover's.
    pub fn unlock_takeover(&self, id: LockId, takeover: TakeoverId) -> Result<Lock, PartError> {
        self.unlock(id, LockBinding::Takeover(takeover))
    }

    /// Ends `id` for `by`. Only the binding that took a lock ends it before its TTL: a host
    /// that ended a takeover's lock would let the agent act before that takeover restores its fit.
    fn unlock(&self, id: LockId, by: LockBinding) -> Result<Lock, PartError> {
        let mut table = self.table();
        let lock = table.get(id).cloned().ok_or(PartError::NotFound)?;
        if lock.binding != by {
            return Err(match lock.binding {
                LockBinding::Takeover(_) => PartError::TakeoverLock,
                LockBinding::Host => PartError::NotFound,
            });
        }
        let ended = LockEnded {
            at: EpochMillis::now(),
            cause: LockEnd::Released,
        };
        match table.end(id, ended, Instant::now()) {
            Some(ended) => {
                self.announce(&ended);
                Ok(ended)
            }
            None => Ok(lock),
        }
    }

    /// One lock, held or ended. `None` for an id no lock has, or one that ended over an hour ago.
    #[must_use]
    pub fn get(&self, id: LockId) -> Option<Lock> {
        self.table().get(id).cloned()
    }

    /// Whether the held locks refuse an agent operation with `effect` on `target`. The
    /// dispatcher's lock policy calls it at both stages. It never waits: it reads the table under
    /// a short mutex, does no I/O, and changes nothing.
    ///
    /// # Errors
    /// `OwnerHasControl` when a held lock refuses it.
    pub fn check(&self, target: LockTarget, effect: Effect) -> Result<(), OwnerHasControl> {
        let table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        lock_decision::decide(table.held.values(), target, effect, Instant::now())
    }

    /// [`Self::check`], with one held lock passed: the lock a host took itself
    /// ([`LockBinding::Host`]) and named when it typed through it into the tab it holds
    /// (claw-sdk's private typing, the fill's `lockId`). Every other lock refuses as in `check`,
    /// a takeover's included even when named: the owner holds that tab, and only the owner may
    /// let the host type into it. A `pass` that is not such a lock on this tab passes nothing.
    ///
    /// # Errors
    /// `OwnerHasControl` when a held lock other than `pass` refuses it.
    pub fn check_passing(
        &self,
        target: LockTarget,
        effect: Effect,
        pass: LockId,
    ) -> Result<(), OwnerHasControl> {
        let table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        let others = table.held.values().filter(|held| {
            !(held.lock.id == pass
                && held.lock.binding == LockBinding::Host
                && LockTarget::Tab(held.lock.tab) == target)
        });
        lock_decision::decide(others, target, effect, Instant::now())
    }

    /// What lock `id` is on `tab` now, for a fill that names it. A lock past its deadline is not
    /// held, though its timer may not have ended it yet.
    #[must_use]
    pub fn named(&self, id: LockId, tab: TabId) -> NamedLock {
        let now = Instant::now();
        match self.table().held.get(&id) {
            Some(held) if held.lock.tab == tab && held.deadline > now => match held.lock.binding {
                LockBinding::Host => NamedLock::HostHeld,
                LockBinding::Takeover(_) => NamedLock::TakeoverHeld,
            },
            Some(_) | None => NamedLock::NotHeld,
        }
    }

    /// Whether the locks refused an agent operation with `effect` on `target` at `at`, a past
    /// instant: [`Self::check`], asked about `at` instead of now. Asked about the instant a page
    /// event was recorded, the lock that held the event's tab then still refuses it after that
    /// lock has ended, though the agent reads the event later.
    ///
    /// A lock covers `at` from its `created_at` through its end, both inclusive: `ended.at` once
    /// it ended, `expires_at` while it is held. A millisecond stamp cannot order an event against
    /// a lock taken or ended in the same millisecond, so that millisecond counts as covered (fail
    /// closed). Among the locks that cover `at`, the rule is [`Self::check`]'s, shared in
    /// `lock_decision`. Like `check`, it never waits, does no I/O and changes nothing. It reads
    /// the table as it stands, without ending expired locks or forgetting old ones; a held lock
    /// past its deadline still ends at its `expires_at`, so the answer is the same whether or not
    /// a timer or a read has ended it yet.
    ///
    /// **Ended locks outlive any event a running run reads.** The table keeps an ended lock for an
    /// hour (`limits::KEEP_ENDED_LOCK`), counted from when it left the held locks, never before
    /// its end. A run lasts at most 900 s (claw-sdk's `Defaults::RUN_TIMEOUT_MAX`). A lock that
    /// covered an event recorded during a run still in progress is either held, or ended no
    /// earlier than that event and so under 900 s ago, and is in the table either way: such an
    /// event can always be judged. An `at` from before the run started has no such bound. A lock
    /// that ended over an hour ago may be forgotten, and then goes uncounted.
    ///
    /// **One clock.** `created_at`, `ended.at` and `expires_at` are [`EpochMillis::now`]
    /// readings, wall-clock milliseconds since the Unix epoch. neo-playwright stamps each
    /// page event's `EventRecord::at_ms` with its own `now_ms()` (`events::event_hub`), which
    /// reads the same `SystemTime` milliseconds. The two must stay one clock, or `at` is compared
    /// against times measured on another. An expired lock's `ended.at` is its `expires_at`, not
    /// the moment a timer or a read noticed the expiry, so a late timer never stretches a lock's
    /// past.
    ///
    /// # Errors
    /// `OwnerHasControl` when a lock that covered `at` refuses it, naming the oldest such lock.
    pub fn check_at(
        &self,
        target: LockTarget,
        effect: Effect,
        at: EpochMillis,
    ) -> Result<(), OwnerHasControl> {
        let table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        let held = table.held.values().map(|held| &held.lock);
        let ended = table.ended.iter().map(|(_, lock)| lock);
        lock_decision::decide_at(held.chain(ended), target, effect, at)
    }

    /// Whether a held lock names `tab`. Tab cleanup never closes a locked tab.
    #[must_use]
    pub fn is_locked(&self, tab: TabId) -> bool {
        self.table().held.values().any(|held| held.lock.tab == tab)
    }

    /// Whether a lock holds `tab` now, or when the last lock on it ended: the idle-tab janitor
    /// keeps a held tab and restarts its idle clock from that end. The end is when the lock left
    /// the held locks, which for an expired lock is no earlier than its `expires_at`. `None`
    /// when no lock named the tab within the last hour (`limits::KEEP_ENDED_LOCK`).
    #[must_use]
    pub fn hold(&self, tab: TabId) -> Option<TabHold> {
        let table = self.table();
        if table.held.values().any(|held| held.lock.tab == tab) {
            return Some(TabHold::Now);
        }
        table
            .ended
            .iter()
            .filter(|(_, lock)| lock.tab == tab)
            .map(|(ended, _)| *ended)
            .max()
            .map(TabHold::Until)
    }

    /// Ends every lock on a tab that closed (`LockEnd::TabClosed`).
    pub fn tab_closed(&self, tab: TabId) {
        self.end_where(LockEnd::TabClosed, |held| held.lock.tab == tab);
    }

    /// Ends every lock whose tab is not in `open`, as [`Self::tab_closed`] does: the browser link
    /// reconnected, and those tabs closed while Neo was not watching, or the browser restarted.
    pub fn retain_tabs(&self, open: &[TabId]) {
        self.end_where(LockEnd::TabClosed, |held| !open.contains(&held.lock.tab));
    }

    /// Each lock once it is taken and settled, and again when it ends: the `lock.state`
    /// events. A lock that ends during its own settle is announced only as ended.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Lock> {
        self.changes.subscribe()
    }

    /// Ends and announces every lock past its deadline, unless a read already did.
    fn end_expired(&self) {
        drop(self.table());
    }

    fn end_where(&self, cause: LockEnd, matches: impl Fn(&Held) -> bool) {
        let mut table = self.table();
        for lock in table.end_where(cause, Instant::now(), matches) {
            self.announce(&lock);
        }
    }

    /// Records how `id`'s settle went, and announces it as taken if it is still held. One that
    /// ended meanwhile was announced when it ended.
    fn settled(&self, id: LockId, settled: bool) -> Option<Lock> {
        let mut table = self.table();
        let held = table.held.contains_key(&id);
        let lock = table.get_mut(id)?;
        lock.settled = settled;
        let lock = lock.clone();
        if held {
            self.announce(&lock);
        }
        Some(lock)
    }

    /// The table, with every lock past its deadline already ended and announced, and locks that
    /// ended over an hour ago forgotten. Every access but the checks ([`Self::check`] and
    /// [`Self::check_at`]) goes through here, so the first one after a deadline ends that lock,
    /// whether or not its timer has run.
    fn table(&self) -> MutexGuard<'_, LockTable> {
        let mut table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        for lock in table.end_where(LockEnd::Expired, now, |held| held.deadline <= now) {
            self.announce(&lock);
        }
        while table
            .ended
            .front()
            .is_some_and(|(ended, _)| now.duration_since(*ended) >= KEEP_ENDED_LOCK)
        {
            table.ended.pop_front();
        }
        table
    }

    /// Broadcasts a state change. Callers hold the table's mutex, which keeps the announcements
    /// in the order the changes happened.
    fn announce(&self, lock: &Lock) {
        // No subscriber is fine: the service subscribes when it starts, and tests may not.
        let _ = self.changes.send(lock.clone());
    }
}

/// One lock's life once it is in the table: the settle, the announcement, then its end at the
/// deadline. `taken` hands the settled lock to [`Locks::lock`]'s caller, if it still waits.
async fn life(
    locks: Arc<Locks>,
    id: LockId,
    tab: TabId,
    deadline: Instant,
    taken: oneshot::Sender<Lock>,
) {
    let settled = settle(&locks.store, &locks.sink, tab).await;
    if let Some(lock) = locks.settled(id, settled) {
        let _ = taken.send(lock);
    }
    // Only a weak handle waits out the TTL, so a dropped `Locks` (with its sink and store) is
    // not kept alive by its timers. The strong handle must be dropped, not just shadowed.
    let weak = Arc::downgrade(&locks);
    drop(locks);
    tokio::time::sleep_until(deadline).await;
    if let Some(locks) = weak.upgrade() {
        locks.end_expired();
    }
}

/// Whether `tab`'s in-flight writes finished within the settle budget. A failed flush or store
/// read counts as not settled: the lock refuses either way, and `settled` is true only when the
/// store showed no open write after every start sent before the lock was committed. If the trace
/// dropped operation events of a running run under backpressure, the store may lack a write
/// that is still in flight, so the answer is not settled.
async fn settle(store: &TraceStore, sink: &TraceSink, tab: TabId) -> bool {
    let deadline = Instant::now() + SETTLE_BUDGET;
    let flushed = match tokio::time::timeout_at(deadline, sink.flush()).await {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            tracing::warn!(%error, tab = tab.0, "a lock could not flush the trace; not settled");
            false
        }
        Err(_) => false,
    };
    if sink.lost_operation_events() {
        tracing::warn!(
            tab = tab.0,
            "the trace dropped operation events; not settled"
        );
        return false;
    }
    loop {
        match tokio::time::timeout_at(deadline, store.open_writes(tab)).await {
            Ok(Ok(0)) => return flushed,
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                tracing::warn!(%error, tab = tab.0, "a lock could not read open writes; not settled");
                return false;
            }
            Err(_) => return false,
        }
        if tokio::time::timeout_at(deadline, tokio::time::sleep(SETTLE_POLL))
            .await
            .is_err()
        {
            return false;
        }
    }
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}
