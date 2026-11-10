//! The door on the fake browser, with a real trace store in a temp dir.

use std::error::Error;
use std::future::{self, Future};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_core::{NativeCapabilities, TabId};
use neo_trace::{
    Effect, OperationId, OperationRow, RedactionLimits, Redactor, RunId, SecretSet,
    SecretSetConfig, SecretValue, TraceSink, TraceStore, WriterConfig, spawn_writer,
};
use serde_json::Value;
use tempfile::TempDir;
use tokio::sync::{Barrier, Notify};
use tokio::time::{self, Instant};

use super::test_methods::{Shared, test_methods};
use crate::browser::{BrowserLink, PageRef, TabRegistry};
use crate::dispatch::{
    Call, Caller, Dispatcher, ErrorCode, Held, HeldValues, HeldWatch, OpReply, OperationError,
    Policy, PolicyCheck, RegistryBuilder, RunCtx, RunGuard, RunStart,
};
use crate::test_support::FakeCdp;

/// How many overlapping calls `test.gate` waits for before any of them returns.
pub(super) const GATE_PARTIES: usize = 50;

/// What builtins' `TypedValues` holds for every run, as a test sets it: nothing, until a test
/// holds a value, as a field typed a moment ago is held, or loses them, as a run does past the
/// cap, now or at a given ask.
#[derive(Default)]
pub(super) struct HeldFake {
    values: Mutex<Vec<String>>,
    lost: AtomicBool,
    /// How many times runs asked what they hold.
    asks: AtomicUsize,
    /// The ask that loses what is held ([`HeldFake::lose_at_ask`]); none when 0.
    lost_at: AtomicUsize,
}

/// One run's watch on the fake.
struct HeldFakeWatch(Arc<HeldFake>);

impl HeldFake {
    /// Every run holds `value` from now on.
    pub(super) fn hold(&self, value: &str) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(value.to_owned());
    }

    /// Every run has lost the values held so far from now on: they mask nothing any more, as a
    /// value forgotten past the cap does.
    pub(super) fn lose(&self) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.lost.store(true, Ordering::SeqCst);
    }

    /// Every run loses the values held so far as runs ask what they hold for the `ask`-th time
    /// from now, so a test can put the loss between any two asks of one op.
    pub(super) fn lose_at_ask(&self, ask: usize) {
        self.lost_at
            .store(self.asks.load(Ordering::SeqCst) + ask, Ordering::SeqCst);
    }

    /// How many times runs asked what they hold so far.
    pub(super) fn asks(&self) -> usize {
        self.asks.load(Ordering::SeqCst)
    }
}

impl HeldValues for HeldFake {
    fn watch(self: Arc<Self>, _run: RunId) -> Box<dyn HeldWatch> {
        Box::new(HeldFakeWatch(self))
    }
}

impl HeldWatch for HeldFakeWatch {
    fn held(&self) -> Held {
        let ask = self.0.asks.fetch_add(1, Ordering::SeqCst) + 1;
        if ask == self.0.lost_at.load(Ordering::SeqCst) {
            self.0.lose();
        }
        let values = self
            .0
            .values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let set = (!values.is_empty()).then(|| {
            Arc::new(SecretSet::with_values(
                SecretSetConfig::default(),
                values.into_iter().map(SecretValue::new),
            ))
        });
        if self.0.lost.load(Ordering::SeqCst) {
            Held::Lost(set)
        } else {
            Held::Kept(set)
        }
    }
}

/// Everything a dispatcher test touches. A plain data bag.
pub(super) struct Harness {
    pub(super) fake: Arc<FakeCdp>,
    pub(super) link: Arc<BrowserLink>,
    pub(super) tabs: Arc<TabRegistry>,
    pub(super) store: TraceStore,
    pub(super) sink: TraceSink,
    pub(super) secrets: Arc<SecretSet>,
    /// What every run holds beyond `secrets`.
    pub(super) held: Arc<HeldFake>,
    pub(super) lock: Arc<TabLock>,
    pub(super) seen: Arc<SeenChecks>,
    /// Notified when `test.hang` has sent and is waiting forever.
    pub(super) hung: Arc<Notify>,
    pub(super) dispatcher: Arc<Dispatcher>,
    /// Keeps the store's directory for the test's life.
    pub(super) _dir: TempDir,
}

/// sdk-traces' takeover lock, reduced to two switches: refuse writes, and refuse reads too.
#[derive(Default)]
pub(super) struct TabLock {
    pub(super) writes: AtomicBool,
    pub(super) reads: AtomicBool,
}

/// A rule that refuses nothing and remembers what it was asked.
#[derive(Default)]
pub(super) struct SeenChecks {
    checks: Mutex<Vec<SeenCheck>>,
}

/// One `PolicyCheck`, owned. A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SeenCheck {
    pub(super) method: &'static str,
    pub(super) caller: Caller,
    pub(super) tab: Option<TabId>,
    pub(super) operation: OperationId,
}

