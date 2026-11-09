//! Neo: The host's single writer: tap → ledger, socket discovery → directory, jobs → persistence
//! fences. Public snapshots are mirrored under a short lock that the run path never takes.
//! End/privacy revocations also update that mirror synchronously; the writer imports them
//! before publishing, so a queued event cannot briefly resurrect a revoked view. A damaged
//! session that lost its release marker cannot expire; it ends only explicitly or on restart.

use std::collections::{HashMap, HashSet};
use std::future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::host::capture::CaptureHub;
use crate::host::cdp;
use crate::host::directory::{DirectoryInput, TabTable};
use crate::host::events::HostEvent;
use crate::host::link::HostLink;
use crate::host::presence::Presences;
use crate::host::privacy::Taint;
use crate::host::sessions::{Admitted, Completion, Job, JobKind, Ledger, Phase};
use crate::host::store::CallArtifacts;
use crate::host::view::FollowView;
use crate::host::{EpochMs, Host, HostError, HostOptions, SessionKey, SessionView, TabRef};
use crate::run::PageId;

/// Neo: complete, unwatched entries leave RAM an hour after settlement, bounding the host ledger without evicting active readers (design §6.9).
const RETAIN_SETTLED: Duration = Duration::from_hours(1);
/// Neo: recheck settled sessions retained by watchers once a minute after retention expires, so the host writer does not spin.
const SETTLED_SWEEP_INTERVAL: Duration = Duration::from_mins(1);
/// Neo: bound derived-job ingress to 1024 rows; refusal finalizes a row's absence before completion can be flushed (design §6.5).
pub(super) const JOB_INGRESS: usize = 1024;

type Durability = watch::Sender<Option<Result<(), HostError>>>;
type WithholdReply = (watch::Receiver<Option<Result<(), HostError>>>, bool);

pub(super) enum Command {
    Admit {
        label: Option<SessionKey>,
        reply: oneshot::Sender<Admitted>,
    },
    End(SessionKey),
    Withhold(SessionKey),
}

/// The published projection and monotonic revocations. Only host API operations and the host
/// writer lock it; neither CDP nor store I/O runs under the lock.
#[derive(Default)]
pub(super) struct Published {
    slots: HashMap<SessionKey, Slot>,
    ended: HashSet<SessionKey>,
    withholds: HashMap<SessionKey, Durability>,
    taint: Arc<Taint>,
    gates: HashMap<PageId, CancellationToken>,
}

struct Slot {
    snapshot: watch::Sender<SessionView>,
    view: FollowView,
    follow: Option<watch::Sender<FollowView>>,
    targets: Vec<PageId>,
    /// Directory facts for all ranked tabs let a synchronous privacy read skip a newly tainted
    /// current tab, using the same order as capture even while the actor cannot republish.
    tabs: HashMap<PageId, TabRef>,
}

impl Slot {
    fn notify(&self) {
        if let Some(follow) = &self.follow {
            follow.send_if_modified(|view| {
                if *view == self.view {
                    false
                } else {
                    *view = self.view.clone();
                    true
                }
            });
        }
    }
}

impl Published {
    pub(super) fn new(taint: Arc<Taint>) -> Self {
        Self {
            taint,
            ..Self::default()
        }
    }

    pub(super) fn view(&self, key: &SessionKey) -> Option<SessionView> {
        let slot = self.slots.get(key)?;
        let mut snapshot = slot.snapshot.borrow().clone();
        snapshot.current_tab = (slot.view.live && !slot.view.withheld)
            .then(|| {
                slot.view
                    .tabs
                    .iter()
                    .find(|shown| !self.taint.contains(&shown.target))
            })
            .flatten()
            .and_then(|shown| slot.tabs.get(&shown.target).cloned());
        Some(snapshot)
    }

    pub(super) fn subscribe(&self, key: &SessionKey) -> Option<watch::Receiver<SessionView>> {
        Some(self.slots.get(key)?.snapshot.subscribe())
    }

    pub(super) fn follow(&mut self, key: &SessionKey) -> Option<watch::Receiver<FollowView>> {
        let slot = self.slots.get_mut(key)?;
        Some(
            slot.follow
                .get_or_insert_with(|| watch::Sender::new(slot.view.clone()))
                .subscribe(),
        )
    }

    pub(super) fn end(&mut self, key: &SessionKey) {
        // An unknown label has no session to end; retaining it would let arbitrary requests
        // grow this mirror without admission's lifecycle accounting.
        if let Some(slot) = self.slots.get_mut(key) {
            self.ended.insert(key.clone());
            slot.view.live = false;
            slot.snapshot.send_modify(|view| {
                view.live = false;
                view.current_tab = None;
            });
            slot.notify();
        }
    }

    pub(super) fn withhold(&mut self, key: &SessionKey, targets: &[PageId]) -> WithholdReply {
        if let Some(durability) = self.withholds.get(key) {
            return (durability.subscribe(), false);
        }
        let durability = watch::Sender::new(None);
        let receiver = durability.subscribe();
        self.withholds.insert(key.clone(), durability);
        self.revoke_targets(targets);
        if let Some(slot) = self.slots.get_mut(key) {
            slot.view.withheld = true;
            slot.snapshot.send_modify(|view| view.withheld = true);
            slot.notify();
        }
        (receiver, true)
    }

    pub(super) fn withhold_failed(&self, key: &SessionKey) {
        if let Some(durability) = self.withholds.get(key) {
            durability.send_replace(Some(Err(HostError::Stopped)));
        }
    }

    fn revoke_targets(&mut self, targets: &[PageId]) {
        if targets.is_empty() {
            return;
        }
        for target in targets {
            self.gates.entry(target.clone()).or_default().cancel();
        }
        for slot in self.slots.values_mut() {
            slot.view
                .tabs
                .retain(|shown| !self.taint.contains(&shown.target));
            slot.snapshot.send_modify(|view| {
                if view
                    .current_tab
                    .as_ref()
                    .is_some_and(|tab| self.taint.contains(&tab.page))
                {
                    view.current_tab = None;
                }
            });
            slot.notify();
        }
    }
}

/// Each derived job owns one settlement marker. Moving it through the worker and artifact
/// writer carries the obligation; every refusal, cancellation, error and drop settles once.
pub(super) struct Work {
    pub(super) job: Job,
    pub(super) fallback: Option<CallArtifacts>,
    pub(super) revoked: CancellationToken,
    pub(super) newest: CancellationToken,
    pub(super) taint: Arc<Taint>,
    settled: mpsc::UnboundedSender<SessionKey>,
}

#[cfg(test)]
impl Work {
    pub(super) fn testing(job: Job, settled: mpsc::UnboundedSender<SessionKey>) -> Self {
        Self {
            job,
            fallback: None,
            revoked: CancellationToken::new(),
            newest: CancellationToken::new(),
            taint: Arc::default(),
            settled,
        }
    }
}

