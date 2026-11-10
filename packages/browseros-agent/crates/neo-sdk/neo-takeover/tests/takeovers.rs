//! `Takeovers` as the service's routes use them: real locks, displays, journal and fits over the
//! fit tests' fake Chromium and a fake Xvnc, and a real `Privacy` over a private-flag store the
//! test can hold or fail.
//!
//! The trace store reads through sqlx, whose pool does not run under paused time, so these tests
//! use real time. Those that wait out a TTL or the give-back's budget pause the clock once nothing
//! reads the store any more (the lock has settled). They resume it before any fit's plan runs,
//! because a fit resizes the screen and its loop watches it over real TCP.

#[allow(
    dead_code,
    reason = "the fit tests' fake Chromium, of which this binary uses a part"
)]
#[path = "fits/browser.rs"]
mod browser;

use browser::{Call, FakeBrowser, SWEEP_READ};
use browseros_core::viewport::Viewports;
use browseros_core::{BrowserSession, BrowserSessionHooks, TabId};
use bytes::Bytes;
use futures_util::future::BoxFuture;
use neo_takeover::testing::{FakeXvnc, NoStamps};
use neo_takeover::{
    Admitted, BrowserTabs, DisplayEnd, DisplayRequest, Displays, FitState, Fits, Follow, Front,
    FrontError, Journal, LockEnd, LockTarget, Locks, OwnerHasControl, OwnerHasScreen, PartError,
    Reads, ReleaseReason, ScreenHolder, ScreenSize, Takeover, TakeoverRequest, TakeoverState,
    TakeoverTtl, Takeovers,
};
use neo_trace::{
    Effect, EpochMillis, OperationId, OperationKind, OperationStarted, Privacy, PrivacyRecord,
    PrivacyStore, PrivateReason, RedactionLimits, Redactor, RunId, RunStarted, SecretSet,
    SecretSetConfig, TaskId, TraceError, TraceSink, TraceStore, WriterConfig, spawn_writer,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type TestResult = Result<(), BoxError>;

const HOME: ScreenSize = ScreenSize {
    width: 1600,
    height: 1000,
};
const PHONE: ScreenSize = ScreenSize {
    width: 390,
    height: 844,
};

/// The tab every test takes over.
const TAB: TabId = TabId(3);

/// How long a test waits for something that should happen.
const WITHIN: Duration = Duration::from_secs(5);

/// How long a test waits before it decides something did not happen. Longer than follow's 1 s
/// tick, so a follower that was not paused would have acted by then.
const QUIET: Duration = Duration::from_millis(1500);

/// The version, security type 1, and a ClientInit that asks for exclusive access (0).
const HANDSHAKE: &[u8] = b"RFB 003.008\n\x01\x00";
/// Key `a` down, as a type-4 KeyEvent.
const KEY_A: [u8; 8] = [4, 1, 0, 0, 0, 0, 0, 0x61];

/// What the owner left in a password field.
const TYPED: &str = "OWNER_TYPED_7f3a";

/// The methods a fit's restore sends to the browser.
const RESTORE: [&str; 4] = [
    "Input.dispatchKeyEvent",
    "Browser.setWindowBounds",
    "Emulation.clearDeviceMetricsOverride",
    "Emulation.setDeviceMetricsOverride",
];

/// Private flags as the trace writer commits them, kept in memory. A test can hold commits
/// until it lets them through, or fail them.
struct Flags {
    committed: Mutex<Vec<TabId>>,
    attempts: AtomicUsize,
    fail: AtomicBool,
    /// While true, every commit waits.
    held: watch::Sender<bool>,
}

impl Flags {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            committed: Mutex::default(),
            attempts: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
            held: watch::channel(false).0,
        })
    }

    fn committed(&self) -> Vec<TabId> {
        self.committed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PrivacyStore for Flags {
    fn persist_private(
        &self,
        tab: TabId,
        _record: PrivacyRecord,
    ) -> BoxFuture<'_, Result<(), TraceError>> {
        Box::pin(async move {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            let mut held = self.held.subscribe();
            held.wait_for(|held| !*held)
                .await
                .map_err(|_| TraceError::Closed)?;
            if self.fail.load(Ordering::SeqCst) {
                return Err(TraceError::Closed);
            }
            self.committed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(tab);
            Ok(())
        })
    }

    fn forget_private(&self, _tab: TabId) -> BoxFuture<'_, Result<(), TraceError>> {
        Box::pin(async { Ok(()) })
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    xvnc: Arc<FakeXvnc>,
    browser: Arc<FakeBrowser>,
    flags: Arc<Flags>,
    privacy: Arc<Privacy>,
    sink: TraceSink,
    locks: Arc<Locks>,
    fits: Arc<Fits>,
    displays: Arc<Displays>,
    takeovers: Arc<Takeovers>,
}

