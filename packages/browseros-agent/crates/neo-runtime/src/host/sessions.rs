//! Neo: The session ledger. From the run path's events it decides which host sessions are live,
//! which tabs each worked in, which tab is current, what is withheld, and when a session is
//! `complete`, a promise `/v1` keeps: once true, never false. No I/O, clock reads or tasks; taint
//! is read from the host's shared monotonic privacy fact. The host task owns the only [`Ledger`],
//! applies the tap's events in FIFO order with the
//! time, and turns what it returns into work.
//!
//! ```text
//!  run tool ───── admit, before Runtime::run ────────────────┐
//!  run path ───── HostEvent ──────▶ tap (one FIFO) ── apply ──┼──▶ Ledger ── Job ──▶ after-call
//!  RunPass drop ─ PassReleased ───▶ the same FIFO ────────────┘      ▲                   │
//!  host task ──── end, expire, withhold, damage, job_settled ────────┴───────────────────┘
//!  readers ◀───── view (FollowView), completion, presence_needed
//! ```
//!
//! The host task is the ledger's one writer. Each cell of a session changes only through the
//! calls below, and readers merge the cells, and the tab directory, at read time:
//!
//! | Cell              | Written by                                  | Moves                     |
//! |-------------------|---------------------------------------------|---------------------------|
//! | `phase`           | `admit` creates it; `end`, `expire` end it; | Live → Ended, never back  |
//! |                   | a consumed `PassReleased` restarts its idle |                           |
//! | `passes`          | `admit` +1, a consumed `PassReleased` −1    |                           |
//! | `touches`, `runs` | `apply`                                     | newest touch per target   |
//! | `withheld`        | `withhold`                                  | false → true              |
//! | `pending`         | `apply` +1 per job, `job_settled` −1        |                           |
//! | `damaged`         | `damage`                                    | false → true              |
//!
//! Completion is the neo-host-events design's four-step protocol, and [`Completion`] names the
//! step a session waits on: admission closed (the phase ended); every admitted run over (its pass's
//! `PassReleased` consumed, which the tap's FIFO puts behind every event the run emitted, so every
//! job the run caused is already counted); every job settled; then the trace store's flush,
//! which the host task requests and awaits. Nothing reopens a finished session: `Phase` has no way
//! back from `Ended`, an admission under an ended key gets a successor, and events and damage of
//! a session with no outstanding pass are ignored, because no admitted run can still emit them.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;

use crate::host::directory::TabTable;
use crate::host::events::HostEvent;
use crate::host::ids::{EpochMs, SessionKey};
use crate::host::privacy::Taint;
use crate::host::view::{FollowView, ShownTab};
use crate::run::{PageId, RunId};

/// Every session this process admitted, and the taint withheld sessions left on tabs.
pub(crate) struct Ledger {
    /// How long a session with no outstanding pass stays live.
    idle: Duration,
    sessions: HashMap<SessionKey, Session>,
    /// Targets any withheld session touched. No session shows them or gets a screenshot of them
    /// for the rest of the tab's life, so a second session watching a shared tab cannot see the
    /// first one's private input. A view of the Host's shared set, never a ledger-owned copy.
    taint: Arc<Taint>,
    /// Touches so far: the arrival order that breaks ties between touches with one start time.
    arrivals: u64,
}

/// One session's cells; the module's table says who writes each.
struct Session {
    phase: Phase,
    /// Passes admitted whose `PassReleased` the ledger has not consumed: runs whose events may
    /// still arrive.
    passes: usize,
    /// The newest touch of each target the session worked in.
    touches: HashMap<PageId, TouchKey>,
    /// Its runs that started and have not ended.
    runs: HashMap<RunId, Run>,
    /// Ordered projection for views and jobs. The synchronous emitter flag lives in privacy.rs;
    /// pixel revocation reads shared taint and never waits for this task-owned cell.
    withheld: bool,
    /// Jobs handed out and not settled yet.
    pending: usize,
    /// A lifecycle event of one of its runs was refused past the tap's reserved band, so its
    /// accounting cannot be trusted and it never completes.
    damaged: bool,
    /// The session that admissions under this key join once it ended, while that one is live.
    successor: Option<SessionKey>,
}

/// A session's lifecycle. Nothing turns `Ended` back into `Live`: once a session ended no run can
/// join it, which is what lets `/v1` promise `complete`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Admitting runs. With no pass outstanding it ends once `idle` has passed since `idle_from`:
    /// its last consumed `PassReleased`, or its admission.
    Live {
        idle_from: Instant,
    },
    Ended {
        at: Instant,
        why: EndReason,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EndReason {
    /// `idle` passed with no pass outstanding; `at` is when that window closed.
    Idle,
    /// Ended on request ([`Ledger::end`]).
    Ended,
    /// The trace store knew the key but this process never admitted it: the server restarted
    /// under it. `at` is when this process first saw the key; its runs ended before that.
    Restarted,
}

/// The order of touches, which is `/v1`'s row order: when the run or call started, then arrival
/// at the ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TouchKey {
    at: EpochMs,
    arrival: u64,
}

/// A run that started and has not ended.
struct Run {
    /// `RunStarted.page`: the tab its root row is about.
    page: PageId,
    /// Every target it touched. A presence must not detach from one while the run is in flight:
    /// the detach would clear the page's focus under the script (probe P-A).
    targets: HashSet<PageId>,
    /// Its calls that started and have not ended.
    calls: HashMap<u64, Call>,
}

/// What the ledger keeps of a call between its start and end: when it started, which is where a
/// page it creates sorts, and whether a screenshot follows it.
#[derive(Clone, Copy)]
struct Call {
    at: EpochMs,
    snapshot: bool,
}

/// What [`Ledger::admit`] answers: the session the run belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Admitted {
    /// The key for `RunRequest.session` and the run result's `_meta`.
    pub(crate) session: SessionKey,
    /// The label, when its session had ended and the run joined a successor.
    pub(crate) successor_of: Option<SessionKey>,
}

/// Work for the after-call worker. Each job is pending for its session until
/// [`Ledger::job_settled`], whether it ran, failed or was dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Job {
    pub(crate) kind: JobKind,
    pub(crate) session: SessionKey,
    pub(crate) run: RunId,
    /// The step's call id; `None` for the run's root row.
    pub(crate) call_id: Option<u64>,
    /// The tab the row is about: the call's page, or the run's own page for the root.
    pub(crate) page: PageId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JobKind {
    /// The tab's id, URL and title as they are after the call: `/v1`'s `ToolCallPage`.
    Facts,
    /// A screenshot of the tab after the call.
    Still,
}

/// Where a session stands in the completion protocol: the step it waits on, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Completion {
    /// Step 1: admission is open.
    Live,
    /// Step 2: a pass is outstanding, so its run may still emit events.
    Running,
    /// Step 3: a job is pending.
    Settling,
    /// Steps 1 to 3 hold, for good. The session is complete once the trace store acknowledges a
    /// flush requested from now on (step 4).
    ReadyForFlush,
    /// It lost a lifecycle event and never completes.
    Damaged,
}