impl Work {
    pub(super) fn tainted(&self) -> bool {
        self.taint.contains(&self.job.page)
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = self.settled.send(self.job.session.clone());
    }
}

pub(super) struct Artifacts {
    pub(super) work: Work,
    pub(super) value: Option<CallArtifacts>,
}

pub(super) struct Jobs {
    pub(super) waiting: mpsc::Receiver<Work>,
    pub(super) results: mpsc::Sender<Artifacts>,
}

/// Started separately from runtime wiring so fake socket tests can hold jobs and flushes at
/// the exact completion interleavings, without sleeps or a second ledger writer.
pub(super) async fn start(
    options: HostOptions,
    link: Arc<HostLink>,
    presences: Arc<Presences>,
    hub: Arc<CaptureHub>,
    shutdown: CancellationToken,
) -> Result<(Host, Jobs), HostError> {
    let withheld = options.store.withheld().await?;
    link.require_directory();
    let ledger = Ledger::with_taint(options.idle, Arc::clone(&hub.privacy.taint));
    let restored: Vec<_> = withheld
        .iter()
        .flat_map(|(_, targets)| targets.iter().cloned())
        .collect();
    for (key, targets) in &withheld {
        hub.privacy.restore(key, targets);
    }
    let (host, commands) = Host::build(options, Some(hub), shutdown.clone());
    if let Ok(mut published) = host.0.published.lock() {
        published.revoke_targets(&restored);
        for (key, _) in withheld {
            published
                .withholds
                .insert(key, watch::Sender::new(Some(Ok(()))));
        }
    }
    let (jobs, waiting) = mpsc::channel(JOB_INGRESS);
    let (results, artifacts) = mpsc::channel(JOB_INGRESS);
    let (settled, settlements) = mpsc::unbounded_channel();
    let (flush, flushes) = mpsc::unbounded_channel();
    let (directory, inputs) = mpsc::channel(JOB_INGRESS);
    tokio::spawn(watch_directory(Arc::clone(&link), directory, shutdown));
    let task = Task {
        host: host.clone(),
        ledger,
        link,
        tabs: TabTable::default(),
        presences,
        commands,
        inputs,
        jobs,
        artifacts,
        settled,
        settlements,
        flush,
        flushes,
        writes: JoinSet::new(),
        completion: HashMap::new(),
        latest: HashMap::new(),
        refused_jobs: 0,
    };
    tokio::spawn(task.run());
    Ok((host, Jobs { waiting, results }))
}

/// A session moves through a flush fence once; a failed fence retries without claiming
/// completion. Complete records cannot acquire new passes or derived work.
enum Fence {
    Flushing,
    Retry(Instant),
    Complete(Instant),
}

struct DirectoryUpdate {
    input: DirectoryInput,
    epoch: u64,
}

struct Task {
    host: Host,
    ledger: Ledger,
    link: Arc<HostLink>,
    tabs: TabTable,
    presences: Arc<Presences>,
    commands: mpsc::UnboundedReceiver<Command>,
    inputs: mpsc::Receiver<DirectoryUpdate>,
    jobs: mpsc::Sender<Work>,
    artifacts: mpsc::Receiver<Artifacts>,
    settled: mpsc::UnboundedSender<SessionKey>,
    settlements: mpsc::UnboundedReceiver<SessionKey>,
    flush: mpsc::UnboundedSender<(SessionKey, bool)>,
    flushes: mpsc::UnboundedReceiver<(SessionKey, bool)>,
    writes: JoinSet<()>,
    completion: HashMap<SessionKey, Fence>,
    latest: HashMap<SessionKey, CancellationToken>,
    refused_jobs: u64,
}

impl Task {
    async fn run(mut self) {
        loop {
            self.refresh().await;
            let deadline = self.next_wake();
            tokio::select! {
                () = self.host.0.shutdown.cancelled() => break,
                event = self.host.tap().recv() => match event {
                    Some(event) => self.event(event),
                    None => break,
                },
                input = self.inputs.recv(), if !self.inputs.is_closed() || !self.inputs.is_empty() => if let Some(update) = input
                    && (matches!(update.input, DirectoryInput::Lost | DirectoryInput::Destroyed(_))
                        || update.epoch == self.link.client().epoch()) {
                        let seeded = matches!(update.input, DirectoryInput::Seeded(_));
                        self.tabs.apply(update.input, Instant::now());
                        if seeded { self.link.directory_seeded(update.epoch); }
                },
                command = self.commands.recv() => if let Some(command) = command {
                    let shutdown = self.host.0.shutdown.clone();
                    tokio::select! { () = shutdown.cancelled() => break, () = self.command(command) => {} }
                },
                artifacts = self.artifacts.recv(), if self.writes.len() < 8 && (!self.artifacts.is_closed() || !self.artifacts.is_empty()) => if let Some(artifacts) = artifacts {
                    self.persist(artifacts);
                },
                _ = self.writes.join_next(), if !self.writes.is_empty() => {},
                key = self.settlements.recv() => if let Some(key) = key {
                    self.ledger.job_settled(&key);
                },
                ack = self.flushes.recv() => if let Some((key, ok)) = ack {
                    let fence = if ok { Fence::Complete(Instant::now()) }
                        else { Fence::Retry(Instant::now() + cdp::WATCHDOG_INTERVAL) };
                    self.completion.insert(key, fence);
                },
                () = sleep_until(deadline) => {},
            }
        }
        self.host.tap().close();
        if let Ok(mut published) = self.host.0.published.lock() {
            let keys: Vec<_> = published.slots.keys().cloned().collect();
            for key in keys {
                published.end(&key);
            }
            for durability in published.withholds.values() {
                durability.send_if_modified(|value| {
                    if value.is_none() {
                        *value = Some(Err(HostError::Stopped));
                        true
                    } else {
                        false
                    }
                });
            }
        }
        self.presences.prune(|_| false).await;
        self.host.0.stopped.cancel();
    }

    fn event(&mut self, event: HostEvent) {
        for key in self.host.tap().take_damaged() {
            self.ledger.damage(&key);
        }
        if let HostEvent::PageCreated {
            page,
            tab: Some(tab),
            ..
        } = &event
        {
            self.tabs.apply(
                DirectoryInput::Committed {
                    target: page.clone(),
                    tab: *tab,
                },
                Instant::now(),
            );
        }
        let jobs = self.ledger.apply(event, Instant::now());
        for job in jobs {
            let fallback = self
                .tabs
                .recent(&job.page, Instant::now())
                .map(|tab| CallArtifacts {
                    tab: tab.tab,
                    url: tab.url.clone(),
                    title: tab.title.clone(),
                    screenshot: None,
                });
            let revoked = if let Ok(mut published) = self.host.0.published.lock() {
                published.gates.entry(job.page.clone()).or_default().clone()
            } else {
                CancellationToken::new()
            };
            let newest = CancellationToken::new();
            if job.kind == JobKind::Still
                && let Some(old) = self.latest.insert(job.session.clone(), newest.clone())
            {
                old.cancel();
            }
            let work = Work {
                job,
                fallback,
                revoked,
                newest,
                taint: Arc::clone(&self.host.0.privacy.taint),
                settled: self.settled.clone(),
            };
            if self.jobs.try_send(work).is_err() {
                self.refused_jobs += 1;
                warn!(
                    refused = self.refused_jobs,
                    "host after-call ingress full; finalizing absence"
                );
            }
        }
    }

