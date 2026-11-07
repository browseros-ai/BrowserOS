//! The run path's one handle on the host side. `Runtime::run` makes a [`RunEmitter`] for each
//! run, and every host event the run causes enters the tap through it.
//!
//! ```text
//!  Runtime::run ── run_started ──────────▶ RunStarted   before the tab check: a run whose tab
//!       │                                               is already closed is still a run
//!       └─ RunEnding::ended / its drop ──▶ RunEnded     every return, and a dropped future
//! ```
//!
//! Exactly one `RunEnded` per run, and nothing after it: the emitter's state moves from running
//! to ended once, under its lock. That is what lets a `RunPass` dropped after the run put
//! `PassReleased` behind every event the run emitted.

use std::sync::{Arc, Mutex};

use crate::host::Host;
use crate::host::events::HostEvent;
use crate::host::ids::{EpochMs, SessionKey};
use crate::host::tap::Tap;
use crate::run::{PageId, RunId, RunStatus};

/// One run's emitter. Cheap to clone. Without a host side, or for a run with no session, it
/// holds nothing and every method returns at once, so such a run does no host work at all.
#[derive(Clone, Default)]
pub(crate) struct RunEmitter(Option<Arc<Emission>>);

struct Emission {
    tap: Arc<Tap>,
    session: SessionKey,
    run: RunId,
    /// Held across each push, so `RunEnded` goes into the tap once and nothing follows it.
    state: Mutex<State>,
}

enum State {
    Running,
    /// `RunEnded` is in the tap; the run emits nothing more.
    Ended,
}

/// The `RunEnded` a started run owes the host side. [`Self::ended`] sends it with the run's
/// status; dropping the guard unsent (the caller dropped the run's future) sends it as
/// `Cancelled`.
#[must_use = "dropping it ends the run on the host side as cancelled"]
pub(crate) struct RunEnding(RunEmitter);

impl RunEmitter {
    /// The emitter for run `run` of `session`, a no-op unless both a host and a session exist.
    pub(crate) fn new(host: Option<&Host>, session: Option<&SessionKey>, run: &RunId) -> Self {
        let (Some(host), Some(session)) = (host, session) else {
            return Self::default();
        };
        Self(Some(Arc::new(Emission {
            tap: Arc::clone(host.tap()),
            session: session.clone(),
            run: run.clone(),
            state: Mutex::new(State::Running),
        })))
    }

    /// Puts `RunStarted` for a run starting on `page` in the tap, and returns the `RunEnded` the
    /// run now owes. `Runtime::run` calls it first, before anything that can end the run.
    pub(crate) fn run_started(&self, page: &PageId) -> RunEnding {
        if let Some(emission) = &self.0 {
            emission.push(HostEvent::RunStarted {
                session: emission.session.clone(),
                run: emission.run.clone(),
                page: page.clone(),
                at: now(),
            });
        }
        RunEnding(self.clone())
    }

    fn run_ended(&self, status: RunStatus) {
        let Some(emission) = &self.0 else {
            return;
        };
        let Ok(mut state) = emission.state.lock() else {
            return;
        };
        if matches!(*state, State::Ended) {
            return;
        }
        *state = State::Ended;
        emission.push(HostEvent::RunEnded {
            session: emission.session.clone(),
            run: emission.run.clone(),
            status,
            at: now(),
        });
    }
}

impl RunEnding {
    pub(crate) fn ended(self, status: RunStatus) {
        self.0.run_ended(status);
    }
}

impl Drop for RunEnding {
    fn drop(&mut self) {
        // After `ended` the run is already over on the host side, and this does nothing.
        self.0.run_ended(RunStatus::Cancelled);
    }
}

impl Emission {
    fn push(&self, event: HostEvent) {
        // The tap counts what it refuses; a run never waits on, or fails for, the host side.
        let _pushed = self.tap.try_push(event);
    }
}