/// The parts as `Service::start` builds them, with `tabs` open.
async fn harness(tabs: &[i64]) -> Result<Harness, BoxError> {
    let dir = tempfile::tempdir()?;
    let store = TraceStore::open(&dir.path().join("trace.db")).await?;
    let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
    let sink = spawn_writer(
        &store,
        Redactor::new(Arc::clone(&secrets), RedactionLimits::default()),
        WriterConfig::default(),
        None,
    );
    let flags = Flags::new();
    let privacy = Privacy::new(
        Arc::clone(&secrets),
        Arc::clone(&flags) as Arc<dyn PrivacyStore>,
        HashMap::new(),
    );
    let xvnc = Arc::new(FakeXvnc::start(HOME).await?);
    let browser = FakeBrowser::new(Arc::clone(&xvnc), tabs);
    let tabs = BrowserTabs::new(
        BrowserSession::new(browser.clone(), BrowserSessionHooks::default()),
        Arc::new(Viewports::default()),
    );
    let locks = Locks::new(store.clone(), sink.clone());
    let journal = Journal::new(tabs.clone(), Arc::clone(&secrets));
    let fits = Fits::new(tabs.clone(), xvnc.xvnc(HOME), secrets);
    let displays = Displays::new(
        xvnc.addr(),
        tabs.clone(),
        Arc::clone(&privacy),
        Arc::new(NoStamps),
        journal.clone(),
        Arc::clone(&locks),
        sink.clone(),
        store,
    );
    let takeovers = Takeovers::new(
        Arc::clone(&privacy),
        Arc::clone(&locks),
        Arc::clone(&fits),
        Arc::clone(&displays),
        journal,
        tabs,
    );
    Ok(Harness {
        _dir: dir,
        xvnc,
        browser,
        flags,
        privacy,
        sink,
        locks,
        fits,
        displays,
        takeovers,
    })
}

/// A takeover of [`TAB`] for the longest TTL.
fn request(viewer: Option<ScreenSize>) -> Result<TakeoverRequest, &'static str> {
    Ok(TakeoverRequest {
        tab: TAB,
        viewer,
        reads: Reads::Block,
        ttl: TakeoverTtl::new(TakeoverTtl::MAX).ok_or("the longest TTL is valid")?,
    })
}

/// A takeover of [`TAB`] for the shortest TTL, 60 s, whose release is due 50 s after its lock.
fn shortest(viewer: Option<ScreenSize>) -> Result<TakeoverRequest, &'static str> {
    Ok(TakeoverRequest {
        ttl: TakeoverTtl::new(TakeoverTtl::MIN).ok_or("the shortest TTL is valid")?,
        ..request(viewer)?
    })
}

impl Harness {
    fn front(&self) -> Front {
        self.displays.front()
    }

    /// Each tab the browser brought to the front, in order.
    fn activated(&self) -> Vec<i64> {
        self.browser
            .calls_of("Browser.activateTab")
            .iter()
            .filter_map(|call| call.params["tabId"].as_i64())
            .collect()
    }

    /// Records an operation of task `task` on `tab`, as the dispatcher would.
    fn operation(&self, task: &TaskId, tab: i64) {
        let run = RunId::new();
        self.sink.run_started(RunStarted {
            id: run,
            code: "await page.click('#next')".into(),
            request_id: None,
            task_id: Some(task.clone()),
            host_data: None,
            also_mask: None,
            at: EpochMillis::now(),
        });
        self.sink.op_started(OperationStarted {
            id: OperationId::new(run, 1),
            parent: None,
            name: "locator.click".into(),
            kind: OperationKind::Builtin,
            effect: Effect::Write,
            tab: Some(TabId(tab)),
            args: json!({ "selector": "#next" }),
            policy: Arc::default(),
            also_mask: None,
            at: EpochMillis::now(),
        });
    }