    async fn command(&mut self, command: Command) {
        match command {
            Command::Admit { label, reply } => {
                self.ledger.expire(Instant::now());
                let known =
                    if let Some(label) = label.as_ref().filter(|key| !self.ledger.knows(key)) {
                        self.store_knows(label).await
                    } else {
                        false
                    };
                // Candidates are checked before insertion. Remapping a session after admission
                // would leave successor pointers and pass accounting under a colliding key.
                let candidate = loop {
                    let key = SessionKey::mint();
                    if !self.ledger.knows(&key) && !self.store_knows(&key).await {
                        break key;
                    }
                };
                // A synchronous end can arrive while the store check awaits. Import it at
                // the actual admission boundary, or a pass could join an already ended key.
                self.controls();
                self.ledger.expire(Instant::now());
                let admitted =
                    self.ledger
                        .admit_with_key(label.as_ref(), known, Instant::now(), candidate);
                self.publish();
                // A cancelled admission must release the pass it would have returned.
                if let Err(admitted) = reply.send(admitted) {
                    drop(
                        self.host
                            .issue_pass(admitted.session, admitted.successor_of),
                    );
                }
            }
            Command::End(key) => {
                self.ledger.end(&key, Instant::now());
            }
            Command::Withhold(key) => {
                self.ledger.withhold(&key);
                let durability = self
                    .host
                    .0
                    .published
                    .lock()
                    .ok()
                    .and_then(|published| published.withholds.get(&key).cloned());
                let store = Arc::clone(&self.host.0.options.store);
                tokio::spawn(async move {
                    let result = store.withhold(&key).await.map_err(HostError::from);
                    if let Some(durability) = durability {
                        durability.send_replace(Some(result));
                    }
                });
            }
        }
    }

    async fn store_knows(&self, key: &SessionKey) -> bool {
        self.host
            .0
            .options
            .store
            .knows(key)
            .await
            .unwrap_or_else(|error| {
                warn!(%error, session = %key, "host admission could not check store");
                false
            })
    }

    fn persist(&mut self, artifacts: Artifacts) {
        let Artifacts { work, mut value } = artifacts;
        if work.job.kind == JobKind::Still
            && (work.revoked.is_cancelled()
                || work.newest.is_cancelled()
                || work.tainted()
                || self
                    .ledger
                    .view(&work.job.session, &self.tabs)
                    .is_none_or(|view| view.withheld))
        {
            return;
        }
        let Some(value) = value.take() else {
            return;
        };
        let store = Arc::clone(&self.host.0.options.store);
        self.writes.spawn(async move {
            // This is the store admission decision, after scheduling too. A taint insert
            // while the actor was blocked invalidates a completed screenshot here.
            if work.job.kind == JobKind::Still && work.tainted() {
                return;
            }
            if let Err(error) = store
                .put_artifacts(&work.job.run, work.job.call_id, value)
                .await
            {
                warn!(%error, "host artifacts finalized as absent");
            }
            drop(work);
        });
    }

    fn controls(&mut self) {
        if let Ok(published) = self.host.0.published.lock() {
            for key in &published.ended {
                self.ledger.end(key, Instant::now());
            }
            for key in published.withholds.keys() {
                self.ledger.withhold(key);
            }
        }
    }

    async fn refresh(&mut self) {
        self.ledger.expire(Instant::now());
        self.controls();
        if self.tabs.ready()
            && let (Some(hub), Ok(published)) =
                (&self.host.0.captures, self.host.0.published.lock())
        {
            for target in published.slots.values().flat_map(|slot| &slot.targets) {
                if self.tabs.open(target).is_none() {
                    hub.end_captures(target);
                }
            }
        }
        self.publish();
        let now = Instant::now();
        let keys: Vec<_> = self.ledger.keys().cloned().collect();
        for key in keys {
            if self.ledger.completion(&key) != Some(Completion::ReadyForFlush) {
                continue;
            }
            if self
                .completion
                .get(&key)
                .is_none_or(|fence| matches!(fence, Fence::Retry(at) if *at <= now))
            {
                self.completion.insert(key.clone(), Fence::Flushing);
                let store = Arc::clone(&self.host.0.options.store);
                let flush = self.flush.clone();
                tokio::spawn(async move {
                    let result = store.flushed().await;
                    if let Err(error) = &result {
                        warn!(%error, "host completion flush failed");
                    }
                    let _ = flush.send((key, result.is_ok()));
                });
            }
        }
        self.evict(now);
        self.presences
            .prune(|target| self.tabs.open(target).is_some() && self.ledger.presence_needed(target))
            .await;
    }

    fn publish(&self) {
        let Ok(mut published) = self.host.0.published.lock() else {
            return;
        };
        let keys: Vec<_> = self.ledger.keys().cloned().collect();
        let tainted: Vec<_> = published
            .gates
            .iter()
            .filter(|(target, gate)| published.taint.contains(target) && !gate.is_cancelled())
            .map(|(target, _)| target.clone())
            .collect();
        published.revoke_targets(&tainted);
        if let Some(hub) = &self.host.0.captures {
            for target in &tainted {
                hub.end_captures(target);
            }
        }
        for key in keys {
            let Some(mut view) = self.ledger.view(&key, &self.tabs) else {
                continue;
            };
            view.live &= !published.ended.contains(&key);
            view.withheld |= published.withholds.contains_key(&key);
            view.tabs
                .retain(|shown| !published.taint.contains(&shown.target));
            let current_tab = view
                .live
                .then(|| view.tabs.first())
                .flatten()
                .and_then(|shown| {
                    let entry = self.tabs.open(&shown.target)?;
                    Some(TabRef {
                        page: shown.target.clone(),
                        tab: shown.tab,
                        url: entry.url.clone(),
                        title: entry.title.clone(),
                    })
                });
            let ended_at = match self.ledger.phase(&key) {
                Some(Phase::Ended { at, .. }) => EpochMs::now().and_then(|now| {
                    let elapsed =
                        u64::try_from(Instant::now().saturating_duration_since(at).as_millis())
                            .unwrap_or(u64::MAX);
                    EpochMs::from_millis(now.get().saturating_sub(elapsed).max(1))
                }),
                _ => None,
            };
            let mut snapshot = SessionView {
                live: view.live,
                withheld: view.withheld,
                current_tab,
                complete: matches!(self.completion.get(&key), Some(Fence::Complete(_))),
                damaged: self.ledger.completion(&key) == Some(Completion::Damaged),
                ended_at,
            };
            if let Some(old) = published.slots.get(&key) {
                snapshot.ended_at = old.snapshot.borrow().ended_at.or(ended_at);
            }
            let slot = published.slots.entry(key.clone()).or_insert_with(|| Slot {
                snapshot: watch::Sender::new(snapshot.clone()),
                view: view.clone(),
                follow: None,
                targets: Vec::new(),
                tabs: HashMap::new(),
            });
            slot.tabs = view
                .tabs
                .iter()
                .filter_map(|shown| {
                    let entry = self.tabs.open(&shown.target)?;
                    Some((
                        shown.target.clone(),
                        TabRef {
                            page: shown.target.clone(),
                            tab: shown.tab,
                            url: entry.url.clone(),
                            title: entry.title.clone(),
                        },
                    ))
                })
                .collect();
            slot.view = view;
            slot.targets = self.ledger.touched(&key);
            slot.snapshot.send_if_modified(|old| {
                if *old == snapshot {
                    false
                } else {
                    *old = snapshot;
                    true
                }
            });
            slot.notify();
        }
    }