impl Harness {
    pub(super) async fn new() -> Result<Self, Box<dyn Error>> {
        let fake = FakeCdp::new();
        let tabs = Arc::new(TabRegistry::new());
        let connect = Arc::clone(&fake);
        let link = BrowserLink::start_with(
            NativeCapabilities::default(),
            Arc::clone(&tabs),
            move || future::ready(connect.connect()),
        );
        // Linked, with target discovery on, so a tab closed by the fake reaches the registry.
        within(Duration::from_secs(5), async {
            while !link.is_ready() || fake.count("Target.setDiscoverTargets") == 0 {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?;
        let dir = tempfile::tempdir()?;
        let store = TraceStore::open(&dir.path().join("trace.db")).await?;
        let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
        let redactor = Redactor::new(Arc::clone(&secrets), RedactionLimits::default());
        let sink = spawn_writer(&store, redactor, WriterConfig::default(), None);
        let lock = Arc::new(TabLock::default());
        let seen = Arc::new(SeenChecks::default());
        let hung = Arc::new(Notify::new());
        let shared = Shared {
            fake: Arc::clone(&fake),
            lock: Arc::clone(&lock),
            gate: Arc::new(Barrier::new(GATE_PARTIES)),
            hung: Arc::clone(&hung),
        };
        let mut registry = RegistryBuilder::default();
        test_methods(&mut registry, &shared)?;
        let policies: Vec<Arc<dyn Policy>> = vec![
            Arc::clone(&lock) as Arc<dyn Policy>,
            Arc::clone(&seen) as Arc<dyn Policy>,
        ];
        let held = Arc::new(HeldFake::default());
        let dispatcher = Arc::new(Dispatcher::new(
            registry.build(),
            policies,
            Arc::clone(&secrets),
            sink.clone(),
            Arc::clone(&held) as Arc<dyn HeldValues>,
            Arc::clone(&link),
            Arc::clone(&tabs),
        ));
        Ok(Self {
            fake,
            link,
            tabs,
            store,
            sink,
            secrets,
            held,
            lock,
            seen,
            hung,
            dispatcher,
            _dir: dir,
        })
    }

    /// A recorded run with no task and no default page yet.
    pub(super) fn run(&self) -> Result<(Arc<RunCtx>, RunGuard), Box<dyn Error>> {
        let view = self.link.view().ok_or("the link is up")?;
        let start = RunStart {
            id: RunId::new(),
            code: "await page.title()".to_owned(),
            request_id: None,
            task_id: None,
            host_data: None,
            view,
            deadline: Instant::now() + Duration::from_secs(300),
            default_page: None,
        };
        Ok(RunCtx::start(&self.dispatcher, start))
    }

    /// A tab the owner opened, as core names it.
    pub(super) async fn page(&self, url: &str) -> Result<PageRef, Box<dyn Error>> {
        let tab = self.fake.open_tab(url, "Example");
        let run = self.link.view().ok_or("the link is up")?;
        let listing = run.view.list_pages().await?;
        let info = listing
            .pages
            .iter()
            .find(|page| page.tab_id == tab)
            .ok_or("core lists the fake's tab")?;
        Ok(PageRef::from(info))
    }

    /// One script call, answered.
    pub(super) async fn submit(
        &self,
        run: &Arc<RunCtx>,
        name: &str,
        input: Value,
    ) -> (OperationId, OpReply) {
        let pending = self
            .dispatcher
            .submit(run, Call::script(name.to_owned(), input, None));
        let operation = pending.operation();
        (operation, pending.wait().await)
    }

    /// The op as the trace stored it, once everything sent so far is committed.
    pub(super) async fn row(&self, operation: OperationId) -> Result<OperationRow, Box<dyn Error>> {
        self.sink.flush().await?;
        let row = self.store.operation(operation).await?;
        row.ok_or_else(|| format!("{operation} is recorded").into())
    }

    /// Waits until core forgot `page`, as it does once `tab_watch` saw the tab close.
    pub(super) async fn until_forgotten(&self, page: &PageRef) -> Result<(), Box<dyn Error>> {
        within(Duration::from_secs(5), async {
            while self.link.cached_page(&page.id).await.is_some() {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
    }
}

impl Policy for TabLock {
    fn check(&self, check: &PolicyCheck<'_>) -> Result<(), OperationError> {
        let held = match check.method.effect {
            Effect::Write => &self.writes,
            Effect::Read => &self.reads,
        };
        if held.load(Ordering::SeqCst) {
            return Err(OperationError::host(
                ErrorCode::OWNER_HAS_CONTROL,
                "the owner has control of this tab",
            ));
        }
        Ok(())
    }
}

impl SeenChecks {
    /// Every check so far, oldest first.
    pub(super) fn all(&self) -> Vec<SeenCheck> {
        self.checks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Policy for SeenChecks {
    fn check(&self, check: &PolicyCheck<'_>) -> Result<(), OperationError> {
        self.checks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SeenCheck {
                method: check.method.name,
                caller: check.caller,
                tab: check.tab,
                operation: check.operation,
            });
        Ok(())
    }
}

/// `work`, or an error once `limit` passes: a deadlock fails the test instead of hanging it.
pub(super) async fn within<T>(
    limit: Duration,
    work: impl Future<Output = T>,
) -> Result<T, Box<dyn Error>> {
    time::timeout(limit, work)
        .await
        .map_err(|error| format!("not within {limit:?}: {error}").into())
}