impl Ledger {
    #[cfg(test)]
    pub(crate) fn new(idle: Duration) -> Self {
        Self::with_taint(idle, Arc::default())
    }

    pub(crate) fn with_taint(idle: Duration, taint: Arc<Taint>) -> Self {
        Self {
            idle,
            sessions: HashMap::new(),
            taint,
            arrivals: 0,
        }
    }

    /// Admits one run, before every `Runtime::run`, and counts its pass. A label naming a live
    /// session joins it. A label the ledger does not know joins a new session, unless the trace
    /// store knows it (`label_known_to_store`, asked only for labels [`Self::knows`] does not
    /// know): the server restarted under it. A label whose session ended, or is past its idle
    /// window at `now`, gets a successor, the same one for every admission under that label
    /// while the successor lives, because clients keep resending a dead handle. No label gets a new session.
    /// Neo: claw-server's `Retired.replacement` (`apps/claw-server/src/services/sessions/manager.rs`), preserving successor reuse so repeated stale partner handles converge on one live session.
    /// Minted keys are unique in the ledger; unique in the trace store too is the caller's check.
    ///
    /// A session past its idle window is ended by [`Self::expire`], never here, so every idle
    /// end is reported once.
    #[cfg(test)]
    pub(crate) fn admit(
        &mut self,
        label: Option<&SessionKey>,
        known: bool,
        now: Instant,
    ) -> Admitted {
        self.admit_with_key(label, known, now, SessionKey::mint())
    }

    /// Admission supplies a candidate already checked against both the ledger and the store.
    /// Keeping the I/O check outside this reducer leaves minting collision-safe after restart.
    pub(crate) fn admit_with_key(
        &mut self,
        label: Option<&SessionKey>,
        label_known_to_store: bool,
        now: Instant,
        candidate: SessionKey,
    ) -> Admitted {
        let Some(label) = label else {
            return Admitted {
                session: self.fresh(now, candidate),
                successor_of: None,
            };
        };
        let idle = self.idle;
        match self.sessions.get_mut(label) {
            Some(session) if session.live_at(now, idle) => {
                session.passes += 1;
                return Admitted {
                    session: label.clone(),
                    successor_of: None,
                };
            }
            Some(_) => {}
            None if label_known_to_store => {
                let ended = Phase::Ended {
                    at: now,
                    why: EndReason::Restarted,
                };
                self.sessions.insert(label.clone(), Session::new(ended, 0));
            }
            None => {
                let live = Phase::Live { idle_from: now };
                self.sessions.insert(label.clone(), Session::new(live, 1));
                return Admitted {
                    session: label.clone(),
                    successor_of: None,
                };
            }
        }
        Admitted {
            session: self.successor(label, now, candidate),
            successor_of: Some(label.clone()),
        }
    }

    /// Whether the ledger knows `key`, live or ended. A key it does not know is the one case
    /// where admission needs the trace store's answer.
    pub(crate) fn knows(&self, key: &SessionKey) -> bool {
        self.sessions.contains_key(key)
    }

    /// Applies one event from the tap, in FIFO order, and returns the after-call work it causes,
    /// each job pending for its session until [`Self::job_settled`]:
    ///
    /// - page facts after every `CallEnded` that names a page, and after every `RunEnded`: the
    ///   run's root row, on its own page;
    /// - a screenshot after those of a call the spec flags `snapshot`, and after every run, unless
    ///   the session is withheld or the tab tainted.
    ///
    /// A touch (a run starting on a tab, a call naming one, a run creating one) moves the
    /// session's current tab; the emitter already tainted a withheld session's target before
    /// queueing that touch. A consumed
    /// `PassReleased` counts its run over and restarts the idle clock at `now`.
    ///
    /// Events of a session with no outstanding pass are ignored: no admitted run can still emit
    /// them, and taking them could change a record already called complete. A `CallEnded` the
    /// tap refused stays a permanent absence: nothing else asks for that row's page facts or
    /// screenshot, and nothing fills them in from live state later.
    pub(crate) fn apply(&mut self, event: HostEvent, now: Instant) -> Vec<Job> {
        let Self {
            sessions,
            taint,
            arrivals,
            ..
        } = self;
        let Some(session) = sessions
            .get_mut(event.session())
            .filter(|session| session.passes > 0)
        else {
            return Vec::new();
        };
        let jobs = match event {
            HostEvent::RunStarted { run, page, at, .. } => {
                session.touch(&page, at, arrivals);
                session.runs.entry(run).or_insert_with(|| {
                    let targets = HashSet::from([page.clone()]);
                    Run {
                        page,
                        targets,
                        calls: HashMap::new(),
                    }
                });
                Vec::new()
            }
            HostEvent::CallStarted {
                run,
                call_id,
                page,
                snapshot,
                at,
                ..
            } => {
                if let Some(page) = &page {
                    session.touch(page, at, arrivals);
                }
                if let Some(entry) = session.runs.get_mut(&run) {
                    entry.targets.extend(page);
                    entry.calls.insert(call_id, Call { at, snapshot });
                }
                Vec::new()
            }
            HostEvent::CallEnded {
                session: key,
                run,
                call_id,
                page,
                ..
            } => {
                let call = session
                    .runs
                    .get_mut(&run)
                    .and_then(|entry| entry.calls.remove(&call_id));
                match page {
                    Some(page) => {
                        let still =
                            call.is_some_and(|call| call.snapshot) && session.visible(&page, taint);
                        row_jobs(key, run, Some(call_id), page, still)
                    }
                    None => Vec::new(),
                }
            }
            HostEvent::PageCreated {
                run, page, call_id, ..
            } => {
                // The creation sorts as its creating call does on `/v1`. Without that call (none,
                // or its start was refused) it is no older than anything the session did.
                let creation = call_id
                    .and_then(|call_id| Some(session.runs.get(&run)?.calls.get(&call_id)?.at));
                if let Some(at) = creation.or_else(|| session.newest_touch()) {
                    session.touch(&page, at, arrivals);
                }
                if let Some(entry) = session.runs.get_mut(&run) {
                    entry.targets.insert(page);
                }
                Vec::new()
            }
            HostEvent::RunEnded {
                session: key, run, ..
            } => match session.runs.remove(&run) {
                Some(entry) => {
                    let still = session.visible(&entry.page, taint);
                    row_jobs(key, run, None, entry.page, still)
                }
                None => Vec::new(),
            },
            HostEvent::PassReleased { .. } => {
                session.passes -= 1;
                if session.passes == 0 {
                    // Every run's events are in; a run still listed lost its `RunEnded`.
                    session.runs.clear();
                }
                if let Phase::Live { idle_from } = &mut session.phase {
                    *idle_from = now;
                }
                Vec::new()
            }
        };
        session.pending += jobs.len();
        jobs
    }