    fn evict(&mut self, now: Instant) {
        let Ok(mut published) = self.host.0.published.lock() else {
            return;
        };
        let doomed: Vec<_> = self
            .completion
            .iter()
            .filter_map(|(key, fence)| {
                let Fence::Complete(at) = fence else {
                    return None;
                };
                let slot = published.slots.get(key)?;
                (now.saturating_duration_since(*at) >= RETAIN_SETTLED
                    && slot.snapshot.receiver_count() == 0
                    && slot
                        .follow
                        .as_ref()
                        .is_none_or(|follow| follow.receiver_count() == 0))
                .then(|| key.clone())
            })
            .collect();
        for key in doomed {
            self.ledger.forget(&key);
            self.host.0.privacy.forget(&key);
            self.completion.remove(&key);
            self.latest.remove(&key);
            published.slots.remove(&key);
            published.ended.remove(&key);
        }
    }

    fn next_wake(&self) -> Option<Instant> {
        self.completion
            .values()
            .filter_map(|fence| match fence {
                Fence::Retry(at) => Some(*at),
                Fence::Complete(at) => {
                    Some((*at + RETAIN_SETTLED).max(Instant::now() + SETTLED_SWEEP_INTERVAL))
                }
                Fence::Flushing => None,
            })
            .filter(|at| *at > Instant::now())
            .chain(self.ledger.next_expiry())
            .min()
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => time::sleep_until(at).await,
        None => future::pending().await,
    }
}

