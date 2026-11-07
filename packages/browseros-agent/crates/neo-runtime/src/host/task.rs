//! The host's single writer: tap → ledger, socket discovery → directory, jobs → persistence
//! fences. Public snapshots are mirrored under a short lock that the run path never takes.
//! End/privacy revocations also update that mirror synchronously; the writer imports them
//! before publishing, so a queued event cannot briefly resurrect a revoked view.

use std::collections::{HashMap, HashSet};
use std::future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::host::capture::CaptureHub;
use crate::host::cdp;
use crate::host::directory::{DirectoryInput, TabTable};
use crate::host::events::HostEvent;
use crate::host::link::HostLink;
use crate::host::presence::Presences;
use crate::host::sessions::{Admitted, Completion, Job, JobKind, Ledger, Phase};
use crate::host::store::CallArtifacts;
use crate::host::view::FollowView;
use crate::host::{EpochMs, Host, HostError, HostOptions, SessionKey, SessionView, TabRef};
use crate::run::PageId;

/// Neo policy (§6.9): complete, unwatched entries leave RAM an hour after settlement.
const RETAIN_SETTLED: Duration = Duration::from_hours(1);
/// Neo policy (§6.5): refusal finalizes the row's absence before completion can be flushed.
pub(super) const JOB_INGRESS: usize = 1024;

type Durability = watch::Sender<Option<Result<(), HostError>>>;
type WithholdReply = (
    watch::Receiver<Option<Result<(), HostError>>>,
    bool,
    Vec<PageId>,
);

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
    taint: HashSet<PageId>,
    gates: HashMap<PageId, CancellationToken>,
}