    /// Settles one of `key`'s jobs, whether it ran, failed or was dropped.
    pub(crate) fn job_settled(&mut self, key: &SessionKey) {
        if let Some(session) = self.sessions.get_mut(key) {
            session.pending = session.pending.saturating_sub(1);
        }
    }

    /// Ends `key`'s session on request: no run joins it again, while runs already admitted finish
    /// and their events still count. Returns whether it was live, so the caller ends its live
    /// views. Idempotent.
    pub(crate) fn end(&mut self, key: &SessionKey, now: Instant) -> bool {
        let Some(session) = self.sessions.get_mut(key) else {
            return false;
        };
        if !session.is_live() {
            return false;
        }
        session.phase = Phase::Ended {
            at: now,
            why: EndReason::Ended,
        };
        true
    }

    /// Ends every session whose idle window closed by `now`, with no pass outstanding, and
    /// returns their keys, sorted. A session with a pass outstanding never expires.
    pub(crate) fn expire(&mut self, now: Instant) -> Vec<SessionKey> {
        let idle = self.idle;
        let mut ended = Vec::new();
        for (key, session) in &mut self.sessions {
            if let Some(deadline) = session.idle_deadline(idle)
                && now >= deadline
            {
                session.phase = Phase::Ended {
                    at: deadline,
                    why: EndReason::Idle,
                };
                ended.push(key.clone());
            }
        }
        ended.sort();
        ended
    }

    /// When the next session expires unless a run is admitted to it first: when the host task
    /// calls [`Self::expire`] next.
    pub(crate) fn next_expiry(&self) -> Option<Instant> {
        self.sessions
            .values()
            .filter_map(|session| session.idle_deadline(self.idle))
            .min()
    }

    /// Withholds `key`'s visuals for good and returns the targets this newly taints (its touched
    /// tabs), so the caller ends their captures and drops their pending screenshots for every
    /// session. `None` when the ledger does not know the session. A repeat taints nothing new.
    pub(crate) fn withhold(&mut self, key: &SessionKey) -> Option<Vec<PageId>> {
        let session = self.sessions.get_mut(key)?;
        session.withheld = true;
        let mut tainted = Vec::new();
        for target in session.touches.keys() {
            if self.taint.insert(target.clone()) {
                tainted.push(target.clone());
            }
        }
        Some(tainted)
    }

    /// Taints `targets` again after a restart: the tabs withheld sessions touched, from the trace
    /// store's withheld flags and call targets, so taint outlives the process.
    #[cfg(test)]
    pub(crate) fn restore_taint(&mut self, targets: impl IntoIterator<Item = PageId>) {
        self.taint.extend(targets);
    }

    /// Whether no session may show `target` or keep a screenshot of it.
    #[cfg(test)]
    pub(crate) fn tainted(&self, target: &PageId) -> bool {
        self.taint.contains(target)
    }

    /// Marks `key` damaged: the tap refused a lifecycle event of one of its runs, past the
    /// reserved band. It applies only while a pass is outstanding, which is when such an event
    /// can exist; the host task applies any damage it learned of before each event it applies,
    /// so damage always lands before that run's `PassReleased`, and a finished session stays
    /// finished.
    pub(crate) fn damage(&mut self, key: &SessionKey) {
        if let Some(session) = self.sessions.get_mut(key)
            && session.passes > 0
        {
            session.damaged = true;
        }
    }

    /// What a live view may show of `key` now: whether it is live or withheld, and its shown
    /// tabs, the current one first. `None` when the ledger does not know the session.
    pub(crate) fn view(&self, key: &SessionKey, tabs: &TabTable) -> Option<FollowView> {
        let session = self.sessions.get(key)?;
        Some(FollowView {
            live: session.is_live(),
            withheld: session.withheld,
            tabs: shown_tabs(&session.touches, tabs, &self.taint),
        })
    }

    /// Where `key` stands in the completion protocol; `None` when the ledger does not know it.
    pub(crate) fn completion(&self, key: &SessionKey) -> Option<Completion> {
        let session = self.sessions.get(key)?;
        Some(if session.damaged {
            Completion::Damaged
        } else if session.is_live() {
            Completion::Live
        } else if session.passes > 0 {
            Completion::Running
        } else if session.pending > 0 {
            Completion::Settling
        } else {
            Completion::ReadyForFlush
        })
    }

    pub(crate) fn phase(&self, key: &SessionKey) -> Option<Phase> {
        self.sessions.get(key).map(|session| session.phase)
    }