/// The wall clock, as host events carry it. A clock set before the epoch reads as its first
/// millisecond: a wrong time beats a missing event.
fn now() -> EpochMs {
    EpochMs::now().unwrap_or(EpochMs::MIN)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use tokio::time;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::fake_browser::{Exchange, FakeBrowser, PAGE, Replies};
    use crate::host::HostOptions;
    use crate::run::RunRequest;
    use crate::runtime::{Runtime, RuntimeOptions};

    const SESSION: &str = "7f3a9c21";

    /// How long a test waits for what a run leaves running in the background.
    const SETTLE: Duration = Duration::from_secs(5);

    /// A click that presses, then holds the button for 5 s: mid-call for as long as a test needs.
    const LONG_CLICK: &str = "await page.click('#go', { delay: 5000 })";

    async fn hosted(browser: &FakeBrowser) -> Runtime {
        let options = RuntimeOptions {
            host: Some(HostOptions::new(9222, 1)),
        };
        Runtime::connect_with(browser.cdp().await, options)
            .await
            .unwrap()
    }

    fn request(code: &str) -> RunRequest {
        RunRequest {
            code: code.to_string(),
            page: PageId::new(PAGE.to_string()),
            timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(),
            session: Some(SessionKey::new(SESSION)),
            artifacts: None,
        }
    }

    fn tap(runtime: &Runtime) -> &Tap {
        runtime.host().unwrap().tap()
    }

    /// The tap's events so far with their times cleared, so tests compare what happened.
    fn events(tap: &Tap) -> Vec<HostEvent> {
        tap.drain().into_iter().map(untimed).collect()
    }

    fn untimed(event: HostEvent) -> HostEvent {
        match event {
            HostEvent::RunStarted {
                session, run, page, ..
            } => HostEvent::RunStarted {
                session,
                run,
                page,
                at: EpochMs::MIN,
            },
            HostEvent::RunEnded {
                session,
                run,
                status,
                ..
            } => HostEvent::RunEnded {
                session,
                run,
                status,
                at: EpochMs::MIN,
            },
            other => other,
        }
    }

    fn started(run: &RunId, page: &str) -> HostEvent {
        HostEvent::RunStarted {
            session: SessionKey::new(SESSION),
            run: run.clone(),
            page: PageId::new(page.to_string()),
            at: EpochMs::MIN,
        }
    }

    fn ended(run: &RunId, status: RunStatus) -> HostEvent {
        HostEvent::RunEnded {
            session: SessionKey::new(SESSION),
            run: run.clone(),
            status,
            at: EpochMs::MIN,
        }
    }

    fn released() -> HostEvent {
        HostEvent::PassReleased {
            session: SessionKey::new(SESSION),
        }
    }

    /// The run a dropped future was running, which no outcome names: the one `RunStarted` names.
    fn run_of(events: &[HostEvent]) -> RunId {
        match events.first() {
            Some(HostEvent::RunStarted { run, .. }) => run.clone(),
            _ => panic!("the first event starts the run: {events:#?}"),
        }
    }

    fn millis_since_epoch() -> u64 {
        let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        u64::try_from(since.as_millis()).unwrap()
    }

    /// Waits until `done` holds for the fake's exchanges.
    async fn until(browser: &FakeBrowser, done: impl Fn(&[Exchange]) -> bool) {
        let give_up = Instant::now() + SETTLE;
        loop {
            let exchanges = browser.exchanges();
            if done(&exchanges) {
                return;
            }
            assert!(Instant::now() < give_up, "never happened: {exchanges:#?}");
            time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Waits until every CDP session the fake handed out was asked to detach. By then every
    /// task a run left behind has ended, since each holds its tab's session, so the run can emit
    /// nothing more.
    async fn detached(browser: &FakeBrowser) {
        until(browser, |exchanges| {
            let count = |method: &str| exchanges.iter().filter(|e| e.method() == method).count();
            let attached = count("Target.attachToTarget");
            attached > 0 && attached == count("Target.detachFromTarget")
        })
        .await;
    }

    /// The CDP methods the fake received, sorted: what was sent, whatever the background order.
    fn methods(browser: &FakeBrowser) -> Vec<String> {
        let mut methods: Vec<String> = browser
            .exchanges()
            .iter()
            .map(|exchange| exchange.method().to_string())
            .collect();
        methods.sort();
        methods
    }

    /// Both events carry the run's id and session, and wall-clock times taken within the run.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_reports_its_start_and_its_end_once_each() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&browser).await;
        let before = millis_since_epoch();
        let outcome = runtime.run(request("return 1")).await;
        let after = millis_since_epoch();

        assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
        let raw = tap(&runtime).drain();
        let times: Vec<u64> = raw
            .iter()
            .filter_map(|event| match event {
                HostEvent::RunStarted { at, .. } | HostEvent::RunEnded { at, .. } => Some(at.get()),
                _ => None,
            })
            .collect();
        assert!(
            matches!(times[..], [start, end] if before <= start && start <= end && end <= after),
            "{times:?} lie within [{before}, {after}]"
        );
        assert_eq!(
            raw.into_iter().map(untimed).collect::<Vec<_>>(),
            [
                started(&outcome.run_id, PAGE),
                ended(&outcome.run_id, RunStatus::Ok)
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_whose_tab_is_closed_still_starts_and_ends() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&browser).await;
        let mut closed = request("return 1");
        closed.page = PageId::new("closed-tab".to_string());
        let outcome = runtime.run(closed).await;

        assert_eq!(outcome.status, RunStatus::Error, "{outcome:?}");
        assert_eq!(
            events(tap(&runtime)),
            [
                started(&outcome.run_id, "closed-tab"),
                ended(&outcome.run_id, RunStatus::Error)
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pass_dropped_after_its_run_lands_behind_every_event_of_the_run() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&browser).await;
        let pass = runtime
            .host()
            .unwrap()
            .issue_pass(SessionKey::new(SESSION), None);
        let mut labeled = request("return 1");
        labeled.session = Some(pass.session().clone());
        let outcome = runtime.run(labeled).await;
        drop(pass);

        assert_eq!(
            events(tap(&runtime)),
            [
                started(&outcome.run_id, PAGE),
                ended(&outcome.run_id, RunStatus::Ok),
                released(),
            ]
        );
    }

    /// The run tool drops a run's future mid-click and then its pass. The run ends on the host
    /// side once, as cancelled, the release lands behind it, and nothing follows once the
    /// click's own task has ended too.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dropped_run_ends_once_as_cancelled() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&browser).await;
        let pass = runtime
            .host()
            .unwrap()
            .issue_pass(SessionKey::new(SESSION), None);
        let run = tokio::spawn({
            let runtime = runtime.clone();
            async move { runtime.run(request(LONG_CLICK)).await }
        });
        until(&browser, |exchanges| {
            exchanges
                .iter()
                .any(|exchange| exchange.command["params"]["type"] == "mousePressed")
        })
        .await;
        run.abort();
        assert!(run.await.is_err_and(|error| error.is_cancelled()));
        drop(pass);
        detached(&browser).await;

        let events = events(tap(&runtime));
        let run = run_of(&events);
        assert_eq!(
            events,
            [
                started(&run, PAGE),
                ended(&run, RunStatus::Cancelled),
                released()
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_without_a_session_tells_the_host_nothing() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&browser).await;
        let mut unlabeled = request("await page.click('#go')");
        unlabeled.session = None;
        let outcome = runtime.run(unlabeled).await;

        assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
        assert!(tap(&runtime).drain().is_empty());
    }

    /// The host side sends no CDP command of its own: a tracked run on a runtime with a host
    /// sends exactly what the same run sends on a runtime without one.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_host_side_adds_no_cdp_traffic_to_a_run() {
        let code = "await page.click('#go'); return page.url()";
        let plain = FakeBrowser::start(Replies::default()).await;
        let untracked = plain.runtime().await.run(request(code)).await;
        let tracked_browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&tracked_browser).await;
        let tracked = runtime.run(request(code)).await;

        assert_eq!(untracked.status, RunStatus::Ok, "{untracked:?}");
        assert_eq!(tracked.status, RunStatus::Ok, "{tracked:?}");
        assert_eq!(events(tap(&runtime)).len(), 2, "the tracked run reported");
        detached(&plain).await;
        detached(&tracked_browser).await;
        assert_eq!(methods(&tracked_browser), methods(&plain));
    }
}