struct Slot {
    snapshot: watch::Sender<SessionView>,
    view: FollowView,
    follow: Option<watch::Sender<FollowView>>,
    targets: Vec<PageId>,
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
    pub(super) fn view(&self, key: &SessionKey) -> Option<SessionView> {
        Some(self.slots.get(key)?.snapshot.borrow().clone())
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

    pub(super) fn withhold(&mut self, key: &SessionKey) -> WithholdReply {
        if let Some(durability) = self.withholds.get(key) {
            return (durability.subscribe(), false, Vec::new());
        }
        let durability = watch::Sender::new(None);
        let receiver = durability.subscribe();
        self.withholds.insert(key.clone(), durability);
        let targets = self
            .slots
            .get(key)
            .map(|slot| slot.targets.clone())
            .unwrap_or_default();
        self.revoke_targets(&targets);
        if let Some(slot) = self.slots.get_mut(key) {
            slot.view.withheld = true;
            slot.snapshot.send_modify(|view| view.withheld = true);
            slot.notify();
        }
        (receiver, true, targets)
    }

    fn revoke_targets(&mut self, targets: &[PageId]) {
        for target in targets {
            self.taint.insert(target.clone());
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
    settled: mpsc::UnboundedSender<SessionKey>,
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
    pub(super) results: mpsc::UnboundedSender<Artifacts>,
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
    let mut ledger = Ledger::new(options.idle);
    let restored: Vec<_> = withheld
        .into_iter()
        .flat_map(|(_, targets)| targets)
        .collect();
    ledger.restore_taint(restored.iter().cloned());
    let (host, commands) = Host::build(options, Some(hub), shutdown.clone());
    if let Ok(mut published) = host.0.published.lock() {
        published.revoke_targets(&restored);
    }
    let (jobs, waiting) = mpsc::channel(JOB_INGRESS);
    let (results, artifacts) = mpsc::unbounded_channel();
    let (settled, settlements) = mpsc::unbounded_channel();
    let (flush, flushes) = mpsc::unbounded_channel();
    let (directory, inputs) = mpsc::channel(JOB_INGRESS);
    tokio::spawn(watch_directory(Arc::clone(&link), directory, shutdown));
    let task = Task {
        host: host.clone(),
        ledger,
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
        completion: HashMap::new(),
        latest: HashMap::new(),
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

struct Task {
    host: Host,
    ledger: Ledger,
    tabs: TabTable,
    presences: Arc<Presences>,
    commands: mpsc::UnboundedReceiver<Command>,
    inputs: mpsc::Receiver<DirectoryInput>,
    jobs: mpsc::Sender<Work>,
    artifacts: mpsc::UnboundedReceiver<Artifacts>,
    settled: mpsc::UnboundedSender<SessionKey>,
    settlements: mpsc::UnboundedReceiver<SessionKey>,
    flush: mpsc::UnboundedSender<(SessionKey, bool)>,
    flushes: mpsc::UnboundedReceiver<(SessionKey, bool)>,
    completion: HashMap<SessionKey, Fence>,
    latest: HashMap<SessionKey, CancellationToken>,
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
                input = self.inputs.recv() => if let Some(input) = input {
                    self.tabs.apply(input, Instant::now());
                },
                command = self.commands.recv() => if let Some(command) = command {
                    self.command(command).await;
                },
                artifacts = self.artifacts.recv() => if let Some(artifacts) = artifacts {
                    self.persist(artifacts);
                },
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
        }
        self.presences.prune(|_| false).await;
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
                settled: self.settled.clone(),
            };
            if self.jobs.try_send(work).is_err() {
                warn!("host after-call ingress full; finalizing absence");
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

    fn persist(&self, artifacts: Artifacts) {
        let Artifacts { work, mut value } = artifacts;
        if work.job.kind == JobKind::Still
            && (work.revoked.is_cancelled()
                || work.newest.is_cancelled()
                || self.ledger.tainted(&work.job.page)
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
        tokio::spawn(async move {
            if let Err(error) = store
                .put_artifacts(&work.job.run, work.job.call_id, value)
                .await
            {
                warn!(%error, "host artifacts finalized as absent");
            }
            drop(work);
        });
    }

    async fn refresh(&mut self) {
        self.ledger.expire(Instant::now());
        if let Ok(published) = self.host.0.published.lock() {
            for key in &published.ended {
                self.ledger.end(key, Instant::now());
            }
            for key in published.withholds.keys() {
                self.ledger.withhold(key);
            }
        }
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
        let tainted: Vec<_> = keys
            .iter()
            .flat_map(|key| self.ledger.touched(key))
            .filter(|target| self.ledger.tainted(target))
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
            });
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
                Fence::Complete(at) => Some(*at + RETAIN_SETTLED),
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
    directory: mpsc::Sender<DirectoryInput>,
    shutdown: CancellationToken,
) {
    let mut generations = link.generations();
    loop {
        let mut events = link.events();
        let _ = directory.send(DirectoryInput::Lost).await;
        let seeded = tokio::select! {
            () = shutdown.cancelled() => return,
            _ = generations.changed() => continue,
            seed = cdp::discover_and_seed(&link) => seed,
        };
        let Ok((seed, after_reply)) = seeded else {
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
                        && directory.send(input).await.is_err()
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
        if directory.send(seed).await.is_err() {
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
                        && directory.send(input).await.is_err()
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

    use futures_util::future::BoxFuture;
    use serde_json::json;
    use tokio::sync::{Mutex, Notify};

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
    }

    impl HostStore for Store {
        fn knows<'a>(&'a self, key: &'a SessionKey) -> BoxFuture<'a, Result<bool, StoreError>> {
            Box::pin(async move {
                self.checks.lock().await.push(key.clone());
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
            let hub = CaptureHub::new(Arc::clone(&link), Arc::clone(&presences), shutdown.clone());
            let store = Arc::new(Store::default());
            let mut options =
                HostOptions::new(browser.port(), 8, Arc::clone(&store) as Arc<dyn HostStore>);
            options.idle = idle;
            let (host, jobs) = start(options, link, presences, hub, shutdown)
                .await
                .unwrap();
            browser.wait_for("Browser.getTabs", 1).await;
            Self {
                browser,
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
    async fn startup_restores_taint_and_complete_entries_are_evicted_after_an_hour() {
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
        time::sleep(RETAIN_SETTLED + Duration::from_secs(1)).await;
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
}