    /// Whether `target` needs a presence: a live session touched it, or a run is in flight on
    /// it. While this holds, detaching the presence could clear the page's focus under a script
    /// or stop a watched background tab painting.
    pub(crate) fn presence_needed(&self, target: &PageId) -> bool {
        self.sessions.values().any(|session| {
            (session.is_live() && session.touches.contains_key(target))
                || session
                    .runs
                    .values()
                    .any(|run| run.targets.contains(target))
        })
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = &SessionKey> {
        self.sessions.keys()
    }

    pub(crate) fn touched(&self, key: &SessionKey) -> Vec<PageId> {
        self.sessions
            .get(key)
            .map(|session| session.touches.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Called only for ended, complete, unwatched sessions. Durable knowledge belongs to the
    /// store, so later admission still gives an evicted label a successor.
    pub(crate) fn forget(&mut self, key: &SessionKey) {
        self.sessions.remove(key);
    }

    /// The session admissions under the ended `label` join: the successor an earlier one opened,
    /// while it is live, else a fresh one, remembered for the next.
    fn successor(&mut self, label: &SessionKey, now: Instant, candidate: SessionKey) -> SessionKey {
        let idle = self.idle;
        let remembered = self
            .sessions
            .get(label)
            .and_then(|ended| ended.successor.clone());
        if let Some(successor) = remembered
            && let Some(session) = self.sessions.get_mut(&successor)
            && session.live_at(now, idle)
        {
            session.passes += 1;
            return successor;
        }
        let successor = self.fresh(now, candidate);
        if let Some(ended) = self.sessions.get_mut(label) {
            ended.successor = Some(successor.clone());
        }
        successor
    }

    /// Opens a live session under a key no session has, holding this admission's pass.
    fn fresh(&mut self, now: Instant, candidate: SessionKey) -> SessionKey {
        let mut key = candidate;
        loop {
            if !self.sessions.contains_key(&key) {
                break;
            }
            key = SessionKey::mint();
        }
        let live = Phase::Live { idle_from: now };
        self.sessions.insert(key.clone(), Session::new(live, 1));
        key
    }
}

impl Session {
    fn new(phase: Phase, passes: usize) -> Self {
        Self {
            phase,
            passes,
            touches: HashMap::new(),
            runs: HashMap::new(),
            withheld: false,
            pending: 0,
            damaged: false,
            successor: None,
        }
    }

    fn is_live(&self) -> bool {
        matches!(self.phase, Phase::Live { .. })
    }

    /// When the session ends by itself: `None` once it ended, and while a pass is outstanding.
    fn idle_deadline(&self, idle: Duration) -> Option<Instant> {
        match self.phase {
            Phase::Live { idle_from } if self.passes == 0 => idle_from.checked_add(idle),
            _ => None,
        }
    }

    /// Whether a run may join it at `now`: it is live and not past its idle window.
    fn live_at(&self, now: Instant, idle: Duration) -> bool {
        self.is_live()
            && self
                .idle_deadline(idle)
                .is_none_or(|deadline| now < deadline)
    }

    /// The session worked in `target` with a run or call that started `at`.
    fn touch(&mut self, target: &PageId, at: EpochMs, arrivals: &mut u64) {
        *arrivals += 1;
        let touch = TouchKey {
            at,
            arrival: *arrivals,
        };
        self.touches
            .entry(target.clone())
            .and_modify(|newest| *newest = (*newest).max(touch))
            .or_insert(touch);
    }

    fn newest_touch(&self) -> Option<EpochMs> {
        self.touches.values().map(|touch| touch.at).max()
    }

    /// Whether a screenshot of `target` may be taken for this session.
    fn visible(&self, target: &PageId, taint: &Taint) -> bool {
        !self.withheld && !taint.contains(target)
    }
}

/// THE current-tab rule, the only one: the session's touched tabs that are open in the
/// directory and not tainted, newest touch first. The order is `/v1`'s row order, so the
/// screencast follower, the `?tab=` check and `currentTabId` agree with a host reading the last
/// `toolCalls[].page.tabId` once the call ends. A touched target the directory does not report
/// open (not discovered yet, or closed) is skipped until it is.
fn shown_tabs(
    touches: &HashMap<PageId, TouchKey>,
    tabs: &TabTable,
    taint: &Taint,
) -> Vec<ShownTab> {
    let mut shown: Vec<(TouchKey, ShownTab)> = touches
        .iter()
        .filter(|(target, _)| !taint.contains(target))
        .filter_map(|(target, touch)| {
            let entry = tabs.open(target)?;
            let tab = ShownTab {
                target: target.clone(),
                tab: entry.tab,
                url: entry.url.clone(),
            };
            Some((*touch, tab))
        })
        .collect();
    shown.sort_unstable_by_key(|(touch, _)| Reverse(*touch));
    shown.into_iter().map(|(_, tab)| tab).collect()
}

/// A finished row's jobs: its page facts, and a screenshot of its tab when `still`.
fn row_jobs(
    session: SessionKey,
    run: RunId,
    call_id: Option<u64>,
    page: PageId,
    still: bool,
) -> Vec<Job> {
    let facts = Job {
        kind: JobKind::Facts,
        session,
        run,
        call_id,
        page,
    };
    if still {
        let screenshot = Job {
            kind: JobKind::Still,
            ..facts.clone()
        };
        vec![facts, screenshot]
    } else {
        vec![facts]
    }
}

#[cfg(test)]
mod tests {
    use crate::host::directory::{DirectoryInput, PageTarget, TabEntry};
    use crate::host::ids::TabId;
    use crate::run::{CallResult, RunEnd, RunStatus};

    use super::*;

    const IDLE: Duration = Duration::from_mins(30);

    fn key(label: &str) -> SessionKey {
        SessionKey::new(label)
    }

    fn page(id: &str) -> PageId {
        PageId::new(id.to_string())
    }

    fn ms(at: u64) -> EpochMs {
        EpochMs::from_millis(at).expect("a positive time")
    }

    fn admitted(ledger: &mut Ledger, label: &str, now: Instant) -> SessionKey {
        ledger.admit(Some(&key(label)), false, now).session
    }

    fn settle(ledger: &mut Ledger, jobs: &[Job]) {
        for job in jobs {
            ledger.job_settled(&job.session);
        }
    }

    /// A directory with `tabs` open, as a seed reports them.
    fn directory(tabs: &[(&str, i64)]) -> TabTable {
        let seed = tabs
            .iter()
            .map(|(id, tab)| PageTarget {
                target: page(id),
                entry: TabEntry {
                    tab: TabId::parse(*tab).expect("a positive tab id"),
                    url: format!("https://{id}.test/"),
                    title: String::new(),
                },
            })
            .collect();
        let mut table = TabTable::default();
        table.apply(DirectoryInput::Seeded(seed), Instant::now());
        table
    }

    /// The targets `session`'s view shows, the current one first.
    fn shown(ledger: &Ledger, session: &SessionKey, tabs: &TabTable) -> Vec<String> {
        let view = ledger.view(session, tabs).expect("a known session");
        view.tabs
            .iter()
            .map(|shown| shown.target.as_str().to_string())
            .collect()
    }

    /// The events one run of a session emits, shaped as the run path emits them, and the jobs
    /// its rows cause.
    struct Script {
        session: SessionKey,
        run: RunId,
    }

    impl Script {
        fn of(session: &SessionKey) -> Self {
            Self {
                session: session.clone(),
                run: RunId::mint(),
            }
        }

        fn started(&self, target: &str, at: u64) -> HostEvent {
            HostEvent::RunStarted {
                session: self.session.clone(),
                run: self.run.clone(),
                page: page(target),
                at: ms(at),
            }
        }

        /// A call naming `target`, or a context call for `None`, that started `at`.
        fn call(&self, call_id: u64, target: Option<&str>, snapshot: bool, at: u64) -> HostEvent {
            HostEvent::CallStarted {
                session: self.session.clone(),
                run: self.run.clone(),
                call_id,
                page: target.map(page),
                snapshot,
                at: ms(at),
            }
        }

        fn call_ended(&self, call_id: u64, target: Option<&str>) -> HostEvent {
            self.call_ended_as(call_id, target, CallResult::Ok)
        }

        fn call_ended_as(
            &self,
            call_id: u64,
            target: Option<&str>,
            result: CallResult,
        ) -> HostEvent {
            HostEvent::CallEnded {
                session: self.session.clone(),
                run: self.run.clone(),
                call_id,
                page: target.map(page),
                result,
            }
        }

        fn created(&self, target: &str, call_id: Option<u64>) -> HostEvent {
            HostEvent::PageCreated {
                session: self.session.clone(),
                run: self.run.clone(),
                page: page(target),
                tab: None,
                call_id,
            }
        }

        fn ended(&self, status: RunStatus, at: u64) -> HostEvent {
            HostEvent::RunEnded {
                session: self.session.clone(),
                run: self.run.clone(),
                status,
                at: ms(at),
            }
        }

        fn released(&self) -> HostEvent {
            HostEvent::PassReleased {
                session: self.session.clone(),
            }
        }

        fn job(&self, kind: JobKind, call_id: Option<u64>, target: &str) -> Job {
            Job {
                kind,
                session: self.session.clone(),
                run: self.run.clone(),
                call_id,
                page: page(target),
            }
        }

        fn facts(&self, call_id: Option<u64>, target: &str) -> Job {
            self.job(JobKind::Facts, call_id, target)
        }

        fn still(&self, call_id: Option<u64>, target: &str) -> Job {
            self.job(JobKind::Still, call_id, target)
        }
    }

    #[test]
    fn admissions_racing_on_a_new_label_share_one_session() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = key("task");
        assert!(!ledger.knows(&label), "a label never seen is unknown");
        let first = ledger.admit(Some(&label), false, t0);
        let second = ledger.admit(Some(&label), true, t0);
        let joined = Admitted {
            session: label.clone(),
            successor_of: None,
        };
        assert_eq!(first, joined, "a new label opens its own session");
        assert_eq!(
            second, joined,
            "a racing admission joins it, whatever the store said"
        );

        let script = Script::of(&label);
        let t1 = t0 + 2 * IDLE;
        ledger.apply(script.released(), t1);
        assert_eq!(
            ledger.expire(t1 + IDLE),
            Vec::<SessionKey>::new(),
            "both passes count on the one session"
        );
        let t2 = t1 + IDLE;
        ledger.apply(script.released(), t2);
        assert_eq!(
            ledger.expire(t2 + IDLE),
            vec![label],
            "it ends once both are released and idle"
        );
    }

    #[test]
    fn an_ended_label_gets_one_successor_while_it_lives() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        assert!(ledger.end(&label, t0), "ending a live session reports it");
        let first = ledger.admit(Some(&label), false, t0);
        assert_ne!(first.session, label, "an ended session takes no run");
        assert_eq!(
            first.successor_of,
            Some(label.clone()),
            "the run joins a successor"
        );
        assert_eq!(
            ledger.admit(Some(&label), false, t0),
            first,
            "a resent dead label lands in the same successor"
        );
        assert_eq!(
            ledger.completion(&first.session),
            Some(Completion::Live),
            "the successor is live"
        );

        ledger.end(&first.session, t0);
        let next = ledger.admit(Some(&label), false, t0);
        assert!(
            next.session != first.session && next.session != label,
            "once the successor ended too, the label gets a new one"
        );
    }

    #[test]
    fn an_expired_label_gets_a_successor() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        ledger.apply(Script::of(&label).released(), t0);
        let late = t0 + IDLE;
        assert_eq!(
            ledger.admit(Some(&label), false, late).successor_of,
            Some(label.clone()),
            "a session past its idle window takes no run, swept or not"
        );
        assert_eq!(
            ledger.expire(late),
            vec![label.clone()],
            "the sweep still reports its end"
        );
        assert_eq!(
            ledger.phase(&label),
            Some(Phase::Ended {
                at: late,
                why: EndReason::Idle,
            }),
            "ended when its window closed"
        );
    }