    /// Whether a view display is admitted now; one that is goes away at once.
    async fn views_admitted(&self) -> Result<bool, BoxError> {
        match self
            .displays
            .admit(DisplayRequest::View { follow: None })
            .await
        {
            Ok(_) => Ok(true),
            Err(PartError::DisplayPaused) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// The owner's control display, bound to `takeover`, with a key forwarded through it.
    async fn owner_typing(&self, takeover: &Takeover) -> Result<Viewer, BoxError> {
        let binding = self.takeovers.binding(takeover.id)?;
        let control = serve(
            self.displays
                .admit(DisplayRequest::Control {
                    binding: Some(binding),
                })
                .await?,
        );
        control.send(HANDSHAKE);
        control.send(&KEY_A);
        eventually(|| {
            (0..self.xvnc.connections())
                .any(|connection| self.xvnc.received(connection).contains(&KEY_A.to_vec()))
        })
        .await?;
        Ok(control)
    }

    /// The calls since `from` that are of `method`.
    fn calls_since(&self, from: usize, method: &str) -> Vec<Call> {
        self.browser
            .calls()
            .into_iter()
            .skip(from)
            .filter(|call| call.method == method)
            .collect()
    }

    /// From now on, each call keeps whether the agent's write on the tab was refused then.
    fn probe_writes(&self) {
        let locks = Arc::clone(&self.locks);
        self.browser
            .probe_each_call(move || locks.check(LockTarget::Tab(TAB), Effect::Write).is_err());
    }

    /// Checks what a start leaves when it was not done by the time its TTL release was due. It
    /// answered `TimedOut`. Its takeover was announced once, as released (`Expired`), never as
    /// active. The tab was swept and the journal disarmed while the agent's writes were refused,
    /// and the give-back lifted the lock before the lock's own TTL could.
    async fn given_back_in_time(
        &self,
        started: Result<Takeover, PartError>,
        changes: &mut broadcast::Receiver<Takeover>,
    ) -> TestResult {
        assert!(
            matches!(started, Err(PartError::TimedOut(_))),
            "a start not done when its release was due answered {started:?}"
        );
        let announced = std::iter::from_fn(|| changes.try_recv().ok()).collect::<Vec<_>>();
        assert!(
            announced
                .iter()
                .all(|takeover| takeover.state != TakeoverState::Active),
            "never active: {announced:#?}"
        );
        let [released] = announced.as_slice() else {
            return Err(format!("one announcement, the release: {announced:#?}").into());
        };
        assert_eq!(released_for(released), Some(ReleaseReason::Expired));
        assert_eq!(self.takeovers.get(released.id).as_ref(), Some(released));

        let calls = self.browser.calls();
        let sweeps = calls
            .iter()
            .filter(|call| is_sweep_read(call))
            .collect::<Vec<_>>();
        assert!(
            !sweeps.is_empty() && sweeps.iter().all(|call| call.probed == Some(true)),
            "the tab was swept while the agent's writes were refused: {sweeps:#?}"
        );
        assert!(
            calls.iter().all(|call| call.probed != Some(false)),
            "and so they were at every call: {calls:#?}"
        );
        assert_eq!(
            self.calls_since(0, "Page.removeScriptToEvaluateOnNewDocument")
                .len(),
            1,
            "the journal was disarmed"
        );
        let lock = self.locks.get(released.lock).ok_or("the lock")?;
        assert_eq!(
            lock.ended.map(|ended| ended.cause),
            Some(LockEnd::Released),
            "the give-back lifted the lock before its own TTL could"
        );
        assert_eq!(
            self.locks.check(LockTarget::Tab(TAB), Effect::Write),
            Ok(()),
            "the agent writes again once the lock lifted"
        );
        assert!(self.views_admitted().await?, "spectators are back");
        let mut result = format!("the field held {TYPED}");
        assert!(
            self.privacy.secrets().mask_str(&mut result),
            "what the owner left in the field is a known secret"
        );
        Ok(())
    }
}

/// A viewer's end of a display: what it sends, and how the display ended.
struct Viewer {
    to_display: mpsc::UnboundedSender<Bytes>,
    serving: JoinHandle<DisplayEnd>,
}

fn serve(admitted: Admitted) -> Viewer {
    let (to_display, mut from_viewer) = mpsc::unbounded_channel::<Bytes>();
    let from_viewer = futures_util::stream::poll_fn(move |cx| from_viewer.poll_recv(cx));
    let to_viewer = futures_util::sink::drain::<Bytes>();
    Viewer {
        to_display,
        serving: tokio::spawn(admitted.serve(from_viewer, to_viewer)),
    }
}

impl Viewer {
    fn send(&self, bytes: &[u8]) {
        let _ = self.to_display.send(Bytes::copy_from_slice(bytes));
    }

    async fn ended(self) -> Result<DisplayEnd, BoxError> {
        Ok(timeout(WITHIN, self.serving).await??)
    }
}

/// Waits until `done` holds.
async fn eventually(done: impl Fn() -> bool) -> TestResult {
    timeout(WITHIN, async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    Ok(())
}

fn is_sweep_read(call: &Call) -> bool {
    call.method == "Runtime.evaluate" && call.params["expression"] == SWEEP_READ
}

fn released_for(takeover: &Takeover) -> Option<ReleaseReason> {
    match takeover.state {
        TakeoverState::Active => None,
        TakeoverState::Released { reason, .. } => Some(reason),
    }
}

#[tokio::test]
async fn the_tab_is_private_on_disk_before_the_agent_is_locked_out() -> TestResult {
    let h = harness(&[3, 4]).await?;
    let request = request(Some(PHONE))?;
    h.flags.held.send_replace(true);
    let starting = tokio::spawn({
        let takeovers = Arc::clone(&h.takeovers);
        async move { takeovers.start(request).await }
    });
    eventually(|| h.flags.attempts.load(Ordering::SeqCst) == 1).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        h.privacy.is_private(TAB).is_some(),
        "captures are refused from the moment the flag is set"
    );
    assert!(
        !h.locks.is_locked(TAB),
        "no lock while the flag is not on disk"
    );
    assert!(h.xvnc.resizes().is_empty(), "and no fit");

    h.flags.held.send_replace(false);
    let takeover = timeout(WITHIN, starting).await???;
    assert_eq!(h.flags.committed(), [TAB]);
    assert_eq!(
        h.privacy.is_private(TAB).map(|record| record.reason),
        Some(PrivateReason::Takeover)
    );
    assert_eq!(takeover.state, TakeoverState::Active);
    assert_eq!(
        h.locks.check(LockTarget::Tab(TAB), Effect::Read),
        Err(OwnerHasControl {
            lock: takeover.lock
        }),
        "the agent cannot even read the tab"
    );
    assert_eq!(
        takeover.expires_at.millis() - takeover.created_at.millis(),
        1_200_000
    );
    assert!(!h.views_admitted().await?, "spectators are held off");
    assert_eq!(
        h.calls_since(0, "Page.addScriptToEvaluateOnNewDocument")
            .iter()
            .map(|call| call.tab)
            .collect::<Vec<_>>(),
        [Some(3)],
        "the journal is armed on the tab, and only there"
    );
    let fit = h.fits.view(takeover.fit.ok_or("a fit")?).ok_or("the fit")?;
    assert_eq!((fit.tab, fit.screen), (TAB, PHONE));
    assert_eq!(h.xvnc.size(), PHONE);
    assert_eq!(h.browser.window(), "fullscreen");
    Ok(())
}

#[tokio::test]
async fn a_flag_that_is_not_committed_refuses_the_start_with_nothing_taken() -> TestResult {
    let h = harness(&[3]).await?;
    h.flags.fail.store(true, Ordering::SeqCst);
    let refused = h.takeovers.start(request(Some(PHONE))?).await;
    assert!(matches!(refused, Err(PartError::Privacy(_))), "{refused:?}");
    assert!(!h.locks.is_locked(TAB));
    assert!(h.views_admitted().await?);
    assert!(
        h.calls_since(0, "Page.addScriptToEvaluateOnNewDocument")
            .is_empty()
    );
    assert!(h.xvnc.resizes().is_empty());

    h.flags.fail.store(false, Ordering::SeqCst);
    let started = h.takeovers.start(request(None)?).await?;
    assert!(h.locks.is_locked(TAB), "a start after the store recovers");
    assert_eq!(started.fit, None);
    Ok(())
}

#[tokio::test]
async fn a_failed_fit_gives_back_what_the_start_took() -> TestResult {
    let h = harness(&[3]).await?;
    h.xvnc.refuse_next(3);
    let refused = h.takeovers.start(request(Some(PHONE))?).await;
    assert!(matches!(refused, Err(PartError::Screen(_))), "{refused:?}");
    assert!(!h.locks.is_locked(TAB), "the lock is lifted");
    assert!(h.views_admitted().await?, "spectators are let back");
    assert_eq!(
        h.calls_since(0, "Page.removeScriptToEvaluateOnNewDocument")
            .len(),
        1,
        "the journal is disarmed"
    );
    assert!(
        h.privacy.is_private(TAB).is_some(),
        "the tab stays private, which only over-protects it"
    );
    assert_eq!(h.xvnc.size(), HOME);
    Ok(())
}

/// The give-back's order, read from the browser's own log: the display's close sweep of every
/// open tab, its journal's disarm, the tab's sweep, the takeover's journal, then the fit's
/// restore. Sweep reads take a moment, so a step that did not wait for the one before it would
/// interleave with it. The probe reads the lock at every one of those calls.
#[tokio::test]
async fn release_gives_back_in_order_and_lifts_the_lock_last() -> TestResult {
    let h = harness(&[3, 4]).await?;
    let takeover = h.takeovers.start(request(Some(PHONE))?).await?;
    let control = h.owner_typing(&takeover).await?;
    h.browser.type_into_fields(&[TYPED]);
    h.browser.slow_sweep_reads(Duration::from_millis(100));
    let locks = Arc::clone(&h.locks);
    h.browser.probe_each_call(move || locks.is_locked(TAB));
    let from = h.browser.calls().len();
    let resizes = h.xvnc.resizes().len();

    let released = h
        .takeovers
        .release(takeover.id, ReleaseReason::Done)
        .await?;
    assert_eq!(released_for(&released), Some(ReleaseReason::Done));
    assert_eq!(control.ended().await?, DisplayEnd::Released);

    let calls = h.browser.calls().split_off(from);
    let at = |found: &dyn Fn(&Call) -> bool| -> Vec<usize> {
        calls
            .iter()
            .enumerate()
            .filter(|(_, call)| found(call))
            .map(|(at, _)| at)
            .collect()
    };
    let sweeps = at(&is_sweep_read);
    let mut swept = sweeps
        .iter()
        .filter_map(|&at| calls.get(at).and_then(|call| call.tab))
        .collect::<Vec<_>>();
    assert_eq!(
        swept.pop(),
        Some(3),
        "the takeover's own sweep comes last: {swept:?}"
    );
    swept.sort_unstable();
    assert_eq!(swept, [3, 4], "the display's close sweep reads every tab");
    let removed = |script: &'static str| {
        move |call: &Call| {
            call.method == "Page.removeScriptToEvaluateOnNewDocument"
                && call.params["identifier"] == script
        }
    };
    // The takeover armed its tab at start (script 1); the display armed both tabs after.
    let displays_journal = [at(&removed("script-2")), at(&removed("script-3"))].concat();
    let takeovers_journal = at(&removed("script-1"));
    let restore = at(&|call: &Call| RESTORE.contains(&call.method.as_str()));
    let (Some(&own_sweep), Some(&own_disarm), Some(&first_restore)) =
        (sweeps.last(), takeovers_journal.first(), restore.first())
    else {
        return Err(format!("a step is missing from the give-back: {calls:#?}").into());
    };
    assert_eq!(displays_journal.len(), 2, "{calls:#?}");
    assert!(
        displays_journal.iter().all(|&at| at < own_sweep),
        "the display ended, its journal disarmed, before the tab's sweep"
    );
    assert!(own_sweep < own_disarm, "the tab's sweep, then its journal");
    assert!(own_disarm < first_restore, "the journal, then the fit");
    assert_eq!(
        calls.get(own_disarm).map(|call| call.resizes),
        Some(resizes),
        "the screen goes home only after the journal is disarmed"
    );
    assert!(
        calls.iter().all(|call| call.probed == Some(true)),
        "the agent was locked out through every step: {calls:#?}"
    );

    assert!(!h.locks.is_locked(TAB), "then the lock lifted");
    assert!(h.views_admitted().await?, "and spectators are back");
    assert_eq!(h.xvnc.size(), HOME);
    assert_eq!(h.browser.window(), "maximized");
    let mut result = format!("the field held {TYPED}");
    assert!(
        h.privacy.secrets().mask_str(&mut result),
        "what the owner left in the field is a known secret"
    );
    Ok(())
}

#[tokio::test]
async fn views_and_the_lock_stay_until_the_give_back_is_done() -> TestResult {
    let h = harness(&[3]).await?;
    let takeover = h.takeovers.start(request(None)?).await?;
    h.browser.hold_sweep_reads();
    let releasing = tokio::spawn({
        let takeovers = Arc::clone(&h.takeovers);
        async move { takeovers.release(takeover.id, ReleaseReason::Done).await }
    });
    eventually(|| h.browser.held_sweep_reads() > 0).await?;
    assert!(!h.views_admitted().await?, "spectators are still held off");
    assert!(
        h.locks.check(LockTarget::Tab(TAB), Effect::Write).is_err(),
        "the agent is still locked out"
    );
    assert_eq!(
        h.takeovers.get(takeover.id).map(|takeover| takeover.state),
        Some(TakeoverState::Active),
        "a release that runs reads as active"
    );
    assert!(
        matches!(
            h.takeovers.binding(takeover.id),
            Err(PartError::TakeoverReleased)
        ),
        "but no display can bind to it any more"
    );

    h.browser.let_sweep_reads_go();
    let released = timeout(WITHIN, releasing).await???;
    assert_eq!(released_for(&released), Some(ReleaseReason::Done));
    assert!(!h.locks.is_locked(TAB));
    assert!(h.views_admitted().await?);
    Ok(())
}

/// The service's routes are cancelled when a host hangs up. A start cut short after its lock
/// would leave a lock nothing can lift, so it goes on, and a retry finds the takeover it made.
#[tokio::test]
async fn a_start_whose_caller_hangs_up_still_finishes() -> TestResult {
    let h = harness(&[3]).await?;
    h.flags.held.send_replace(true);
    let hung_up = timeout(
        Duration::from_millis(100),
        h.takeovers.start(request(None)?),
    )
    .await;
    assert!(hung_up.is_err(), "the caller stopped waiting");
    h.flags.held.send_replace(false);
    eventually(|| h.locks.is_locked(TAB)).await?;

    let retried = h.takeovers.start(request(None)?).await?;
    assert_eq!(h.flags.attempts.load(Ordering::SeqCst), 1, "one start ran");
    assert_eq!(
        h.locks.check(LockTarget::Tab(TAB), Effect::Write),
        Err(OwnerHasControl { lock: retried.lock }),
        "the retry found the takeover the first start made"
    );
    assert!(!h.views_admitted().await?);
    let released = h.takeovers.release(retried.id, ReleaseReason::Done).await?;
    assert_eq!(released_for(&released), Some(ReleaseReason::Done));
    assert!(!h.locks.is_locked(TAB), "the host can still give it back");
    Ok(())
}

#[tokio::test]
async fn a_release_whose_caller_hangs_up_still_lifts_the_lock() -> TestResult {
    let h = harness(&[3]).await?;
    let takeover = h.takeovers.start(request(None)?).await?;
    h.browser.hold_sweep_reads();
    let hung_up = timeout(
        Duration::from_millis(100),
        h.takeovers.release(takeover.id, ReleaseReason::Done),
    )
    .await;
    assert!(hung_up.is_err(), "the caller stopped waiting");
    h.browser.let_sweep_reads_go();
    eventually(|| !h.locks.is_locked(TAB)).await?;
    let released = h.takeovers.get(takeover.id).ok_or("the takeover")?;
    assert_eq!(released_for(&released), Some(ReleaseReason::Done));
    assert!(h.views_admitted().await?);
    Ok(())
}

#[tokio::test]
async fn a_step_that_fails_does_not_keep_the_lock() -> TestResult {
    let h = harness(&[3]).await?;
    let takeover = h.takeovers.start(request(None)?).await?;
    // The sweep's read never answers, so the sweep gives up on the frame and fails.
    h.browser.hold_sweep_reads();
    let released = h
        .takeovers
        .release(takeover.id, ReleaseReason::Idle)
        .await?;
    assert_eq!(released_for(&released), Some(ReleaseReason::Idle));
    assert!(!h.locks.is_locked(TAB));
    assert!(h.views_admitted().await?);
    assert_eq!(
        h.calls_since(0, "Page.removeScriptToEvaluateOnNewDocument")
            .len(),
        1,
        "the journal was still disarmed"
    );
    Ok(())
}

/// The owner's display arms the journal on six tabs, and none of its scripts can be removed: its
/// end takes 2 s a tab, past the give-back's 10 s. The release stops waiting at 10 s and unlocks,
/// and the steps it no longer waits for still run: the tab's sweep registers what it finds.
#[tokio::test]
async fn a_give_back_that_runs_out_of_its_budget_still_lifts_the_lock() -> TestResult {
    let h = harness(&[3, 4, 5, 6, 7, 8]).await?;
    let takeover = h.takeovers.start(request(None)?).await?;
    let binding = h.takeovers.binding(takeover.id)?;
    let control = serve(
        h.displays
            .admit(DisplayRequest::Control {
                binding: Some(binding),
            })
            .await?,
    );
    h.browser.type_into_fields(&[TYPED]);
    h.browser.hang("Page.removeScriptToEvaluateOnNewDocument");

    tokio::time::pause();
    let started = Instant::now();
    let released = h
        .takeovers
        .release(takeover.id, ReleaseReason::Done)
        .await?;
    let took = started.elapsed();
    let still_ending = !control.serving.is_finished();
    tokio::time::resume();

    assert_eq!(released_for(&released), Some(ReleaseReason::Done));
    assert!(
        took >= Duration::from_secs(10) && took < Duration::from_secs(11),
        "{took:?}"
    );
    assert!(still_ending, "the display's end goes on in the background");
    assert!(!h.locks.is_locked(TAB));
    eventually(|| {
        let mut result = format!("the field held {TYPED}");
        h.privacy.secrets().mask_str(&mut result)
    })
    .await?;
    Ok(())
}

#[tokio::test]
async fn the_ttl_release_comes_10_s_before_the_lock_ends() -> TestResult {
    let h = harness(&[3]).await?;
    let ttl = TakeoverTtl::new(Duration::from_secs(60)).ok_or("a valid TTL")?;
    let before = Instant::now();
    let takeover = h
        .takeovers
        .start(TakeoverRequest {
            ttl,
            ..request(None)?
        })
        .await?;
    let after = Instant::now();

    tokio::time::pause();
    let early = (before + Duration::from_secs(50) - Duration::from_millis(1))
        .saturating_duration_since(Instant::now());
    tokio::time::advance(early).await;
    let still = h.takeovers.get(takeover.id).map(|takeover| takeover.state);
    let released = h
        .takeovers
        .wait(takeover.id, after + Duration::from_secs(55))
        .await
        .ok_or("the takeover")?;
    let released_at = Instant::now();
    tokio::time::resume();

    assert_eq!(
        still,
        Some(TakeoverState::Active),
        "not before deadline - 10 s"
    );
    assert_eq!(released_for(&released), Some(ReleaseReason::Expired));
    assert!(
        released_at >= before + Duration::from_secs(50)
            && released_at <= after + Duration::from_secs(50),
        "released {:?} after the start began",
        released_at - before
    );
    let lock = h.locks.get(takeover.lock).ok_or("the lock")?;
    assert_eq!(
        lock.ended.map(|ended| ended.cause),
        Some(LockEnd::Released),
        "the release lifted the lock before its own TTL could"
    );
    Ok(())
}

/// A page's frames do not answer while the journal arms them. Each waits out its 2 s frame
/// budget, so arming 37 frames would outlast the 50 s a 60 s takeover has before its release is
/// due. The start stops waiting then and gives back with the agent still locked out. The frames
/// answer again just after, so the give-back, the late arming's disarm included, ends in time.
#[tokio::test]
async fn a_start_whose_frames_stall_is_given_back_before_its_lock_ends() -> TestResult {
    let h = harness(&[3]).await?;
    h.browser.add_child_frames(36);
    h.browser.hold("Page.createIsolatedWorld");
    h.browser.type_into_fields(&[TYPED]);
    let mut changes = h.takeovers.subscribe();
    let request = shortest(None)?;
    let starting = tokio::spawn({
        let takeovers = Arc::clone(&h.takeovers);
        async move { takeovers.start(request).await }
    });
    // Arming begins after the lock has settled, the last read of the store.
    eventually(|| h.browser.held("Page.createIsolatedWorld") > 0).await?;
    h.probe_writes();

    tokio::time::pause();
    // The lock was taken before now, so its release fell due within these 50 s.
    tokio::time::sleep(Duration::from_secs(50)).await;
    h.browser.let_go("Page.createIsolatedWorld");
    let started = timeout(WITHIN, starting).await??;
    tokio::time::resume();

    h.given_back_in_time(started, &mut changes).await
}

/// A fit waits on the browser past the takeover's release. The start stops waiting then and gives
/// back with the agent still locked out. The fit lands during the give-back and is restored
/// before the lock lifts.
#[tokio::test]
async fn a_start_whose_fit_stalls_is_given_back_before_its_lock_ends() -> TestResult {
    let h = harness(&[3]).await?;
    h.browser.hold("Browser.getWindowForTarget");
    h.browser.type_into_fields(&[TYPED]);
    let mut changes = h.takeovers.subscribe();
    let request = shortest(Some(PHONE))?;
    let starting = tokio::spawn({
        let takeovers = Arc::clone(&h.takeovers);
        async move { takeovers.start(request).await }
    });
    eventually(|| h.browser.held("Browser.getWindowForTarget") > 0).await?;
    h.probe_writes();

    tokio::time::pause();
    tokio::time::sleep(Duration::from_secs(50)).await;
    // The fit's plan resizes the screen over real TCP.
    tokio::time::resume();
    h.browser.let_go("Browser.getWindowForTarget");
    let started = timeout(WITHIN, starting).await??;

    h.given_back_in_time(started, &mut changes).await?;
    let fitted = h
        .browser
        .calls()
        .into_iter()
        .filter(|call| RESTORE.contains(&call.method.as_str()))
        .collect::<Vec<_>>();
    assert!(
        !fitted.is_empty() && fitted.iter().all(|call| call.probed == Some(true)),
        "the fit landed and was restored while the agent's writes were refused: {fitted:#?}"
    );
    let resizes = h.xvnc.resizes();
    assert_eq!(resizes.len(), 2, "{resizes:?}");
    assert!(
        resizes
            .first()
            .is_some_and(|resize| resize.starts_with(&PHONE.to_string())),
        "the fit landed during the give-back: {resizes:?}"
    );
    assert_eq!(h.xvnc.size(), HOME, "and the give-back put the screen back");
    assert_eq!(h.browser.window(), "maximized");
    Ok(())
}

#[tokio::test]
async fn a_closed_tab_releases_its_takeover_without_the_steps_that_need_it() -> TestResult {
    let h = harness(&[3, 4]).await?;
    let takeover = h.takeovers.start(request(Some(PHONE))?).await?;
    h.browser.type_into_fields(&[TYPED]);
    h.browser.close_tab(3);
    let from = h.browser.calls().len();
    h.takeovers.tab_closed(TAB).await;

    let released = h.takeovers.get(takeover.id).ok_or("the takeover")?;
    assert_eq!(released_for(&released), Some(ReleaseReason::TabClosed));
    let calls = h.browser.calls().split_off(from);
    assert!(!calls.iter().any(is_sweep_read), "no sweep of a closed tab");
    assert!(
        !calls
            .iter()
            .any(|call| RESTORE.contains(&call.method.as_str())),
        "no restore"
    );
    assert!(!h.locks.is_locked(TAB));
    assert!(h.views_admitted().await?);
    let fit = takeover.fit.ok_or("a fit")?;
    assert_eq!(
        h.fits.view(fit).map(|fit| fit.state),
        Some(FitState::Active),
        "the fit is left to the fits' own tab_closed"
    );
    h.fits.tab_closed(TAB).await;
    assert_eq!(h.xvnc.size(), HOME);
    Ok(())
}

#[tokio::test]
async fn one_takeover_per_tab_and_one_give_back_per_takeover() -> TestResult {
    let h = harness(&[3]).await?;
    let (first, second) = tokio::join!(
        h.takeovers.start(request(None)?),
        h.takeovers.start(request(Some(PHONE))?)
    );
    let (first, second) = (first?, second?);
    assert_eq!(first, second, "the second start found the first");
    assert_eq!(
        h.calls_since(0, "Page.addScriptToEvaluateOnNewDocument")
            .len(),
        1,
        "one journal"
    );

    let from = h.browser.calls().len();
    let (done, idle) = tokio::join!(
        h.takeovers.release(first.id, ReleaseReason::Done),
        h.takeovers.release(first.id, ReleaseReason::Idle)
    );
    let (done, idle) = (done?, idle?);
    assert_eq!(done, idle, "both callers get the one release's answer");
    assert!(released_for(&done).is_some());
    assert_eq!(
        h.browser
            .calls()
            .split_off(from)
            .iter()
            .filter(|call| is_sweep_read(call))
            .count(),
        1,
        "the steps ran once"
    );
    let again = h.takeovers.release(first.id, ReleaseReason::Host).await?;
    assert_eq!(again, done, "the first reason stands");

    let next = h.takeovers.start(request(None)?).await?;
    assert_ne!(next.id, first.id, "a released takeover is not returned");
    Ok(())
}

#[tokio::test]
async fn wait_wakes_at_release_and_returns_a_released_one_at_once() -> TestResult {
    let h = harness(&[3]).await?;
    let takeover = h.takeovers.start(request(None)?).await?;
    let waiting = tokio::spawn({
        let takeovers = Arc::clone(&h.takeovers);
        async move {
            takeovers
                .wait(takeover.id, Instant::now() + Duration::from_secs(30))
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!waiting.is_finished(), "an active takeover holds the wait");

    let released = h
        .takeovers
        .release(takeover.id, ReleaseReason::Host)
        .await?;
    let woke = timeout(WITHIN, waiting).await??;
    assert_eq!(woke, Some(released.clone()));

    let asked = Instant::now();
    let again = h
        .takeovers
        .wait(takeover.id, asked + Duration::from_secs(30))
        .await;
    assert_eq!(again, Some(released));
    assert!(asked.elapsed() < Duration::from_secs(1));

    let idle = h
        .takeovers
        .wait(
            neo_takeover::TakeoverId::new(),
            Instant::now() + Duration::from_secs(30),
        )
        .await;
    assert_eq!(idle, None, "no takeover has the id");
    Ok(())
}

#[tokio::test]
async fn each_state_change_is_announced() -> TestResult {
    let h = harness(&[3]).await?;
    let mut changes = h.takeovers.subscribe();
    let takeover = h.takeovers.start(request(None)?).await?;
    let released = h
        .takeovers
        .release(takeover.id, ReleaseReason::Done)
        .await?;
    assert_eq!(timeout(WITHIN, changes.recv()).await??, takeover);
    assert_eq!(timeout(WITHIN, changes.recv()).await??, released);
    assert!(
        matches!(
            h.takeovers.binding(takeover.id),
            Err(PartError::TakeoverReleased)
        ),
        "a released takeover binds no display"
    );
    assert!(matches!(
        h.takeovers.binding(neo_takeover::TakeoverId::new()),
        Err(PartError::NotFound)
    ));
    Ok(())
}

/// During a takeover of 3 with a phone viewer, the agent's bringToFront(4) is refused for the
/// takeover: its commit never runs, nothing is sent, and 3 stays in front. After the release the
/// same move goes through.
#[tokio::test]
async fn a_takeovers_tab_stays_in_front_when_the_agent_brings_another_forward() -> TestResult {
    let h = harness(&[3, 4]).await?;
    let takeover = h.takeovers.start(request(Some(PHONE))?).await?;
    assert_eq!(
        h.browser.active(),
        Some(3),
        "the takeover's fit brought 3 forward"
    );
    let from = h.browser.calls().len();
    let commits = AtomicUsize::new(0);
    let refused = h
        .front()
        .bring_to_front(TabId(4), None, || {
            commits.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ()>(())
        })
        .await;
    assert!(
        matches!(
            refused,
            Err(FrontError::OwnerHasScreen(OwnerHasScreen {
                by: ScreenHolder::Takeover
            }))
        ),
        "{refused:?}"
    );
    assert_eq!(commits.load(Ordering::SeqCst), 0, "commit never ran");
    assert!(
        h.calls_since(from, "Browser.activateTab").is_empty(),
        "nothing was sent"
    );
    assert_eq!(h.browser.active(), Some(3), "3 stays in front");

    h.takeovers
        .release(takeover.id, ReleaseReason::Done)
        .await?;
    h.front()
        .bring_to_front(TabId(4), None, || Ok::<_, ()>(()))
        .await?;
    assert_eq!(
        h.browser.active(),
        Some(4),
        "the screen is the agent's again"
    );
    Ok(())
}

/// R4: the agent's move is past its last check and sent, its answer held, when a takeover of 3
/// with a viewer starts. The takeover's fit asks for the front turn and waits for it, so its
/// activation is not sent beside the agent's: the agent's lands first, and 3 ends in front.
#[tokio::test]
async fn a_move_in_flight_lands_before_the_takeovers_fit_brings_its_tab_forward() -> TestResult {
    let h = harness(&[3, 4]).await?;
    h.browser.hold("Browser.activateTab");
    let moving = tokio::spawn({
        let front = h.front();
        async move {
            front
                .bring_to_front(TabId(4), None, || Ok::<_, ()>(()))
                .await
        }
    });
    eventually(|| h.browser.held("Browser.activateTab") == 1).await?;
    let starting = tokio::spawn({
        let takeovers = Arc::clone(&h.takeovers);
        let request = request(Some(PHONE))?;
        async move { takeovers.start(request).await }
    });
    eventually(|| !h.browser.calls_of("Browser.getWindowForTarget").is_empty()).await?;
    tokio::time::sleep(QUIET).await;
    assert_eq!(
        h.browser.held("Browser.activateTab"),
        1,
        "the fit's activation waits for the turn instead of going out beside the agent's"
    );

    h.browser.let_go("Browser.activateTab");
    timeout(WITHIN, moving).await???;
    timeout(WITHIN, starting).await???;
    assert_eq!(
        h.activated(),
        [4, 3],
        "the agent's move, then the takeover's"
    );
    assert_eq!(
        h.browser.active(),
        Some(3),
        "the takeover's tab is in front"
    );
    Ok(())
}

/// R3: a takeover with no viewer brings nothing forward, and still holds the screen against the
/// agent from its start until its release has given everything back.
#[tokio::test]
async fn a_takeover_without_a_viewer_holds_the_screen_and_activates_nothing() -> TestResult {
    let h = harness(&[3, 4]).await?;
    let front = h.front();
    assert_eq!(front.check(), Ok(()));
    let takeover = h.takeovers.start(request(None)?).await?;
    let held = Err(OwnerHasScreen {
        by: ScreenHolder::Takeover,
    });
    assert_eq!(front.check(), held);

    h.browser.hold_sweep_reads();
    let releasing = tokio::spawn({
        let takeovers = Arc::clone(&h.takeovers);
        async move { takeovers.release(takeover.id, ReleaseReason::Done).await }
    });
    eventually(|| h.browser.held_sweep_reads() > 0).await?;
    assert_eq!(
        front.check(),
        held,
        "still the owner's while the release runs"
    );
    h.browser.let_sweep_reads_go();
    timeout(WITHIN, releasing).await???;
    assert_eq!(front.check(), Ok(()), "the release gave the screen back");
    assert!(
        h.browser.calls_of("Browser.activateTab").is_empty(),
        "a takeover without a viewer brings nothing forward"
    );
    Ok(())
}

/// A host fit holds the screen against the agent from the start of its claim, before the claim
/// awaits anything, until its restore's plan has run. A claim that fails holds nothing.
#[tokio::test]
async fn a_host_fit_holds_the_screen_from_its_claim_to_its_restore() -> TestResult {
    let h = harness(&[3, 4]).await?;
    let front = h.front();
    let held = Err(OwnerHasScreen {
        by: ScreenHolder::Fit,
    });
    h.browser.hold("Browser.getWindowForTarget");
    let fitting = tokio::spawn({
        let fits = Arc::clone(&h.fits);
        async move { fits.fit(TAB, PHONE).await }
    });
    eventually(|| h.browser.held("Browser.getWindowForTarget") > 0).await?;
    assert_eq!(front.check(), held, "held mid-claim");
    h.browser.let_go("Browser.getWindowForTarget");
    let fit = timeout(WITHIN, fitting).await???;
    assert_eq!(front.check(), held, "held while the fit is active");

    h.browser.hold("Browser.setWindowBounds");
    let restoring = tokio::spawn({
        let fits = Arc::clone(&h.fits);
        async move { fits.restore(fit.id).await }
    });
    eventually(|| h.browser.held("Browser.setWindowBounds") > 0).await?;
    assert_eq!(
        front.check(),
        held,
        "held while its restore puts the window back"
    );
    h.browser.let_go("Browser.setWindowBounds");
    timeout(WITHIN, restoring).await???;
    assert_eq!(front.check(), Ok(()), "the restore gave the screen back");

    h.xvnc.refuse_next(3);
    let refused = h.fits.fit(TAB, PHONE).await;
    assert!(matches!(refused, Err(PartError::Screen(_))), "{refused:?}");
    assert_eq!(front.check(), Ok(()), "a claim that failed holds nothing");
    Ok(())
}

/// A followTask view is open when a host fits tab 3 to a phone. The task acts on 4 meanwhile:
/// its follower stays paused while the fit holds the screen, and brings 4 forward once the fit
/// is restored.
#[tokio::test]
async fn follow_pauses_while_a_fit_holds_the_screen() -> TestResult {
    let h = harness(&[3, 4]).await?;
    let task = TaskId::parse("checkout")?;
    let _view = serve(
        h.displays
            .admit(DisplayRequest::View {
                follow: Some(Follow::Task(task.clone())),
            })
            .await?,
    );
    let fit = h.fits.fit(TAB, PHONE).await?;
    h.operation(&task, 4);
    tokio::time::sleep(QUIET).await;
    assert_eq!(
        h.activated(),
        [3],
        "the fit's own activation only: the follower waits while the fit holds the screen"
    );

    h.fits.restore(fit.id).await?;
    eventually(|| h.activated() == [3, 4]).await?;
    Ok(())
}