async fn watch_directory(
    link: Arc<HostLink>,
    directory: mpsc::Sender<DirectoryUpdate>,
    shutdown: CancellationToken,
) {
    let mut generations = link.generations();
    loop {
        let mut events = link.events();
        link.directory_lost();
        let _ = directory
            .send(DirectoryUpdate {
                input: DirectoryInput::Lost,
                epoch: link.client().epoch(),
            })
            .await;
        let seeded = tokio::select! {
            () = shutdown.cancelled() => return,
            _ = generations.changed() => continue,
            seed = cdp::discover_and_seed(&link) => seed,
        };
        let Ok((epoch, seed, after_reply)) = seeded else {
            tokio::select! { () = shutdown.cancelled() => return, () = time::sleep(cdp::WATCHDOG_INTERVAL) => {} }
            continue;
        };
        // The new receiver starts at the reply inside the socket reader. Drain the old
        // receiver's queued destroys (always authoritative), then replace it; post-reply
        // changes remain on the new receiver and cannot be mistaken for stale seed input.
        let queued = events.len();
        let mut lagged = false;
        for _ in 0..queued {
            match events.try_recv() {
                Ok(event) => {
                    if let Some(input @ DirectoryInput::Destroyed(_)) = cdp::directory_input(&event)
                        && directory
                            .send(DirectoryUpdate { input, epoch })
                            .await
                            .is_err()
                    {
                        return;
                    }
                }
                Err(TryRecvError::Lagged(_)) => {
                    lagged = true;
                    break;
                }
                Err(_) => break,
            }
        }
        if lagged {
            continue;
        }
        if directory
            .send(DirectoryUpdate { input: seed, epoch })
            .await
            .is_err()
        {
            return;
        }
        events = after_reply;
        loop {
            let event = tokio::select! {
                () = shutdown.cancelled() => return,
                _ = generations.changed() => break,
                event = events.recv() => event,
            };
            match event {
                Ok(event) => {
                    if let Some(input) = cdp::directory_input(&event)
                        && directory
                            .send(DirectoryUpdate { input, epoch })
                            .await
                            .is_err()
                    {
                        return;
                    }
                }
                Err(RecvError::Lagged(count)) => {
                    debug!(count, "host directory lagged; reseeding");
                    break;
                }
                Err(RecvError::Closed) => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::slice;
    use std::time::SystemTime;

    use crate::host::TabId;
    use crate::host::emit::RunEnding;
    use crate::host::tap::TAP_QUEUE;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use bytes::Bytes;
    use futures_util::StreamExt;
    use futures_util::future::BoxFuture;
    use serde_json::json;
    use tokio::sync::{Mutex, Notify};
    use tokio::task;

    use super::*;
    use crate::fake_host_cdp::FakeHostCdp;
    use crate::host::store::{HostStore, MemoryStore, StoreError, WithheldSessions};
    use crate::host::{LiveError, Quality, RunEmitter};
    use crate::run::{CallResult, RunId, RunStatus};

    #[derive(Default)]
    struct Store {
        memory: MemoryStore,
        blocked: AtomicBool,
        flushes: AtomicUsize,
        release: Notify,
        checks: Mutex<Vec<SessionKey>>,
        collide: AtomicBool,
        admission_blocked: AtomicBool,
        admission_waiting: Notify,
        admission_release: Notify,
    }

    impl HostStore for Store {
        fn knows<'a>(&'a self, key: &'a SessionKey) -> BoxFuture<'a, Result<bool, StoreError>> {
            Box::pin(async move {
                self.checks.lock().await.push(key.clone());
                if (key.as_str() == "hold-admission" || key.as_str().len() == 8)
                    && self.admission_blocked.load(Ordering::SeqCst)
                {
                    self.admission_waiting.notify_one();
                    while self.admission_blocked.load(Ordering::SeqCst) {
                        self.admission_release.notified().await;
                    }
                }
                if key.as_str().len() == 8 && self.collide.swap(false, Ordering::SeqCst) {
                    return Ok(true);
                }
                self.memory.knows(key).await
            })
        }
        fn flushed(&self) -> BoxFuture<'_, Result<(), StoreError>> {
            Box::pin(async move {
                self.flushes.fetch_add(1, Ordering::SeqCst);
                while self.blocked.load(Ordering::SeqCst) {
                    self.release.notified().await;
                }
                Ok(())
            })
        }
        fn put_artifacts<'a>(
            &'a self,
            run: &'a RunId,
            call: Option<u64>,
            value: CallArtifacts,
        ) -> BoxFuture<'a, Result<(), StoreError>> {
            self.memory.put_artifacts(run, call, value)
        }
        fn withhold<'a>(&'a self, key: &'a SessionKey) -> BoxFuture<'a, Result<(), StoreError>> {
            self.memory.withhold(key)
        }
        fn withheld(&self) -> BoxFuture<'_, Result<WithheldSessions, StoreError>> {
            self.memory.withheld()
        }
    }

    struct Harness {
        browser: FakeHostCdp,
        link: Arc<HostLink>,
        host: Host,
        jobs: Jobs,
        store: Arc<Store>,
    }

    impl Harness {
        async fn new(idle: Duration) -> Self {
            let browser = FakeHostCdp::start().await;
            browser.set_tabs(vec![
                FakeHostCdp::tab("one", 1, "https://one.test/"),
                FakeHostCdp::tab("two", 2, "https://two.test/"),
            ]);
            let shutdown = CancellationToken::new();
            let link = HostLink::connect(browser.port(), shutdown.clone())
                .await
                .unwrap();
            let presences = Arc::new(Presences::new(Arc::clone(&link)));
            let hub = CaptureHub::new(
                Arc::clone(&link),
                Arc::clone(&presences),
                Arc::default(),
                shutdown.clone(),
            );
            let store = Arc::new(Store::default());
            let mut options =
                HostOptions::new(browser.port(), 8, Arc::clone(&store) as Arc<dyn HostStore>);
            options.idle = idle;
            let (host, jobs) = start(options, Arc::clone(&link), presences, hub, shutdown)
                .await
                .unwrap();
            browser.wait_for("Browser.getTabs", 1).await;
            Self {
                browser,
                link,
                host,
                jobs,
                store,
            }
        }
        async fn job(&mut self) -> Work {
            time::timeout(Duration::from_secs(5), self.jobs.waiting.recv())
                .await
                .unwrap()
                .unwrap()
        }
        async fn settled(&mut self, count: usize) {
            for _ in 0..count {
                drop(self.job().await);
            }
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            self.host.shutdown();
        }
    }

    async fn until(
        host: &Host,
        key: &SessionKey,
        predicate: impl Fn(&SessionView) -> bool,
    ) -> SessionView {
        let mut changes = host.0.published.lock().unwrap().subscribe(key).unwrap();
        let view = time::timeout(Duration::from_secs(10), changes.wait_for(predicate))
            .await
            .unwrap()
            .unwrap();
        view.clone()
    }

    fn run(host: &Host, key: &SessionKey, target: &str) -> (RunEmitter, RunEnding) {
        let emitter = RunEmitter::new(Some(host), Some(key), &RunId::mint());
        let ending = emitter.run_started(&PageId::new(target.to_string()));
        (emitter, ending)
    }

    #[tokio::test(start_paused = true)]
    async fn completion_waits_for_release_jobs_and_a_flush_requested_after_both() {
        let mut h = Harness::new(Duration::from_mins(1)).await;
        h.store.blocked.store(true, Ordering::SeqCst);
        let pass = h.host.admit_run(Some(&SessionKey::new("label"))).await;
        let key = pass.session().clone();
        let (emitter, ending) = run(&h.host, &key, "one");
        h.host.end_session(&key);
        assert!(
            !h.host.view(&key).unwrap().live,
            "end revokes synchronously"
        );
        assert_eq!(h.store.flushes.load(Ordering::SeqCst), 0);
        emitter.call_started(
            1,
            Some(&PageId::new("two".to_string())),
            false,
            SystemTime::now(),
        );
        emitter.call_ended(1, &CallResult::Ok);
        ending.ended(RunStatus::Ok);
        h.settled(3).await; // call facts; root facts and still
        time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            h.store.flushes.load(Ordering::SeqCst),
            0,
            "a live pass fences the flush even after RunEnded"
        );
        drop(pass);
        let before_flush = h
            .host
            .wait_complete(&key, Instant::now() + Duration::from_secs(1))
            .await
            .unwrap();
        assert!(!before_flush.complete);
        assert_eq!(h.store.flushes.load(Ordering::SeqCst), 1);
        h.store.blocked.store(false, Ordering::SeqCst);
        h.store.release.notify_one();
        assert!(until(&h.host, &key, |view| view.complete).await.complete);
        let successor = h.host.admit_run(Some(&key)).await;
        assert_ne!(successor.session(), &key);
        assert!(
            h.host.view(&key).unwrap().complete,
            "completion never reopens"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn root_jobs_in_the_tap_prevent_an_early_flush_and_drop_settles_them() {
        let mut h = Harness::new(Duration::from_mins(1)).await;
        let pass = h.host.admit_run(None).await;
        let key = pass.session().clone();
        let (_, ending) = run(&h.host, &key, "one");
        ending.ended(RunStatus::Ok);
        h.host.end_session(&key);
        drop(pass);
        let first = h.job().await;
        let second = h.job().await;
        assert_eq!(
            h.store.flushes.load(Ordering::SeqCst),
            0,
            "root jobs register before release is consumed"
        );
        drop(first);
        drop(second);
        assert!(until(&h.host, &key, |view| view.complete).await.complete);
    }

    #[tokio::test(start_paused = true)]
    async fn admission_checks_restart_and_minted_collisions_and_shares_successors() {
        let h = Harness::new(Duration::from_mins(1)).await;
        let old = SessionKey::new("stored-label");
        h.store.memory.record_run(&old, &RunId::mint(), &[]).await;
        h.store.collide.store(true, Ordering::SeqCst);
        let first = h.host.admit_run(Some(&old)).await;
        let second = h.host.admit_run(Some(&old)).await;
        assert_ne!(first.session(), &old);
        assert_eq!(first.session(), second.session());
        assert_eq!(first.successor_of(), Some(&old));
        let checks = h.store.checks.lock().await;
        assert_eq!(checks.iter().filter(|key| *key == &old).count(), 1);
        assert_ne!(
            first.session(),
            &checks[1],
            "the store-known minted key was rejected"
        );
        drop(checks);
        let unknown = SessionKey::new("never-seen");
        let (a, b) = tokio::join!(
            h.host.admit_run(Some(&unknown)),
            h.host.admit_run(Some(&unknown))
        );
        assert_eq!(a.session(), &unknown);
        assert_eq!(b.session(), &unknown);
        h.host.end_session(&unknown);
        let a = h.host.admit_run(Some(&unknown)).await;
        let b = h.host.admit_run(Some(&unknown)).await;
        assert_eq!(a.session(), b.session());
        assert_ne!(a.session(), &unknown);
    }

    #[tokio::test(start_paused = true)]
    async fn followers_move_on_touches_committed_tab_ids_and_directory_events() {
        let h = Harness::new(Duration::from_mins(1)).await;
        let pass = h.host.admit_run(None).await;
        let key = pass.session();
        let (emitter, _ending) = run(&h.host, key, "one");
        until(&h.host, key, |view| view.current_tab.is_some()).await;
        let mut follow = h.host.follow(key).unwrap();
        emitter.call_started(1, None, false, SystemTime::now());
        emitter.page_created(1, &PageId::new("new".to_string()), TabId::parse(3));
        time::timeout(Duration::from_secs(5), follow.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            follow.borrow_and_update().tabs[0].tab.get(),
            3,
            "the commit maps its tab id before discovery"
        );
        h.browser
            .discovery("Target.targetCreated", "new", 3, "https://new.test/");
        until(&h.host, key, |view| {
            view.current_tab
                .as_ref()
                .is_some_and(|tab| tab.url == "https://new.test/")
        })
        .await;
        h.browser.discovery(
            "Target.targetInfoChanged",
            "new",
            3,
            "https://changed.test/",
        );
        until(&h.host, key, |view| {
            view.current_tab
                .as_ref()
                .is_some_and(|tab| tab.url == "https://changed.test/")
        })
        .await;
        h.browser.destroyed("new");
        until(&h.host, key, |view| {
            view.current_tab
                .as_ref()
                .is_some_and(|tab| tab.tab.get() == 1)
        })
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn idle_expiry_ends_a_follower_and_completes_without_an_api_poll() {
        let mut h = Harness::new(Duration::from_secs(1)).await;
        let pass = h.host.admit_run(None).await;
        let key = pass.session().clone();
        let (_, ending) = run(&h.host, &key, "one");
        until(&h.host, &key, |view| view.current_tab.is_some()).await;
        let mut follow = h.host.follow(&key).unwrap();
        ending.ended(RunStatus::Ok);
        drop(pass);
        h.settled(2).await;
        let done = until(&h.host, &key, |view| view.complete).await;
        assert!(!done.live);
        assert!(done.ended_at.is_some());
        follow.changed().await.unwrap();
        assert!(!follow.borrow().live);
    }

    #[tokio::test(start_paused = true)]
    async fn withhold_revokes_shared_targets_and_survives_an_unpolled_future() {
        let mut h = Harness::new(Duration::from_mins(1)).await;
        let a = h.host.admit_run(None).await;
        let b = h.host.admit_run(None).await;
        let (emitter, _ending) = run(&h.host, a.session(), "one");
        let (_, _other) = run(&h.host, b.session(), "one");
        until(&h.host, a.session(), |view| view.current_tab.is_some()).await;
        until(&h.host, b.session(), |view| view.current_tab.is_some()).await;
        let other = h.host.follow(b.session()).unwrap();
        emitter.call_started(
            1,
            Some(&PageId::new("one".to_string())),
            true,
            SystemTime::now(),
        );
        emitter.call_ended(1, &CallResult::Ok);
        drop(h.job().await);
        let still = h.job().await;
        drop(h.host.withhold(a.session()));
        assert!(h.host.view(a.session()).unwrap().withheld);
        assert!(
            other.borrow().tabs.is_empty(),
            "shared targets revoke synchronously too"
        );
        assert!(
            still.revoked.is_cancelled(),
            "pending screenshots cannot start"
        );
        h.host.withhold(a.session()).await.unwrap();
        assert_eq!(h.store.memory.withheld().await.unwrap().len(), 1);
        assert_eq!(
            h.host
                .screencast(a.session(), Quality::Low, None)
                .await
                .err(),
            Some(LiveError::Withheld)
        );
        drop(still);
    }

    #[tokio::test(start_paused = true)]
    async fn complete_entries_are_evicted_after_an_hour() {
        let mut h = Harness::new(Duration::from_mins(1)).await;
        let pass = h.host.admit_run(Some(&SessionKey::new("evict-me"))).await;
        let key = pass.session().clone();
        let run_id = RunId::mint();
        h.store
            .memory
            .record_run(
                &key,
                &run_id,
                slice::from_ref(&PageId::new("one".to_string())),
            )
            .await;
        h.host.end_session(&key);
        drop(pass);
        until(&h.host, &key, |view| view.complete).await;
        time::advance(RETAIN_SETTLED + Duration::from_secs(1)).await;
        time::timeout(Duration::from_secs(5), async {
            while h.host.view(&key).is_some() {
                task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(h.host.view(&key).is_none());
        let next = h.host.admit_run(Some(&key)).await;
        assert_ne!(next.session(), &key);
        h.jobs.waiting.close();
    }

    #[tokio::test(start_paused = true)]
    async fn seed_reply_boundary_preserves_later_changes_and_prior_destroys() {
        let h = Harness::new(Duration::from_mins(1)).await;
        let pass = h.host.admit_run(None).await;
        let (_, _ending) = run(&h.host, pass.session(), "one");
        until(&h.host, pass.session(), |view| view.current_tab.is_some()).await;
        h.browser.around_seed(vec![json!({"method":"Target.targetInfoChanged","params":{"targetInfo":{
            "targetId":"one","type":"page","tabId":1,"url":"stale","title":"one","attached":false,"canAccessOpener":false}}})],
            vec![json!({"method":"Target.targetInfoChanged","params":{"targetInfo":{
            "targetId":"one","type":"page","tabId":1,"url":"after","title":"one","attached":false,"canAccessOpener":false}}})]);
        h.browser.drop_connection();
        h.browser.wait_for("Browser.getTabs", 2).await;
        until(&h.host, pass.session(), |view| {
            view.current_tab
                .as_ref()
                .is_some_and(|tab| tab.url == "after")
        })
        .await;
    }
    #[tokio::test(start_paused = true)]
    async fn a_refused_call_end_stays_absent_after_completion() {
        let mut h = Harness::new(Duration::from_mins(1)).await;
        let pass = h.host.admit_run(None).await;
        let key = pass.session().clone();
        let run = RunId::mint();
        let emitter = RunEmitter::new(Some(&h.host), Some(&key), &run);
        let ending = emitter.run_started(&PageId::new("one".to_string()));
        emitter.call_started(
            1,
            Some(&PageId::new("one".to_string())),
            false,
            SystemTime::now(),
        );
        // With no await the actor cannot consume this thread's FIFO. Fill its per-call band
        // with harmless unknown-session events, then refuse this row's CallEnded.
        let filler = HostEvent::CallStarted {
            session: SessionKey::new("unadmitted"),
            run: RunId::mint(),
            call_id: 1,
            page: None,
            snapshot: false,
            at: EpochMs::MIN,
        };
        for _ in 0..TAP_QUEUE {
            let _ = h.host.tap().try_push(filler.clone());
        }
        emitter.call_ended(1, &CallResult::Ok);
        ending.ended(RunStatus::Ok);
        h.host.end_session(&key);
        drop(pass);
        let facts = h.job().await;
        assert_eq!(facts.job.call_id, None);
        let value = facts.fallback.clone();
        h.jobs
            .results
            .send(Artifacts { work: facts, value })
            .await
            .ok()
            .unwrap();
        drop(h.job().await);
        until(&h.host, &key, |view| view.complete).await;
        assert_eq!(h.store.memory.artifacts(&run).await.len(), 1);
        assert!(!h.store.memory.artifacts(&run).await.contains_key(&Some(1)));
        h.browser
            .discovery("Target.targetInfoChanged", "one", 1, "later");
        time::sleep(Duration::from_millis(100)).await;
        assert!(!h.store.memory.artifacts(&run).await.contains_key(&Some(1)));
    }
    #[tokio::test(start_paused = true)]
    async fn a_lagged_directory_ring_reseeds_on_the_same_epoch() {
        let h = Harness::new(Duration::from_mins(1)).await;
        let pass = h.host.admit_run(None).await;
        let (_, _ending) = run(&h.host, pass.session(), "one");
        until(&h.host, pass.session(), |view| view.current_tab.is_some()).await;
        let marker = h
            .link
            .attach(&PageId::new("one".to_string()))
            .await
            .unwrap();
        h.store.admission_blocked.store(true, Ordering::SeqCst);
        let blocked = tokio::spawn({
            let host = h.host.clone();
            async move {
                host.admit_run(Some(&SessionKey::new("hold-admission")))
                    .await
            }
        });
        h.store.admission_waiting.notified().await;
        h.browser
            .set_tabs(vec![FakeHostCdp::tab("one", 1, "reseeded")]);
        for _ in 0..8192 {
            h.browser
                .discovery("Target.targetInfoChanged", "one", 1, "lost");
        }
        h.browser.detach(marker.session.as_str());
        time::timeout(Duration::from_secs(20), marker.gone())
            .await
            .unwrap();
        h.store.admission_blocked.store(false, Ordering::SeqCst);
        h.store.admission_release.notify_one();
        drop(blocked.await.unwrap());
        h.browser.wait_for("Browser.getTabs", 2).await;
        until(&h.host, pass.session(), |view| {
            view.current_tab
                .as_ref()
                .is_some_and(|tab| tab.url == "reseeded")
        })
        .await;
        assert!(
            h.browser
                .commands()
                .iter()
                .all(|command| command.connection == 1),
            "lag alone triggered the reseed"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_follower_releases_only_after_the_new_epochs_seed_reply() {
        let h = Harness::new(Duration::from_mins(1)).await;
        let pass = h.host.admit_run(None).await;
        let (_, _ending) = run(&h.host, pass.session(), "one");
        until(&h.host, pass.session(), |view| view.current_tab.is_some()).await;
        let mut frames = h
            .host
            .screencast(pass.session(), Quality::Low, None)
            .await
            .unwrap();
        h.browser.delay_seeds(Duration::from_secs(2));
        h.browser
            .set_tabs(vec![FakeHostCdp::tab("one", 1, "fresh-epoch")]);
        h.browser.drop_connection();
        h.browser.wait_for("Browser.getTabs", 2).await;
        assert_eq!(
            h.browser.sent("Page.startScreencast").len(),
            1,
            "the new socket cannot capture before its seed reply"
        );
        let capture = h.browser.session_of("Page.startScreencast", 1).await;
        h.browser.frame(&capture, b"fresh", Some(1.0));
        let frame = time::timeout(Duration::from_secs(10), frames.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame.url, "fresh-epoch");
    }

    #[tokio::test(start_paused = true)]
    async fn startup_restores_withheld_flags_and_target_taint() {
        let h = Harness::new(Duration::from_mins(1)).await;
        let old = SessionKey::new("private-before-restart");
        let page = PageId::new("one".to_string());
        h.store
            .memory
            .record_run(&old, &RunId::mint(), slice::from_ref(&page))
            .await;
        h.store.memory.withhold(&old).await.unwrap();
        h.host.shutdown();
        h.host.stopped().await;
        let shutdown = CancellationToken::new();
        let link = HostLink::connect(h.browser.port(), shutdown.clone())
            .await
            .unwrap();
        let presences = Arc::new(Presences::new(Arc::clone(&link)));
        let hub = CaptureHub::new(
            Arc::clone(&link),
            Arc::clone(&presences),
            Arc::default(),
            shutdown.clone(),
        );
        let (host, mut jobs) = start(
            HostOptions::new(
                h.browser.port(),
                1,
                Arc::clone(&h.store) as Arc<dyn HostStore>,
            ),
            link,
            presences,
            hub,
            shutdown,
        )
        .await
        .unwrap();
        let pass = host.admit_run(Some(&old)).await;
        assert!(host.view(&old).unwrap().withheld);
        assert_ne!(pass.session(), &old);
        let (_, ending) = run(&host, pass.session(), "one");
        ending.ended(RunStatus::Ok);
        let facts = time::timeout(Duration::from_secs(5), jobs.waiting.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(facts.job.kind, JobKind::Facts);
        assert!(
            host.follow(pass.session())
                .unwrap()
                .borrow()
                .tabs
                .is_empty()
        );
        let key = pass.session().clone();
        host.end_session(&key);
        drop(pass);
        drop(facts);
        assert!(until(&host, &key, |view| view.complete).await.complete);
        assert!(
            jobs.waiting.try_recv().is_err(),
            "a restored tainted tab gets no still job"
        );
        host.shutdown();
        host.stopped().await;
    }

    #[tokio::test(start_paused = true)]
    async fn withholding_stops_shared_streams_and_discards_the_other_sessions_in_flight_still() {
        let mut h = Harness::new(Duration::from_mins(1)).await;
        let a = h.host.admit_run(None).await;
        let b = h.host.admit_run(None).await;
        let (_, _a_end) = run(&h.host, a.session(), "one");
        let (other, _b_end) = run(&h.host, b.session(), "one");
        until(&h.host, a.session(), |view| view.current_tab.is_some()).await;
        until(&h.host, b.session(), |view| view.current_tab.is_some()).await;
        let mut first = h
            .host
            .screencast(a.session(), Quality::Low, None)
            .await
            .unwrap();
        let mut second = h
            .host
            .screencast(b.session(), Quality::Low, None)
            .await
            .unwrap();
        let capture = h.browser.session_of("Page.startScreencast", 0).await;
        other.call_started(
            1,
            Some(&PageId::new("one".to_string())),
            true,
            SystemTime::now(),
        );
        other.call_ended(1, &CallResult::Ok);
        drop(h.job().await);
        let work = h.job().await;
        let run = work.job.run.clone();
        let mut value = work.fallback.clone().unwrap();
        value.screenshot = Some(Bytes::from_static(b"late screenshot"));
        h.store.memory.record_run(b.session(), &run, &[]).await;
        h.host.withhold(a.session()).await.unwrap();
        h.browser.frame(&capture, b"secret", Some(1.0));
        h.jobs
            .results
            .send(Artifacts {
                work,
                value: Some(value),
            })
            .await
            .ok()
            .unwrap();
        assert!(
            time::timeout(Duration::from_secs(5), first.next())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            time::timeout(Duration::from_millis(100), second.next())
                .await
                .is_err(),
            "the other live session gets no frame of the tainted tab"
        );
        assert!(
            h.store.memory.artifacts(&run).await.is_empty(),
            "revoked in-flight results cannot reach persistence"
        );
    }
    #[tokio::test(start_paused = true)]
    async fn an_end_during_a_store_await_closes_admission_under_that_key() {
        let h = Harness::new(Duration::from_mins(1)).await;
        let label = SessionKey::new("live-label");
        let first = h.host.admit_run(Some(&label)).await;
        h.store.admission_blocked.store(true, Ordering::SeqCst);
        let admitting = tokio::spawn({
            let host = h.host.clone();
            let label = label.clone();
            async move { host.admit_run(Some(&label)).await }
        });
        h.store.admission_waiting.notified().await;
        h.host.end_session(&label);
        h.store.admission_blocked.store(false, Ordering::SeqCst);
        h.store.admission_release.notify_one();
        let next = admitting.await.unwrap();
        assert_ne!(
            next.session(),
            first.session(),
            "an end during the candidate check still forces a successor"
        );
        assert_eq!(next.successor_of(), Some(&label));
    }

    #[tokio::test(start_paused = true)]
    async fn privacy_before_a_first_run_revokes_its_target_before_the_tap_is_consumed() {
        let h = Harness::new(Duration::from_mins(1)).await;
        let watching = h.host.admit_run(None).await;
        let (_, _watching_run) = run(&h.host, watching.session(), "one");
        until(&h.host, watching.session(), |view| {
            view.current_tab.is_some()
        })
        .await;
        let mut frames = h
            .host
            .screencast(watching.session(), Quality::Low, None)
            .await
            .unwrap();
        let capture = h.browser.session_of("Page.startScreencast", 0).await;

        let private = h.host.admit_run(None).await;
        h.host.withhold(private.session()).await.unwrap();
        h.store.admission_blocked.store(true, Ordering::SeqCst);
        let blocked = tokio::spawn({
            let host = h.host.clone();
            async move {
                host.admit_run(Some(&SessionKey::new("hold-admission")))
                    .await
            }
        });
        h.store.admission_waiting.notified().await;

        // A private run may emit RunStarted and execute its first input while the actor is
        // awaiting unrelated store I/O. The emitter must taint before that queued marker is read.
        let (_, _private_run) = run(&h.host, private.session(), "one");
        h.browser
            .frame(&capture, b"pixels after private input", Some(1.0));
        let received = time::timeout(Duration::from_millis(100), frames.next()).await;
        h.store.admission_blocked.store(false, Ordering::SeqCst);
        h.store.admission_release.notify_one();
        drop(blocked.await.unwrap());
        assert!(
            !matches!(received, Ok(Some(_))),
            "another session must not receive pixels after a private run touches its tab"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_withheld_call_revokes_shared_pixels_before_the_tap_is_consumed() {
        use crate::fake_browser::{self, FakeBrowser, Replies};
        use crate::run::RunRequest;
        use crate::runtime::Runtime;

        let h = Harness::new(Duration::from_mins(1)).await;
        let browser = FakeBrowser::start(Replies::default()).await;
        browser.page.lock().unwrap().tabs.push(json!({
            "targetId":"two", "tabId":2, "url":"https://two.test/", "windowId":1,
            "liveBrowserContextId":"profile", "isActive":false,
        }));
        h.browser.discovery(
            "Target.targetCreated",
            fake_browser::PAGE,
            41,
            fake_browser::PAGE_URL,
        );
        let watching = h.host.admit_run(None).await;
        let (watcher, _watching_run) = run(&h.host, watching.session(), "one");
        watcher.call_started(
            1,
            Some(&PageId::new(fake_browser::PAGE.to_string())),
            false,
            SystemTime::now(),
        );
        until(&h.host, watching.session(), |view| {
            view.current_tab
                .as_ref()
                .is_some_and(|tab| tab.page.as_str() == fake_browser::PAGE)
        })
        .await;
        let mut frames = h
            .host
            .screencast(watching.session(), Quality::Low, None)
            .await
            .unwrap();
        let capture = h.browser.session_of("Page.startScreencast", 0).await;
        let mut video = h
            .host
            .video(watching.session(), Quality::Low, None)
            .await
            .unwrap();

        let private = h.host.admit_run(None).await;
        h.host.withhold(private.session()).await.unwrap();
        h.store.admission_blocked.store(true, Ordering::SeqCst);
        let blocked = tokio::spawn({
            let host = h.host.clone();
            async move {
                host.admit_run(Some(&SessionKey::new("hold-admission")))
                    .await
            }
        });
        h.store.admission_waiting.notified().await;

        // RunStarted touches another tab. The real dispatcher's later CallStarted, emitted
        // before its input task, must revoke the watched tab while the host actor is blocked.
        let runtime = Runtime::connect(browser.cdp().await)
            .await
            .unwrap()
            .with_host_for_test(h.host.clone());
        let outcome = runtime.run(RunRequest {
            code: format!("const tabs = await context.pages(); const watched = tabs.find(p => p.url() === {}); await watched.click('#go'); return 'clicked'", json!(fake_browser::PAGE_URL)),
            page: PageId::new("two".to_string()), timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(), session: Some(private.session().clone()), artifacts: None,
        }).await;
        h.browser
            .frame(&capture, b"pixels after private click", Some(1.0));
        let received = time::timeout(Duration::from_millis(100), frames.next()).await;
        let video_after = time::timeout(Duration::from_secs(5), video.next()).await;
        let shown_after = h.host.view(watching.session()).unwrap().current_tab;
        h.store.admission_blocked.store(false, Ordering::SeqCst);
        h.store.admission_release.notify_one();
        drop(blocked.await.unwrap());
        assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
        assert_eq!(outcome.value, Some(json!("clicked")));
        assert_eq!(
            shown_after.as_ref().map(|tab| tab.page.as_str()),
            Some("one"),
            "a snapshot uses the same untainted fallback as the follower while the actor is blocked"
        );
        assert!(
            matches!(video_after, Ok(None)),
            "video cannot start a fragment after the shared insert"
        );
        assert!(
            !matches!(received, Ok(Some(_))),
            "CallStarted must suppress another session's next part without waiting for the actor"
        );
    }
}