    #[test]
    fn a_label_the_store_knows_gets_a_successor() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = key("from-before-the-restart");
        let first = ledger.admit(Some(&label), true, t0);
        assert_ne!(first.session, label, "the old session takes no run");
        assert_eq!(
            first.successor_of,
            Some(label.clone()),
            "the run joins a successor"
        );
        assert_eq!(
            ledger.phase(&label),
            Some(Phase::Ended {
                at: t0,
                why: EndReason::Restarted,
            }),
            "the ledger remembers the label as ended by the restart"
        );
        assert_eq!(
            ledger.admit(Some(&label), true, t0),
            first,
            "a resent label lands in the same successor"
        );
        assert_eq!(
            ledger.completion(&label),
            Some(Completion::ReadyForFlush),
            "nothing in this process can add to the old session"
        );
    }

    #[test]
    fn unlabeled_runs_get_fresh_sessions() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let first = ledger.admit(None, false, t0);
        let second = ledger.admit(None, false, t0);
        assert_ne!(
            first.session, second.session,
            "each unlabeled run gets its own session"
        );
        assert_eq!(
            (first.successor_of, second.successor_of),
            (None, None),
            "neither succeeds an ended session"
        );
        assert_eq!(
            ledger.completion(&first.session),
            Some(Completion::Live),
            "a fresh session is live"
        );
    }

    #[test]
    fn a_session_with_a_pass_outstanding_never_expires() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        assert_eq!(
            ledger.next_expiry(),
            None,
            "no deadline while a run holds its pass"
        );
        assert_eq!(
            ledger.expire(t0 + 100 * IDLE),
            Vec::<SessionKey>::new(),
            "however long the run takes"
        );
        assert_eq!(
            ledger.completion(&label),
            Some(Completion::Live),
            "it stays live"
        );
    }

    #[test]
    fn idle_counts_from_the_last_pass_release() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        ledger.admit(Some(&label), false, t0);
        let script = Script::of(&label);
        let t1 = t0 + Duration::from_mins(1);
        ledger.apply(script.released(), t1);
        assert_eq!(ledger.next_expiry(), None, "one pass is still outstanding");
        let t2 = t1 + Duration::from_mins(1);
        ledger.apply(script.released(), t2);
        assert_eq!(
            ledger.next_expiry(),
            Some(t2 + IDLE),
            "the idle window opens at the last release"
        );
        assert_eq!(
            ledger.expire(t2 + IDLE - Duration::from_millis(1)),
            Vec::<SessionKey>::new(),
            "live until the window closes"
        );
        let late_sweep = t2 + IDLE + Duration::from_mins(5);
        assert_eq!(
            ledger.expire(late_sweep),
            vec![label.clone()],
            "a sweep after the window closed ends it"
        );
        assert_eq!(
            ledger.phase(&label),
            Some(Phase::Ended {
                at: t2 + IDLE,
                why: EndReason::Idle,
            }),
            "as of the moment the window closed, not the sweep"
        );
        assert_eq!(
            ledger.expire(late_sweep + IDLE),
            Vec::<SessionKey>::new(),
            "an idle end is reported once"
        );
        assert_eq!(
            ledger.next_expiry(),
            None,
            "an ended session has no deadline"
        );
    }

    #[test]
    fn ending_is_final_and_idempotent() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        assert!(ledger.end(&label, t0), "ending a live session reports it");
        assert!(!ledger.end(&label, t0), "ending it again reports nothing");
        assert!(
            !ledger.end(&key("unknown"), t0),
            "an unknown session has nothing to end"
        );
        let ended = Some(Phase::Ended {
            at: t0,
            why: EndReason::Ended,
        });
        assert_eq!(ledger.phase(&label), ended, "ended on request");
        ledger.apply(Script::of(&label).released(), t0 + IDLE);
        assert_eq!(
            ledger.phase(&label),
            ended,
            "a pass release never revives it"
        );
    }

    /// The cross-judge's interleaving (a): admit, end, then a read. Not complete while the pass
    /// lives, and the run's later calls still get their rows' work.
    #[test]
    fn an_ended_session_is_not_complete_while_its_pass_lives() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        ledger.apply(script.started("p", 10), t0);
        assert_eq!(
            ledger.completion(&label),
            Some(Completion::Live),
            "admission is open"
        );
        ledger.end(&label, t0);
        assert_eq!(
            ledger.completion(&label),
            Some(Completion::Running),
            "ended, but its run still holds the pass"
        );
        ledger.apply(script.call(1, Some("p"), false, 20), t0);
        let mut jobs = ledger.apply(script.call_ended(1, Some("p")), t0);
        assert_eq!(
            jobs,
            vec![script.facts(Some(1), "p")],
            "a call after the end still gets its page facts"
        );
        jobs.extend(ledger.apply(script.ended(RunStatus::Ok, 30), t0));
        ledger.apply(script.released(), t0);
        assert_eq!(
            ledger.completion(&label),
            Some(Completion::Settling),
            "every run is over, its jobs are not"
        );
        settle(&mut ledger, &jobs);
        assert_eq!(
            ledger.completion(&label),
            Some(Completion::ReadyForFlush),
            "ready once they settle"
        );
    }

    /// The cross-judge's interleaving (b): the store already committed the run's end while the
    /// tap still holds its pass release.
    #[test]
    fn a_consumed_run_end_does_not_complete_before_its_pass_release() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        ledger.apply(script.started("p", 10), t0);
        ledger.end(&label, t0);
        let root = ledger.apply(script.ended(RunStatus::Ok, 20), t0);
        assert_eq!(
            root,
            vec![script.facts(None, "p"), script.still(None, "p")],
            "the run's end asks for its root row's page facts and screenshot"
        );
        settle(&mut ledger, &root);
        assert_eq!(
            ledger.completion(&label),
            Some(Completion::Running),
            "not complete before the pass release is consumed"
        );
        ledger.apply(script.released(), t0);
        assert_eq!(
            ledger.completion(&label),
            Some(Completion::ReadyForFlush),
            "ready once it is"
        );
    }

    /// The cross-judge's interleaving (c): the run's future was dropped mid-call. The run path
    /// ends the cut call `Unfinished` and the run, the pass's drop releases it, and the session
    /// completes.
    #[test]
    fn a_pass_released_after_a_cut_run_still_completes() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        ledger.apply(script.started("p", 10), t0);
        ledger.apply(script.call(1, Some("p"), true, 20), t0);
        ledger.apply(script.call(2, Some("q"), true, 21), t0);
        let cut = CallResult::Unfinished {
            reason: RunEnd::Cancelled,
        };
        let mut jobs = ledger.apply(script.call_ended_as(1, Some("p"), cut), t0);
        assert_eq!(
            jobs,
            vec![script.facts(Some(1), "p"), script.still(Some(1), "p")],
            "a cut call's row still gets its work"
        );
        jobs.extend(ledger.apply(script.ended(RunStatus::Cancelled, 30), t0));
        ledger.apply(script.released(), t0);
        ledger.end(&label, t0);
        settle(&mut ledger, &jobs);
        assert_eq!(
            ledger.completion(&label),
            Some(Completion::ReadyForFlush),
            "the dropped run's pass still completes the session"
        );
        assert!(
            !ledger.presence_needed(&page("q")),
            "a call whose end never came holds no tab once its run is over"
        );
    }

    /// The cross-judge's interleaving (d): the tap refused a `CallEnded`. Its row never gets page
    /// facts: not at completion, not later, not after a restart.
    #[test]
    fn a_refused_call_end_is_a_permanent_absence() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        ledger.apply(script.started("p", 10), t0);
        ledger.apply(script.call(1, Some("p"), true, 20), t0);
        let root = ledger.apply(script.ended(RunStatus::Ok, 30), t0);
        assert_eq!(
            root,
            vec![script.facts(None, "p"), script.still(None, "p")],
            "only the root row gets work"
        );
        ledger.apply(script.released(), t0);
        ledger.end(&label, t0);
        settle(&mut ledger, &root);
        assert_eq!(
            ledger.completion(&label),
            Some(Completion::ReadyForFlush),
            "the absence does not hold completion up"
        );
        assert_eq!(
            ledger.apply(script.call_ended(1, Some("p")), t0),
            Vec::new(),
            "a late copy of the call's end fills nothing"
        );
        assert_eq!(
            ledger.completion(&label),
            Some(Completion::ReadyForFlush),
            "and the session stays complete"
        );

        let mut restarted = Ledger::new(IDLE);
        assert_ne!(
            restarted.admit(Some(&label), true, t0).session,
            label,
            "after a restart the label takes no run"
        );
        assert_eq!(
            restarted.apply(script.call_ended(1, Some("p")), t0),
            Vec::new(),
            "so nothing fills the row after a restart either"
        );
    }

    #[test]
    fn completion_never_turns_back() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        ledger.apply(script.started("p", 10), t0);
        let root = ledger.apply(script.ended(RunStatus::Ok, 20), t0);
        ledger.apply(script.released(), t0);
        ledger.end(&label, t0);
        settle(&mut ledger, &root);
        assert_eq!(
            ledger.completion(&label),
            Some(Completion::ReadyForFlush),
            "ready"
        );

        let stray = Script::of(&label);
        let events = [
            stray.started("p", 40),
            stray.call(1, Some("p"), true, 41),
            stray.call_ended(1, Some("p")),
            stray.created("n", Some(1)),
            stray.ended(RunStatus::Error, 42),
            stray.released(),
        ];
        for event in events {
            assert_eq!(
                ledger.apply(event, t0),
                Vec::new(),
                "an event of no admitted run causes no work"
            );
            assert_eq!(
                ledger.completion(&label),
                Some(Completion::ReadyForFlush),
                "and leaves the session complete"
            );
        }
        assert_ne!(
            ledger.admit(Some(&label), false, t0).session,
            label,
            "an admission goes to a successor"
        );
        ledger.end(&label, t0);
        ledger.expire(t0 + 10 * IDLE);
        ledger.withhold(&label);
        ledger.damage(&label);
        ledger.job_settled(&label);
        assert_eq!(
            ledger.completion(&label),
            Some(Completion::ReadyForFlush),
            "no operation reopens a finished session"
        );
    }

    #[test]
    fn a_damaged_session_never_completes() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        ledger.apply(script.started("p", 10), t0);
        ledger.damage(&label);
        ledger.apply(script.released(), t0);
        ledger.end(&label, t0);
        assert_eq!(
            ledger.completion(&label),
            Some(Completion::Damaged),
            "a session that lost a lifecycle event never completes"
        );
        assert!(
            !ledger.presence_needed(&page("p")),
            "a run whose end was lost holds no tab once its pass is released"
        );
    }

    #[test]
    fn every_finished_row_gets_its_page_facts() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        ledger.apply(script.started("p", 10), t0);
        ledger.apply(script.call(1, Some("p"), false, 11), t0);
        assert_eq!(
            ledger.apply(script.call_ended(1, Some("p")), t0),
            vec![script.facts(Some(1), "p")],
            "a call the spec does not flag gets page facts only"
        );
        ledger.apply(script.call(2, Some("p"), true, 12), t0);
        assert_eq!(
            ledger.apply(script.call_ended(2, Some("p")), t0),
            vec![script.facts(Some(2), "p"), script.still(Some(2), "p")],
            "a snapshot-flagged call also gets a screenshot"
        );
        ledger.apply(script.call(3, None, false, 13), t0);
        assert_eq!(
            ledger.apply(script.call_ended(3, None), t0),
            Vec::new(),
            "a context call names no tab"
        );
        ledger.apply(script.call(4, None, false, 14), t0);
        ledger.apply(script.created("n", Some(4)), t0);
        assert_eq!(
            ledger.apply(script.call_ended(4, Some("n")), t0),
            vec![script.facts(Some(4), "n")],
            "a context newPage's row is about the page it created"
        );
        assert_eq!(
            ledger.apply(script.ended(RunStatus::Error, 20), t0),
            vec![script.facts(None, "p"), script.still(None, "p")],
            "the root row gets both, on the run's own tab, however the run ended"
        );
    }

    #[test]
    fn a_withheld_session_gets_no_screenshots() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        ledger.apply(script.started("p", 10), t0);
        ledger.withhold(&label);
        ledger.apply(script.call(1, Some("p"), true, 11), t0);
        assert_eq!(
            ledger.apply(script.call_ended(1, Some("p")), t0),
            vec![script.facts(Some(1), "p")],
            "a withheld session's call keeps its page facts, not a screenshot"
        );
        assert_eq!(
            ledger.apply(script.ended(RunStatus::Ok, 20), t0),
            vec![script.facts(None, "p")],
            "nor does its run"
        );
    }

    #[test]
    fn a_tainted_tab_gets_no_screenshots_for_any_session() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let private = admitted(&mut ledger, "private", t0);
        ledger.apply(Script::of(&private).started("shared", 10), t0);
        ledger.withhold(&private);

        let other = admitted(&mut ledger, "other", t0);
        let script = Script::of(&other);
        ledger.apply(script.started("shared", 20), t0);
        ledger.apply(script.call(1, Some("shared"), true, 21), t0);
        assert_eq!(
            ledger.apply(script.call_ended(1, Some("shared")), t0),
            vec![script.facts(Some(1), "shared")],
            "no screenshot of a tab a withheld session touched"
        );
        ledger.apply(script.call(2, Some("own"), true, 22), t0);
        assert_eq!(
            ledger.apply(script.call_ended(2, Some("own")), t0),
            vec![script.facts(Some(2), "own"), script.still(Some(2), "own")],
            "the session's other tabs are unaffected"
        );
        assert_eq!(
            ledger.apply(script.ended(RunStatus::Ok, 30), t0),
            vec![script.facts(None, "shared")],
            "nor of its run's own tab when that is the tainted one"
        );
    }

    #[test]
    fn withholding_taints_every_tab_the_session_touches() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        ledger.apply(script.started("t1", 10), t0);
        ledger.apply(script.call(1, Some("t2"), false, 11), t0);
        let mut tainted = ledger.withhold(&label).expect("a known session");
        tainted.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        assert_eq!(
            tainted,
            vec![page("t1"), page("t2")],
            "withholding taints the tabs it touched"
        );
        assert_eq!(
            ledger.withhold(&label),
            Some(Vec::new()),
            "a repeat taints nothing new"
        );
        // The emitter inserts before the event reaches this reader of the shared set.
        ledger.taint.insert(page("t3"));
        ledger.apply(script.call(2, Some("t3"), false, 12), t0);
        assert!(
            ledger.tainted(&page("t3")),
            "a withheld session's emitter taint is visible to the ledger too"
        );
        let tabs = directory(&[("t1", 1), ("t2", 2), ("t3", 3)]);
        let view = ledger.view(&label, &tabs).expect("a known session");
        assert!(
            view.withheld && view.tabs.is_empty(),
            "a withheld session shows nothing"
        );
        assert_eq!(
            ledger.withhold(&key("unknown")),
            None,
            "an unknown session cannot be withheld"
        );
    }

    #[test]
    fn taint_outlives_the_withheld_session() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let private = admitted(&mut ledger, "private", t0);
        let script = Script::of(&private);
        ledger.apply(script.started("shared", 10), t0);
        ledger.withhold(&private);
        ledger.apply(script.ended(RunStatus::Ok, 20), t0);
        ledger.apply(script.released(), t0);
        assert_eq!(
            ledger.expire(t0 + IDLE),
            vec![private],
            "the withheld session ends"
        );

        let later = t0 + IDLE;
        let other = admitted(&mut ledger, "other", later);
        let watcher = Script::of(&other);
        ledger.apply(watcher.started("shared", 30), later);
        ledger.apply(watcher.call(1, Some("own"), false, 31), later);
        let tabs = directory(&[("shared", 1), ("own", 2)]);
        assert_eq!(
            shown(&ledger, &other, &tabs),
            ["own"],
            "its tabs stay tainted for every later session"
        );
    }

    #[test]
    fn taint_survives_a_restart() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        ledger.restore_taint([page("shared")]);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        ledger.apply(script.started("own", 10), t0);
        ledger.apply(script.call(1, Some("shared"), true, 11), t0);
        assert_eq!(
            ledger.apply(script.call_ended(1, Some("shared")), t0),
            vec![script.facts(Some(1), "shared")],
            "no screenshot of a tab tainted before the restart"
        );
        let tabs = directory(&[("shared", 1), ("own", 2)]);
        assert_eq!(
            shown(&ledger, &label, &tabs),
            ["own"],
            "and no session shows it"
        );
    }

    #[test]
    fn the_current_tab_is_the_newest_touch() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        let tabs = directory(&[("p", 1), ("q", 2)]);
        ledger.apply(script.started("p", 10), t0);
        let view = ledger.view(&label, &tabs).expect("a known session");
        assert_eq!(
            view.tabs,
            vec![ShownTab {
                target: page("p"),
                tab: TabId::parse(1).expect("a positive tab id"),
                url: "https://p.test/".to_string(),
            }],
            "a run's page is current before its first call"
        );
        ledger.apply(script.call(1, Some("q"), false, 20), t0);
        assert_eq!(
            shown(&ledger, &label, &tabs),
            ["q", "p"],
            "a call on another tab moves it"
        );
        ledger.apply(script.call(2, Some("p"), false, 30), t0);
        assert_eq!(shown(&ledger, &label, &tabs), ["p", "q"], "and back");
    }

    #[test]
    fn touches_order_by_start_time_then_arrival() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        let tabs = directory(&[("p", 1), ("q", 2), ("r", 3)]);
        ledger.apply(script.call(1, Some("p"), false, 10), t0);
        ledger.apply(script.call(2, Some("q"), false, 10), t0);
        assert_eq!(
            shown(&ledger, &label, &tabs),
            ["q", "p"],
            "with one start time, the later arrival is newer"
        );
        ledger.apply(script.call(3, Some("r"), false, 5), t0);
        assert_eq!(
            shown(&ledger, &label, &tabs),
            ["q", "p", "r"],
            "a call that started earlier but arrived later does not take over"
        );
        ledger.apply(script.call(4, Some("q"), false, 1), t0);
        ledger.apply(script.call(5, Some("p"), false, 2), t0);
        assert_eq!(
            shown(&ledger, &label, &tabs),
            ["q", "p", "r"],
            "nor does an older touch replace a tab's newest one"
        );
    }

    #[test]
    fn only_open_untainted_tabs_are_shown() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let private = admitted(&mut ledger, "private", t0);
        ledger.apply(Script::of(&private).started("t", 5), t0);
        ledger.withhold(&private);

        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        let mut tabs = directory(&[("p", 1), ("q", 2), ("t", 4)]);
        tabs.apply(DirectoryInput::Destroyed(page("q")), t0);
        for (call_id, target) in [(1, "p"), (2, "q"), (3, "r"), (4, "t")] {
            ledger.apply(script.call(call_id, Some(target), false, 10), t0);
        }
        assert_eq!(
            shown(&ledger, &label, &tabs),
            ["p"],
            "closed, undiscovered and tainted tabs are not shown"
        );
    }

    #[test]
    fn a_page_created_by_new_page_becomes_current() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        let mut tabs = directory(&[("p", 1)]);
        for (target, tab) in [("n", 9), ("m", 10)] {
            let committed = DirectoryInput::Committed {
                target: page(target),
                tab: TabId::parse(tab).expect("a positive tab id"),
            };
            tabs.apply(committed, t0);
        }
        ledger.apply(script.started("p", 10), t0);
        ledger.apply(script.call(1, None, false, 20), t0);
        ledger.apply(script.created("n", Some(1)), t0);
        assert_eq!(
            shown(&ledger, &label, &tabs),
            ["n", "p"],
            "the created page is current as of its newPage call"
        );
        ledger.apply(script.call(2, Some("p"), false, 30), t0);
        assert_eq!(
            shown(&ledger, &label, &tabs),
            ["p", "n"],
            "a later call on another tab takes over, as on /v1"
        );
        ledger.apply(script.created("m", None), t0);
        assert_eq!(
            shown(&ledger, &label, &tabs),
            ["m", "p", "n"],
            "a creation with no known call counts as the newest touch"
        );
    }

    #[test]
    fn a_created_page_sorts_as_its_new_page_call() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        let mut tabs = directory(&[("p", 1)]);
        let committed = DirectoryInput::Committed {
            target: page("n"),
            tab: TabId::parse(9).expect("a positive tab id"),
        };
        tabs.apply(committed, t0);
        ledger.apply(script.started("p", 10), t0);
        ledger.apply(script.call(1, None, false, 20), t0);
        ledger.apply(script.call(2, Some("p"), false, 25), t0);
        ledger.apply(script.created("n", Some(1)), t0);
        assert_eq!(
            shown(&ledger, &label, &tabs),
            ["p", "n"],
            "a click that started after the newPage call stays current, as /v1 orders the rows"
        );
    }

    #[test]
    fn a_view_reports_liveness_and_withholding() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let tabs = TabTable::default();
        assert_eq!(
            ledger.view(&key("unknown"), &tabs),
            None,
            "an unknown session has no view"
        );
        let label = admitted(&mut ledger, "task", t0);
        let live = FollowView {
            live: true,
            withheld: false,
            tabs: Vec::new(),
        };
        assert_eq!(ledger.view(&label, &tabs), Some(live), "admitted, live");
        ledger.end(&label, t0);
        ledger.withhold(&label);
        let over = FollowView {
            live: false,
            withheld: true,
            tabs: Vec::new(),
        };
        assert_eq!(ledger.view(&label, &tabs), Some(over), "ended and withheld");
    }

    #[test]
    fn a_presence_is_needed_while_a_live_session_or_a_run_is_on_the_tab() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        ledger.apply(script.started("p", 10), t0);
        ledger.apply(script.call(1, Some("q"), false, 11), t0);
        ledger.end(&label, t0);
        assert!(
            ledger.presence_needed(&page("p")) && ledger.presence_needed(&page("q")),
            "a run is still in flight on the tabs it touched"
        );
        assert!(
            !ledger.presence_needed(&page("r")),
            "a tab no run touched needs none"
        );
        ledger.apply(script.ended(RunStatus::Ok, 20), t0);
        assert!(
            !ledger.presence_needed(&page("p")),
            "an ended session whose runs are over needs none"
        );

        let live = admitted(&mut ledger, "live", t0);
        let between = Script::of(&live);
        ledger.apply(between.started("s", 30), t0);
        ledger.apply(between.ended(RunStatus::Ok, 31), t0);
        ledger.apply(between.released(), t0);
        assert!(
            ledger.presence_needed(&page("s")),
            "a live session's tab keeps its presence between runs"
        );
        ledger.expire(t0 + IDLE);
        assert!(
            !ledger.presence_needed(&page("s")),
            "until the session ends"
        );
    }

    #[test]
    fn events_without_an_outstanding_pass_are_ignored() {
        let t0 = Instant::now();
        let mut ledger = Ledger::new(IDLE);
        let stranger = Script::of(&key("never-admitted"));
        assert_eq!(
            ledger.apply(stranger.started("p", 10), t0),
            Vec::new(),
            "a session the ledger never admitted gets no work"
        );
        assert_eq!(
            ledger.completion(&key("never-admitted")),
            None,
            "and does not come into being from events"
        );

        let label = admitted(&mut ledger, "task", t0);
        let script = Script::of(&label);
        ledger.apply(script.started("p", 10), t0);
        ledger.apply(script.released(), t0);
        let late = Script::of(&label);
        ledger.apply(late.call(1, Some("q"), true, 20), t0);
        assert_eq!(
            ledger.apply(late.call_ended(1, Some("q")), t0),
            Vec::new(),
            "with no pass outstanding a call causes no work"
        );
        let tabs = directory(&[("p", 1), ("q", 2)]);
        assert_eq!(shown(&ledger, &label, &tabs), ["p"], "and touches no tab");
    }
}
