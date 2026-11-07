//! Main-frame selector, URL and lifecycle waits behind one loop, `waitForTimeout`'s timer, and an
//! input-free tab seam.
//!
//! ```text
//! wait_for_selector ── SelectorCondition ── check: prepare, pre-check, selectorState
//!                      │                   changed: loop A's backoff timer
//! wait_for_url ─────┐   │
//! wait_for_load_state ─ NavigationCondition ─ check: URL/flags/lifecycle counts
//!                      │                    changed: buffered commits + watched state
//!                      └─ wait_until: check → changed → check (Progress::race on each)
//! wait_for_timeout ─── one timer: prepare raced against it (arms close detection), then the
//!                      rest of the timer through Progress::race
//! ```
//!
//! No wait acquires or holds the tab's input lock, including during probes and sleeps.
//! [`WaitTab`] has no input operation: a concurrent click or key press may satisfy a wait.
//! All awaits use the call's existing [`Progress`], so its deadline never restarts. Dropping
//! the condition drops every subscription and timer; waits spawn no tasks. Commit queues
//! retain intermediate URLs, and lifecycle counters retain events across watch coalescing.

use std::future::{self, Future};
use std::mem;
use std::pin;
use std::sync::Arc;
use std::time::Duration;

use neo_runtime_spec::injected::{
    InjectedDescribeSelectorParams, InjectedSelectorStateParams, SelectorMatch,
};
use neo_runtime_spec::serve::{
    ElementHandleChannel, FrameWaitForSelectorParams,
    FrameWaitForSelectorParamsState as SelectorState, FrameWaitForTimeoutParams, LifecycleEvent,
};
use serde::Deserialize;
use tokio::time;

use crate::click::{LoopAWaits, PageError, SelectorFacts};
use crate::navigate::{Lifecycle, Navigation, Observation};
use crate::page::{NavigationObservation, PageSession};
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::url_match::{SerializedUrlMatch, UrlMatcher};
use crate::world;

/// Only the operations waits need. Every future is cancel-safe: dropping it loses at most
/// its answer. There is deliberately no input lock or input operation at this seam.
pub(crate) trait WaitTab: Send + Sync {
    type Observation: Observation;

    fn prepare(&self) -> impl Future<Output = Result<(), PageError>> + Send;
    fn describe(
        &self,
        selector: &str,
    ) -> impl Future<Output = Result<SelectorFacts, PageError>> + Send;
    fn observe(&self) -> Result<Self::Observation, PageError>;
    fn pending_navigation(&self) -> Option<String>;
    fn navigation_settled(&self) -> impl Future<Output = Result<(), PageError>> + Send;
    fn selector_state(
        &self,
        selector: &str,
        strict: bool,
    ) -> impl Future<Output = Result<Option<SelectorMatch>, PageError>> + Send;
}

/// Decoded once by the route, with Playwright's defaults (`frames.ts:843`): `visible`, and not
/// strict, so `page`/`frame.waitForSelector` judge the first match while `locator.waitFor` sends
/// `strict: true`. `omitReturnValue` changes nothing yet. Every state replies `{}` until Neo
/// serves `ElementHandle` (the assertion below).
#[derive(Debug)]
pub(crate) struct WaitForSelectorOptions {
    pub(crate) selector: String,
    pub(crate) strict: bool,
    pub(crate) state: SelectorState,
}

impl WaitForSelectorOptions {
    pub(crate) fn from_params(params: FrameWaitForSelectorParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
            state: params.state.unwrap_or(SelectorState::Visible),
        }
    }
}

// Playwright resolves an `attached` or `visible` wait sent without `omitReturnValue` with the
// element it judged, adopted into the main world (`frames.ts:879-894`). Neo has no
// `ElementHandle` to send, so `FrameWaitForSelectorResult.element` is always `None`, and the
// script gets `null` for every state (exception E1). The generated channel is uninhabited, and
// so zero-sized, only while `spec/serve.yml` does not serve `ElementHandle`. This assertion stops
// the build the day it does, so the handle step lands with it. That step belongs at
// `wait_for_selector`'s success point, on the element the same probe judged; neo-evaluate owns
// turning it into a handle (`main_world::adopt`, then `handles.mint`).
const _: () = assert!(
    mem::size_of::<ElementHandleChannel>() == 0,
    "ElementHandle is served now: mint it for attached/visible waits without omitReturnValue \
     (frames.ts:879-894) via neo-evaluate's main_world::adopt + handles.mint",
);

/// Neo extension on the Frame guid: the SDK sends one call because its bridge has no event
/// channel. Keep this shape in sync with neo-sdk's `NeoFrameCommands` and the wire tests.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NeoWaitForUrlParams {
    pub(crate) url: SerializedUrlMatch,
    pub(crate) wait_until: Option<LifecycleEvent>,
}

/// The second Frame extension, over the same call/reply transport as Playwright commands.
#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) struct NeoWaitForLoadStateParams {
    pub(crate) state: Option<LifecycleEvent>,
}

#[derive(Debug)]
pub(crate) struct WaitForUrlOptions {
    matcher: UrlMatcher,
    wait_until: LifecycleEvent,
}

impl WaitForUrlOptions {
    pub(crate) fn from_params(params: &NeoWaitForUrlParams) -> Result<Self, PlaywrightError> {
        let wait_until = supported_lifecycle(params.wait_until, "waitUntil")?;
        Ok(Self {
            matcher: UrlMatcher::new(&params.url)?,
            wait_until,
        })
    }
}

#[derive(Debug)]
pub(crate) struct WaitForLoadStateOptions {
    state: LifecycleEvent,
}

impl WaitForLoadStateOptions {
    pub(crate) fn from_params(params: NeoWaitForLoadStateParams) -> Result<Self, PlaywrightError> {
        Ok(Self {
            state: supported_lifecycle(params.state, "state")?,
        })
    }
}

fn supported_lifecycle(
    state: Option<LifecycleEvent>,
    name: &str,
) -> Result<LifecycleEvent, PlaywrightError> {
    if state == Some(LifecycleEvent::Networkidle) {
        return Err(PlaywrightError::error(format!(
            "Neo does not support {name}: \"networkidle\" yet"
        )));
    }
    Ok(state.unwrap_or(LifecycleEvent::Load))
}

/// Node's `TIMEOUT_MAX` (`lib/internal/timers.js`), 2^31 − 1 ms, the longest delay `setTimeout`
/// keeps.
const NODE_TIMEOUT_MAX_MS: f64 = 2_147_483_647.0;

/// `waitForTimeout`'s delay, decoded once by the route. Playwright's server hands `waitTimeout`
/// to Node's `setTimeout` (`frames.ts:1461`), so the delay follows Node's rule in
/// `lib/internal/timers.js`. Its `Timeout` constructor turns a delay that is NaN, below 1 or
/// above [`NODE_TIMEOUT_MAX_MS`] into 1 ms, and `insert` truncates any other to whole ms.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WaitForTimeoutOptions {
    pub(crate) delay: Duration,
}

impl WaitForTimeoutOptions {
    pub(crate) fn from_params(params: &FrameWaitForTimeoutParams) -> Self {
        let ms = params.wait_timeout;
        let ms = if (1.0..=NODE_TIMEOUT_MAX_MS).contains(&ms) {
            ms.trunc()
        } else {
            1.0
        };
        // Whole milliseconds in [1, 2^31 − 1] convert exactly; the fallback is unreachable.
        let delay = Duration::try_from_secs_f64(ms / 1000.0).unwrap_or(Duration::from_millis(1));
        Self { delay }
    }
}

/// The three public waits share deadline, cancellation and disposal here. A condition's
/// entire check is raced, including every page step inside it; ready answers still check
/// Progress first, so even an already-satisfied wait rejects a closed page.
async fn wait_until(condition: &mut impl Condition, progress: &Progress) -> Result<(), CallError> {
    loop {
        progress.check()?;
        if progress.race(condition.check(progress)).await?? {
            return Ok(());
        }
        progress.race(condition.changed()).await??;
    }
}

trait Condition {
    async fn check(&mut self, progress: &Progress) -> Result<bool, PlaywrightError>;
    async fn changed(&mut self) -> Result<(), PlaywrightError>;
}

/// Preparation can lose its execution context to a commit. Retry through the same loop and
/// deadline as a DOM probe, before subscribing to navigation events.
async fn prepare_tab<T: WaitTab>(tab: &T, progress: &Progress) -> Result<(), CallError> {
    let mut backoff = LoopAWaits::new(progress.timeout_ms());
    let _ = backoff.next_wait();
    wait_until(&mut PrepareCondition { tab, backoff }, progress).await
}

struct PrepareCondition<'a, T> {
    tab: &'a T,
    backoff: LoopAWaits,
}

impl<T: WaitTab> Condition for PrepareCondition<'_, T> {
    async fn check(&mut self, _progress: &Progress) -> Result<bool, PlaywrightError> {
        match self.tab.prepare().await {
            Ok(()) => Ok(true),
            Err(PageError::NotConnected) => Ok(false),
            Err(error) => Err(page_error(error)),
        }
    }

    async fn changed(&mut self) -> Result<(), PlaywrightError> {
        time::sleep(self.backoff.next_wait()).await;
        Ok(())
    }
}

pub(crate) async fn wait_for_selector<T: WaitTab>(
    tab: &T,
    options: &WaitForSelectorOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    name_selector(progress, &options.selector, options.state);
    let mut backoff = LoopAWaits::new(progress.timeout_ms());
    // wait_until checks immediately; consume loop A's initial zero before parking.
    let _ = backoff.next_wait();
    wait_until(
        &mut SelectorCondition {
            tab,
            options,
            named: false,
            backoff,
        },
        progress,
    )
    .await
}

struct SelectorCondition<'a, T> {
    tab: &'a T,
    options: &'a WaitForSelectorOptions,
    named: bool,
    backoff: LoopAWaits,
}

impl<T: WaitTab> SelectorCondition<'_, T> {
    async fn probe(&mut self, progress: &Progress) -> Result<bool, PageError> {
        self.tab.prepare().await?;
        // Playwright server/page.ts:547-569: settle navigation before resolving the selector,
        // including the first description/validity check.
        if let Some(url) = self.tab.pending_navigation() {
            progress.log(format!("  waiting for \"{url}\" navigation to finish..."));
            self.tab.navigation_settled().await?;
        }
        if !self.named {
            let facts = self.tab.describe(&self.options.selector).await?;
            name_selector(progress, &facts.locator, self.options.state);
            if let Some(invalid) = facts.invalid {
                return Err(PageError::Threw(invalid));
            }
            self.named = true;
        }
        let matched = self
            .tab
            .selector_state(&self.options.selector, self.options.strict)
            .await?;
        let visible = matched.as_ref().is_some_and(|matched| matched.visible);
        if let Some(matched) = &matched {
            let log = if matched.count > 1 {
                format!(
                    "  locator resolved to {} elements. Proceeding with the first one: {}",
                    matched.count, matched.preview
                )
            } else {
                format!(
                    "  locator resolved to {} {}",
                    if visible { "visible" } else { "hidden" },
                    matched.preview
                )
            };
            progress.log(log);
        }
        Ok(match self.options.state {
            SelectorState::Attached => matched.is_some(),
            SelectorState::Detached => matched.is_none(),
            SelectorState::Visible => visible,
            SelectorState::Hidden => !visible,
        })
    }
}

impl<T: WaitTab> Condition for SelectorCondition<'_, T> {
    async fn check(&mut self, progress: &Progress) -> Result<bool, PlaywrightError> {
        match self.probe(progress).await {
            Ok(ready) => Ok(ready),
            Err(PageError::NotConnected) => Ok(false),
            Err(error) => Err(page_error(error)),
        }
    }

    async fn changed(&mut self) -> Result<(), PlaywrightError> {
        time::sleep(self.backoff.next_wait()).await;
        Ok(())
    }
}

fn name_selector(progress: &Progress, locator: &str, state: SelectorState) {
    let suffix = match state {
        SelectorState::Attached => "",
        SelectorState::Detached => " to be detached",
        SelectorState::Visible => " to be visible",
        SelectorState::Hidden => " to be hidden",
    };
    progress.name_locator(&format!("{locator}{suffix}"));
}

pub(crate) async fn wait_for_load_state<T: WaitTab>(
    tab: &T,
    options: &WaitForLoadStateOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    prepare_tab(tab, progress).await?;
    progress.check()?;
    if options.state == LifecycleEvent::Commit {
        progress.log("  not waiting, \"commit\" event already fired");
        return Ok(());
    }
    let events = tab.observe().map_err(page_error)?;
    let since = events.state().lifecycle;
    wait_until(
        &mut NavigationCondition {
            events,
            matcher: None,
            phase: Phase::Loading(since),
            wait_until: options.state,
            navigation: None,
            check_current: false,
            initial_check: true,
        },
        progress,
    )
    .await
}

pub(crate) async fn wait_for_url<T: WaitTab>(
    tab: &T,
    options: &WaitForUrlOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    // Preparation can itself observe commits; keep them queued before it starts.
    let events = tab.observe().map_err(page_error)?;
    prepare_tab(tab, progress).await?;
    wait_until(
        &mut NavigationCondition {
            events,
            matcher: Some(&options.matcher),
            phase: Phase::Matching,
            wait_until: options.wait_until,
            navigation: None,
            check_current: true,
            initial_check: true,
        },
        progress,
    )
    .await
}

struct NavigationCondition<'a, O> {
    events: O,
    matcher: Option<&'a UrlMatcher>,
    phase: Phase,
    wait_until: LifecycleEvent,
    navigation: Option<Navigation>,
    check_current: bool,
    initial_check: bool,
}

#[derive(Clone, Copy)]
enum Phase {
    Matching,
    Loading(Lifecycle),
}

impl<O: Observation> NavigationCondition<'_, O> {
    fn satisfied(&mut self, progress: &Progress) -> Result<bool, PlaywrightError> {
        let state = self.events.state();
        let navigation = self.navigation.take();
        if self.matcher.is_some()
            && let Some(navigation) = &navigation
        {
            progress.log(format!("  navigated to \"{}\"", navigation.url));
        }
        if matches!(self.phase, Phase::Matching)
            && let Some(matcher) = self.matcher
        {
            // Once waiting for future commits, consume them in queue order. A watch update
            // (including lag recovery) also checks current state, covering the initial seed.
            if let Some(navigation) = navigation {
                if matcher.matches(&navigation.url)? {
                    self.phase = Phase::Loading(navigation.lifecycle);
                }
            } else if self.check_current
                && let Some(url) = &state.url
                && matcher.matches(url)?
            {
                self.phase = Phase::Loading(state.lifecycle);
            }
            self.check_current = false;
            if self.initial_check && matches!(self.phase, Phase::Matching) {
                let to_url = matcher
                    .glob()
                    .map_or_else(String::new, |glob| format!(" to \"{glob}\""));
                progress.log(format!(
                    "waiting for navigation{to_url} until \"{}\"",
                    lifecycle_name(self.wait_until)
                ));
            }
        }
        let initial = self.initial_check;
        self.initial_check = false;
        let Phase::Loading(since) = self.phase else {
            return Ok(false);
        };
        let ready = lifecycle_reached(since, state.lifecycle, self.wait_until);
        if ready {
            let name = lifecycle_name(self.wait_until);
            if initial && lifecycle_fired(since, self.wait_until) {
                progress.log(format!("  not waiting, \"{name}\" event already fired"));
            } else if !lifecycle_fired(since, self.wait_until) {
                progress.log(format!("  \"{name}\" event fired"));
            }
        }
        Ok(ready)
    }
}

impl<O: Observation> Condition for NavigationCondition<'_, O> {
    fn check(
        &mut self,
        progress: &Progress,
    ) -> impl Future<Output = Result<bool, PlaywrightError>> {
        // Matching runs synchronously inside check, bounded by the matcher's evaluation budget.
        future::ready(self.satisfied(progress))
    }

    async fn changed(&mut self) -> Result<(), PlaywrightError> {
        self.navigation = self.events.next().await.map_err(page_error)?;
        self.check_current = self.navigation.is_none();
        Ok(())
    }
}

fn lifecycle_name(state: LifecycleEvent) -> &'static str {
    match state {
        LifecycleEvent::Commit => "commit",
        LifecycleEvent::Domcontentloaded => "domcontentloaded",
        LifecycleEvent::Load => "load",
        LifecycleEvent::Networkidle => "networkidle",
    }
}

fn lifecycle_fired(lifecycle: Lifecycle, state: LifecycleEvent) -> bool {
    match state {
        LifecycleEvent::Commit => true,
        LifecycleEvent::Domcontentloaded => lifecycle.dom_content_loaded,
        LifecycleEvent::Load => lifecycle.load,
        LifecycleEvent::Networkidle => false,
    }
}

fn lifecycle_reached(since: Lifecycle, current: Lifecycle, state: LifecycleEvent) -> bool {
    match state {
        LifecycleEvent::Commit => true,
        LifecycleEvent::Domcontentloaded => {
            since.dom_content_loaded
                || current.dom_content_loaded_count > since.dom_content_loaded_count
        }
        LifecycleEvent::Load => since.load || current.load_count > since.load_count,
        LifecycleEvent::Networkidle => false,
    }
}

/// `frames.ts:1459-1471` `waitForTimeout`, a timer raced against the page closing.
///
/// The timer counts from the call's arrival at the dispatcher ([`Progress::started`]), so the
/// tab's lazy session attach, which runs before this function, never stretches the delay. Neo
/// learns that a tab was closed from outside only through the event watcher that `prepare`
/// starts, so `prepare` runs first, raced against the same timer, only to arm that watch.
/// `Closed` ends the call with `TargetClosedError`; any other error is ignored and the timer runs
/// on; a `prepare` slower than the delay is dropped, so it never stretches the wait either. The
/// rest of the timer goes through `Progress::race`, so a close mid-wait is `TargetClosedError` and
/// a run end is `RunEnded`. `prepare` is the only page call.
pub(crate) async fn wait_for_timeout<T: WaitTab>(
    tab: &T,
    options: &WaitForTimeoutOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    let mut timer = pin::pin!(match progress.started().checked_add(options.delay) {
        Some(due) => time::sleep_until(due),
        None => time::sleep(options.delay),
    });
    let prepared = progress
        .race(async {
            tokio::select! {
                biased;
                () = timer.as_mut() => None,
                prepared = tab.prepare() => Some(prepared),
            }
        })
        .await?;
    match prepared {
        None => return Ok(()),
        Some(Err(PageError::Closed)) => return Err(PlaywrightError::target_closed().into()),
        Some(
            Ok(()) | Err(PageError::NotConnected | PageError::Threw(_) | PageError::Contract(_)),
        ) => {}
    }
    progress.race(timer).await
}

fn page_error(error: PageError) -> PlaywrightError {
    match error {
        PageError::Closed => PlaywrightError::target_closed(),
        PageError::Threw(message) | PageError::Contract(message) => PlaywrightError::error(message),
        PageError::NotConnected => PlaywrightError::error(
            "Execution context was destroyed, most likely because of a navigation",
        ),
    }
}

impl WaitTab for Arc<PageSession> {
    type Observation = NavigationObservation;

    async fn prepare(&self) -> Result<(), PageError> {
        PageSession::prepare(self)
            .await
            .map_err(|error| world::classify(&error))
    }

    async fn describe(&self, selector: &str) -> Result<SelectorFacts, PageError> {
        let facts = self
            .world
            .call(
                None,
                &InjectedDescribeSelectorParams {
                    selector: selector.to_string(),
                },
            )
            .await?;
        Ok(SelectorFacts {
            locator: facts.locator,
            invalid: facts.invalid_selector,
        })
    }

    fn observe(&self) -> Result<Self::Observation, PageError> {
        Ok(self.observe_waits())
    }
    fn pending_navigation(&self) -> Option<String> {
        PageSession::pending_navigation(self)
    }

    async fn navigation_settled(&self) -> Result<(), PageError> {
        PageSession::navigation_settled(self).await.map(|_| ())
    }

    async fn selector_state(
        &self,
        selector: &str,
        strict: bool,
    ) -> Result<Option<SelectorMatch>, PageError> {
        Ok(self
            .world
            .call(
                None,
                &InjectedSelectorStateParams {
                    selector: selector.to_string(),
                    strict,
                },
            )
            .await?
            .matched)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use neo_runtime_spec::injected::SelectorMatch;
    use neo_runtime_spec::serve::{
        FrameWaitForSelectorParamsState as SelectorState, LifecycleEvent,
    };
    use serde_json::json;
    use tokio::sync::Mutex as AsyncMutex;
    use tokio::sync::{broadcast, watch};
    use tokio::time::{self, Instant};
    use tokio_util::sync::CancellationToken;
    use tokio_util::task::TaskTracker;

    use super::*;
    use crate::deadline::Deadline;
    use crate::navigate::{Navigation, NavigationState, Observation};
    use crate::progress::{CallEntry, CallStart, ErrorName, RunScope};
    use crate::run::RunEnd;

    const START: &str = "https://a.test/start";
    const MATCH: &str = "https://a.test/match";

    struct Step<T> {
        delay: Duration,
        answers: Mutex<VecDeque<Result<T, PageError>>>,
    }

    impl<T: Clone> Step<T> {
        fn ready(value: T) -> Self {
            Self {
                delay: Duration::ZERO,
                answers: Mutex::new([Ok(value)].into()),
            }
        }

        fn script(&mut self, answers: impl IntoIterator<Item = Result<T, PageError>>) {
            *self.answers.get_mut().expect("step lock") = answers.into_iter().collect();
        }

        async fn answer(&self) -> Result<T, PageError> {
            if !self.delay.is_zero() {
                time::sleep(self.delay).await;
            }
            let mut answers = self.answers.lock().expect("step lock");
            if answers.len() > 1 {
                answers.pop_front().expect("scripted answer")
            } else {
                answers.front().cloned().expect("repeated answer")
            }
        }
    }

    #[derive(Clone, Copy)]
    enum Event {
        Commit(&'static str),
        Hash(&'static str),
        Load,
        Dom,
        Seed(&'static str),
        Pending,
        Settled,
    }

    struct Events {
        state: watch::Receiver<NavigationState>,
        commits: broadcast::Receiver<Navigation>,
    }

    impl Observation for Events {
        fn state(&self) -> NavigationState {
            self.state.borrow().clone()
        }

        async fn next(&mut self) -> Result<Option<Navigation>, PageError> {
            tokio::select! {
                biased;
                result = self.commits.recv() => match result {
                    Ok(commit) => Ok(Some(commit)),
                    Err(broadcast::error::RecvError::Lagged(_)) => Ok(None),
                    Err(broadcast::error::RecvError::Closed) => Err(PageError::Closed),
                },
                result = self.state.changed() => result.map(|()| None).map_err(|_| PageError::Closed),
            }
        }
    }

    /// Scripted page steps and real notification channels. There is deliberately no input
    /// operation in `WaitTab`; a held input lock below must not prevent any kind of wait.
    struct FakeTab {
        prepare: Step<()>,
        describe: Step<SelectorFacts>,
        probe: Step<Option<SelectorMatch>>,
        preparations: AtomicUsize,
        probes: AtomicUsize,
        descriptions: AtomicUsize,
        state: watch::Sender<NavigationState>,
        commits: broadcast::Sender<Navigation>,
        on_prepare: Mutex<Vec<Event>>,
        on_subscribe: Mutex<Vec<Event>>,
        input: AsyncMutex<()>,
    }

    impl FakeTab {
        fn new() -> Self {
            let (state, _) = watch::channel(NavigationState {
                url: Some(START.to_string()),
                ..NavigationState::default()
            });
            let (commits, _) = broadcast::channel(16);
            Self {
                prepare: Step::ready(()),
                describe: Step::ready(SelectorFacts {
                    locator: "locator('#item')".to_string(),
                    invalid: None,
                }),
                probe: Step::ready(Some(matched(true))),
                preparations: AtomicUsize::new(0),
                probes: AtomicUsize::new(0),
                descriptions: AtomicUsize::new(0),
                state,
                commits,
                on_prepare: Mutex::default(),
                on_subscribe: Mutex::default(),
                input: AsyncMutex::new(()),
            }
        }

        fn emit(&self, event: Event) {
            let mut navigation = None;
            self.state.send_modify(|state| match event {
                Event::Commit(url) | Event::Hash(url) => {
                    if matches!(event, Event::Commit(_)) {
                        state.lifecycle.dom_content_loaded = false;
                        state.lifecycle.load = false;
                    }
                    state.url = Some(url.to_string());
                    state.pending_url = None;
                    navigation = Some(Navigation {
                        document: matches!(event, Event::Commit(_)).then(|| url.to_string()),
                        url: url.to_string(),
                        lifecycle: state.lifecycle,
                    });
                }
                Event::Load => {
                    if !state.lifecycle.load {
                        state.lifecycle.load = true;
                        state.lifecycle.load_count += 1;
                    }
                }
                Event::Dom => {
                    if !state.lifecycle.dom_content_loaded {
                        state.lifecycle.dom_content_loaded = true;
                        state.lifecycle.dom_content_loaded_count += 1;
                    }
                }
                Event::Seed(url) => state.url = Some(url.to_string()),
                Event::Pending => state.pending_url = Some(MATCH.to_string()),
                Event::Settled => state.pending_url = None,
            });
            if let Some(navigation) = navigation {
                let _ = self.commits.send(navigation);
            }
        }

        fn subscribers(&self) -> (usize, usize) {
            (self.commits.receiver_count(), self.state.receiver_count())
        }
    }

    impl WaitTab for FakeTab {
        type Observation = Events;
        async fn prepare(&self) -> Result<(), PageError> {
            self.preparations.fetch_add(1, Ordering::Relaxed);
            for event in self.on_prepare.lock().expect("prepare hook").drain(..) {
                self.emit(event);
            }
            self.prepare.answer().await
        }
        async fn describe(&self, _: &str) -> Result<SelectorFacts, PageError> {
            self.descriptions.fetch_add(1, Ordering::Relaxed);
            self.describe.answer().await
        }
        fn observe(&self) -> Result<Events, PageError> {
            let observation = Events {
                state: self.state.subscribe(),
                commits: self.commits.subscribe(),
            };
            for event in self.on_subscribe.lock().expect("subscribe hook").drain(..) {
                self.emit(event);
            }
            Ok(observation)
        }
        fn pending_navigation(&self) -> Option<String> {
            self.state.borrow().pending_url.clone()
        }
        async fn navigation_settled(&self) -> Result<(), PageError> {
            self.state
                .subscribe()
                .wait_for(|state| state.pending_url.is_none())
                .await
                .map(|_| ())
                .map_err(|_| PageError::Closed)
        }
        async fn selector_state(
            &self,
            _: &str,
            strict: bool,
        ) -> Result<Option<SelectorMatch>, PageError> {
            assert!(
                self.state.borrow().pending_url.is_none(),
                "probe must wait for navigation"
            );
            self.probes.fetch_add(1, Ordering::Relaxed);
            let answer = self.probe.answer().await?;
            // The glue's strict query throws for more than one match, whatever the state.
            if strict && answer.as_ref().is_some_and(|matched| matched.count > 1) {
                return Err(PageError::Threw(
                    "Error: strict mode violation: locator('#item') resolved to 2 elements"
                        .to_string(),
                ));
            }
            Ok(answer)
        }
    }

    fn matched(visible: bool) -> SelectorMatch {
        SelectorMatch {
            count: 1,
            visible,
            preview: "<div id=\"item\">".to_string(),
        }
    }

    fn selector(state: SelectorState) -> WaitForSelectorOptions {
        WaitForSelectorOptions {
            selector: "#item".to_string(),
            strict: true,
            state,
        }
    }

    /// `page`/`frame.waitForSelector`'s params: `strict` and `state` as the script gave them, and
    /// no `omitReturnValue`.
    fn page_selector(state: Option<SelectorState>, strict: Option<bool>) -> WaitForSelectorOptions {
        WaitForSelectorOptions::from_params(FrameWaitForSelectorParams {
            selector: "#item".to_string(),
            strict,
            state,
            omit_return_value: None,
        })
    }

    /// Two matches whose first is hidden, reported as the page does, by the first match's state.
    fn first_of_two_hidden() -> SelectorMatch {
        SelectorMatch {
            count: 2,
            visible: false,
            preview: "<div id=\"item\">".to_string(),
        }
    }

    fn url(wait_until: LifecycleEvent) -> WaitForUrlOptions {
        WaitForUrlOptions::from_params(&NeoWaitForUrlParams {
            url: SerializedUrlMatch {
                glob: Some(MATCH.to_string()),
                ..SerializedUrlMatch::default()
            },
            wait_until: Some(wait_until),
        })
        .expect("URL options")
    }

    fn load(state: LifecycleEvent) -> WaitForLoadStateOptions {
        WaitForLoadStateOptions::from_params(NeoWaitForLoadStateParams { state: Some(state) })
            .expect("load options")
    }

    fn timer(wait_timeout: f64) -> WaitForTimeoutOptions {
        WaitForTimeoutOptions::from_params(&FrameWaitForTimeoutParams { wait_timeout })
    }

    /// A run whose calls have no timeout of their own, as the SDK sends `waitForTimeout`, and
    /// whose deadline is a minute away.
    fn untimed_run() -> Run {
        Run::with_deadline(0.0, 60_000)
    }

    struct Run {
        progress: Progress,
        scope: RunScope,
        closed: CancellationToken,
        cancel: CancellationToken,
    }

    impl Run {
        fn new(timeout: f64) -> Self {
            Self::with_deadline(timeout, 1000)
        }
        fn with_deadline(timeout: f64, deadline: u64) -> Self {
            let cancel = CancellationToken::new();
            let closed = CancellationToken::new();
            let scope = RunScope::new(
                Deadline::new(Duration::from_millis(deadline), cancel.clone()),
                TaskTracker::new(),
            );
            let entry = Arc::new(CallEntry::new(CallStart {
                call_id: 1,
                guid: "frame@T".to_string(),
                method: "wait".to_string(),
                title: None,
                params: json!({}),
                location: None,
                input: false,
            }));
            let progress = Progress::new(entry, timeout, scope.clone(), closed.clone());
            Self {
                progress,
                scope,
                closed,
                cancel,
            }
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Kind {
        Selector,
        Url,
        Load,
    }
    const KINDS: [Kind; 3] = [Kind::Selector, Kind::Url, Kind::Load];

    async fn wait(kind: Kind, tab: &FakeTab, progress: &Progress) -> Result<(), CallError> {
        match kind {
            Kind::Selector => {
                wait_for_selector(tab, &selector(SelectorState::Visible), progress).await
            }
            Kind::Url => wait_for_url(tab, &url(LifecycleEvent::Load), progress).await,
            Kind::Load => wait_for_load_state(tab, &load(LifecycleEvent::Load), progress).await,
        }
    }

    fn assert_error(result: Result<(), CallError>, name: ErrorName) {
        let Err(CallError::Playwright(error)) = result else {
            panic!("expected {name:?}, got {result:?}");
        };
        assert_eq!(error.name, name);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_all_selector_states_resolve_without_parking_when_satisfied() {
        for (state, answer) in [
            (SelectorState::Attached, Some(matched(false))),
            (SelectorState::Detached, None),
            (SelectorState::Visible, Some(matched(true))),
            (SelectorState::Hidden, Some(matched(false))),
            (SelectorState::Hidden, None),
        ] {
            let mut tab = FakeTab::new();
            tab.probe = Step::ready(answer);
            let run = Run::new(500.0);
            let now = Instant::now();
            assert_eq!(
                wait_for_selector(&tab, &selector(state), &run.progress).await,
                Ok(())
            );
            assert_eq!(now.elapsed(), Duration::ZERO);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_all_selector_states_can_become_satisfied_later() {
        for (state, before, after) in [
            (SelectorState::Attached, None, Some(matched(true))),
            (SelectorState::Detached, Some(matched(true)), None),
            (
                SelectorState::Visible,
                Some(matched(false)),
                Some(matched(true)),
            ),
            (
                SelectorState::Hidden,
                Some(matched(true)),
                Some(matched(false)),
            ),
        ] {
            let mut tab = FakeTab::new();
            tab.probe.script([Ok(before), Ok(after)]);
            let run = Run::new(500.0);
            let now = Instant::now();
            assert_eq!(
                wait_for_selector(&tab, &selector(state), &run.progress).await,
                Ok(())
            );
            assert!(now.elapsed() > Duration::ZERO);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_strict_and_invalid_selectors_fail_at_once() {
        for invalid in [true, false] {
            let mut tab = FakeTab::new();
            if invalid {
                tab.describe = Step::ready(SelectorFacts {
                    locator: "bad".to_string(),
                    invalid: Some("invalid selector".to_string()),
                });
            } else {
                tab.probe
                    .script([Err(PageError::Threw("strict mode violation".to_string()))]);
            }
            let run = Run::new(500.0);
            let now = Instant::now();
            assert_error(
                wait_for_selector(&tab, &selector(SelectorState::Visible), &run.progress).await,
                ErrorName::Error,
            );
            assert_eq!(now.elapsed(), Duration::ZERO);
            if invalid {
                assert_eq!(tab.probes.load(Ordering::Relaxed), 0);
            }
        }
    }

    #[test]
    fn tests_that_selector_options_take_any_omit_return_value_with_playwrights_defaults() {
        for omit_return_value in [None, Some(false), Some(true)] {
            let options = WaitForSelectorOptions::from_params(FrameWaitForSelectorParams {
                selector: "#item".to_string(),
                strict: None,
                state: None,
                omit_return_value,
            });
            assert_eq!(
                (options.selector.as_str(), options.strict, options.state),
                ("#item", false, SelectorState::Visible),
                "{omit_return_value:?}"
            );
        }
        let options = page_selector(Some(SelectorState::Hidden), Some(true));
        assert_eq!(
            (options.strict, options.state),
            (true, SelectorState::Hidden)
        );
    }

    /// `page`/`frame.waitForSelector` are not strict: the first of two matches decides, so a
    /// hidden first match keeps a `visible` wait polling and meets `hidden` and `attached` at once.
    #[tokio::test(start_paused = true)]
    async fn tests_that_page_waits_judge_the_first_of_several_matches() {
        for (state, met) in [
            (None, false),
            (Some(SelectorState::Visible), false),
            (Some(SelectorState::Hidden), true),
            (Some(SelectorState::Attached), true),
            (Some(SelectorState::Detached), false),
        ] {
            let mut tab = FakeTab::new();
            tab.probe = Step::ready(Some(first_of_two_hidden()));
            let run = Run::new(100.0);
            let start = Instant::now();
            let result = wait_for_selector(&tab, &page_selector(state, None), &run.progress).await;
            if met {
                assert_eq!(result, Ok(()), "{state:?}");
                assert_eq!(start.elapsed(), Duration::ZERO, "{state:?}");
            } else {
                assert_error(result, ErrorName::TimeoutError);
                assert_eq!(start.elapsed(), Duration::from_millis(100), "{state:?}");
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_strict_page_wait_on_two_matches_fails_at_once() {
        for state in [
            SelectorState::Attached,
            SelectorState::Detached,
            SelectorState::Visible,
            SelectorState::Hidden,
        ] {
            let mut tab = FakeTab::new();
            tab.probe = Step::ready(Some(first_of_two_hidden()));
            let run = Run::new(500.0);
            let start = Instant::now();
            let result =
                wait_for_selector(&tab, &page_selector(Some(state), Some(true)), &run.progress)
                    .await;
            let Err(CallError::Playwright(error)) = result else {
                panic!("{state:?}: expected a strict mode violation, got {result:?}");
            };
            assert_eq!(error.name, ErrorName::Error, "{state:?}");
            assert!(error.message.contains("strict mode violation"), "{error:?}");
            assert_eq!(start.elapsed(), Duration::ZERO, "{state:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_lost_contexts_retry_at_every_selector_step() {
        for step in 0..3 {
            let mut tab = FakeTab::new();
            match step {
                0 => tab.prepare.script([Err(PageError::NotConnected), Ok(())]),
                1 => tab.describe.script([
                    Err(PageError::NotConnected),
                    Ok(SelectorFacts {
                        locator: "item".to_string(),
                        invalid: None,
                    }),
                ]),
                _ => tab
                    .probe
                    .script([Err(PageError::NotConnected), Ok(Some(matched(true)))]),
            }
            let run = Run::new(500.0);
            assert_eq!(
                wait_for_selector(&tab, &selector(SelectorState::Visible), &run.progress).await,
                Ok(())
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_pending_navigation_finishes_before_the_dom_probe() {
        let tab = FakeTab::new();
        tab.emit(Event::Pending);
        let run = Run::new(500.0);
        let (result, ()) = tokio::join!(
            async {
                wait_for_selector(&tab, &selector(SelectorState::Visible), &run.progress).await
            },
            async {
                time::sleep(Duration::from_millis(40)).await;
                assert_eq!(tab.probes.load(Ordering::Relaxed), 0);
                tab.emit(Event::Settled);
            }
        );
        assert_eq!(result, Ok(()));
        assert_eq!(tab.probes.load(Ordering::Relaxed), 1);
        assert_eq!(tab.subscribers(), (0, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_every_selector_await_obeys_the_same_timeout() {
        for step in 0..4 {
            let mut tab = FakeTab::new();
            match step {
                0 => tab.prepare.delay = Duration::from_secs(5),
                1 => tab.describe.delay = Duration::from_secs(5),
                2 => tab.probe.delay = Duration::from_secs(5),
                _ => tab.emit(Event::Pending),
            }
            let run = Run::new(100.0);
            let start = Instant::now();
            assert_error(
                wait_for_selector(&tab, &selector(SelectorState::Visible), &run.progress).await,
                ErrorName::TimeoutError,
            );
            assert_eq!(start.elapsed(), Duration::from_millis(100));
            assert_eq!(tab.subscribers(), (0, 0));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_dom_probes_do_not_spin() {
        let mut tab = FakeTab::new();
        tab.probe = Step::ready(None);
        let run = Run::new(300.0);
        assert_error(
            wait_for_selector(&tab, &selector(SelectorState::Visible), &run.progress).await,
            ErrorName::TimeoutError,
        );
        let probes = tab.probes.load(Ordering::Relaxed);
        // Catch a hot loop without prescribing a retry cadence.
        assert!(probes <= 100, "too many probes: {probes}");
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_loaded_and_committed_states_resolve_without_parking() {
        for state in [
            LifecycleEvent::Commit,
            LifecycleEvent::Load,
            LifecycleEvent::Domcontentloaded,
        ] {
            let tab = FakeTab::new();
            tab.emit(Event::Load);
            tab.emit(Event::Dom);
            let run = Run::new(100.0);
            let start = Instant::now();
            assert_eq!(
                wait_for_load_state(&tab, &load(state), &run.progress).await,
                Ok(())
            );
            assert_eq!(start.elapsed(), Duration::ZERO);
            assert_eq!(tab.subscribers(), (0, 0));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_lifecycle_event_between_subscribe_and_check_is_seen() {
        for (state, event) in [
            (LifecycleEvent::Load, Event::Load),
            (LifecycleEvent::Domcontentloaded, Event::Dom),
        ] {
            let tab = FakeTab::new();
            tab.on_subscribe.lock().expect("hook").push(event);
            let run = Run::new(100.0);
            assert_eq!(
                wait_for_load_state(&tab, &load(state), &run.progress).await,
                Ok(())
            );
            assert_eq!(tab.subscribers(), (0, 0));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_domcontentloaded_waits_for_the_event_within_its_deadline() {
        let mut observed = Vec::new();
        for arrival_ms in [50, 150] {
            let tab = FakeTab::new();
            let run = Run::new(100.0);
            let started = Instant::now();
            let ((result, elapsed), ()) = tokio::join!(
                async {
                    let result = wait_for_load_state(
                        &tab,
                        &load(LifecycleEvent::Domcontentloaded),
                        &run.progress,
                    )
                    .await;
                    (result, started.elapsed())
                },
                async {
                    time::sleep(Duration::from_millis(arrival_ms)).await;
                    tab.emit(Event::Dom);
                }
            );
            observed.push((arrival_ms, result, elapsed));
            assert_eq!(tab.subscribers(), (0, 0));
        }
        assert_eq!(
            observed,
            [
                (50, Ok(()), Duration::from_millis(50)),
                (
                    150,
                    Err(CallError::Playwright(PlaywrightError::timeout(100.0))),
                    Duration::from_millis(100),
                ),
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_lifecycle_events_survive_a_commit_before_the_waiter_wakes() {
        for (state, event) in [
            (LifecycleEvent::Load, Event::Load),
            (LifecycleEvent::Domcontentloaded, Event::Dom),
        ] {
            let tab = FakeTab::new();
            let run = Run::new(100.0);
            let (result, ()) = tokio::join!(
                async { wait_for_load_state(&tab, &load(state), &run.progress).await },
                async {
                    time::sleep(Duration::from_millis(20)).await;
                    tab.emit(event);
                    tab.emit(Event::Commit(MATCH));
                }
            );
            assert_eq!(result, Ok(()));
            assert!(!tab.state.borrow().lifecycle.load);
            assert_eq!(tab.subscribers(), (0, 0));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_an_intermediate_matching_commit_is_not_lost() {
        let tab = FakeTab::new();
        tab.on_subscribe
            .lock()
            .expect("hook")
            .extend([Event::Commit(MATCH), Event::Commit(START)]);
        let run = Run::new(100.0);
        assert_eq!(
            wait_for_url(&tab, &url(LifecycleEvent::Commit), &run.progress).await,
            Ok(())
        );
        assert_eq!(tab.state.borrow().url.as_deref(), Some(START));
        assert_eq!(tab.subscribers(), (0, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_url_wait_observes_intermediate_commits_during_preparation() {
        let tab = FakeTab::new();
        tab.on_prepare
            .lock()
            .expect("prepare hook")
            .extend([Event::Commit(MATCH), Event::Commit(START)]);
        let run = Run::new(100.0);
        assert_eq!(
            wait_for_url(&tab, &url(LifecycleEvent::Commit), &run.progress).await,
            Ok(())
        );
        assert_eq!(tab.state.borrow().url.as_deref(), Some(START));
        assert_eq!(tab.subscribers(), (0, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_matching_commit_and_its_load_survive_a_second_commit() {
        let tab = FakeTab::new();
        let run = Run::new(100.0);
        let (result, ()) = tokio::join!(
            async { wait_for_url(&tab, &url(LifecycleEvent::Load), &run.progress).await },
            async {
                time::sleep(Duration::from_millis(20)).await;
                tab.emit(Event::Commit(MATCH));
                tab.emit(Event::Load);
                tab.emit(Event::Commit(START));
            }
        );
        assert_eq!(result, Ok(()));
        assert_eq!(tab.subscribers(), (0, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_hash_change_keeps_the_loaded_lifecycle() {
        let tab = FakeTab::new();
        tab.emit(Event::Load);
        tab.on_subscribe
            .lock()
            .expect("hook")
            .push(Event::Hash(MATCH));
        let run = Run::new(100.0);
        let start = Instant::now();
        assert_eq!(
            wait_for_url(&tab, &url(LifecycleEvent::Load), &run.progress).await,
            Ok(())
        );
        assert_eq!(start.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_an_unknown_url_waits_for_the_initial_seed() {
        let tab = FakeTab::new();
        tab.state.send_modify(|state| state.url = None);
        tab.emit(Event::Load);
        let run = Run::new(100.0);
        let (result, ()) = tokio::join!(
            async { wait_for_url(&tab, &url(LifecycleEvent::Load), &run.progress).await },
            async {
                time::sleep(Duration::from_millis(25)).await;
                tab.emit(Event::Seed(MATCH));
            }
        );
        assert_eq!(result, Ok(()));
        assert_eq!(tab.subscribers(), (0, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_url_waits_load_after_the_match_with_one_deadline() {
        let tab = FakeTab::new();
        let run = Run::new(100.0);
        let start = Instant::now();
        let ((result, elapsed), ()) = tokio::join!(
            async {
                let result = wait_for_url(&tab, &url(LifecycleEvent::Load), &run.progress).await;
                (result, start.elapsed())
            },
            async {
                time::sleep(Duration::from_millis(80)).await;
                tab.emit(Event::Commit(MATCH));
                time::sleep(Duration::from_millis(40)).await;
                tab.emit(Event::Load);
            }
        );
        assert_error(result, ErrorName::TimeoutError);
        assert_eq!(elapsed, Duration::from_millis(100));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_unrelated_events_never_restart_the_deadline() {
        for kind in KINDS {
            let mut tab = FakeTab::new();
            tab.probe = Step::ready(None);
            let run = Run::new(100.0);
            let start = Instant::now();
            let ((result, elapsed), ()) = tokio::join!(
                async {
                    let result = wait(kind, &tab, &run.progress).await;
                    (result, start.elapsed())
                },
                async {
                    for _ in 0..4 {
                        time::sleep(Duration::from_millis(30)).await;
                        tab.emit(Event::Commit(START));
                    }
                }
            );
            assert_error(result, ErrorName::TimeoutError);
            assert_eq!(elapsed, Duration::from_millis(100), "{kind:?}");
            assert_eq!(tab.subscribers(), (0, 0));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_zero_timeout_still_obeys_run_deadline_and_cancellation() {
        for kind in KINDS {
            for cancel in [false, true] {
                let mut tab = FakeTab::new();
                tab.probe = Step::ready(None);
                let run = Run::with_deadline(0.0, 100);
                let (result, ()) = tokio::join!(wait(kind, &tab, &run.progress), async {
                    if cancel {
                        time::sleep(Duration::from_millis(20)).await;
                        run.cancel.cancel();
                    }
                });
                assert_eq!(
                    result,
                    Err(CallError::RunEnded(if cancel {
                        RunEnd::Cancelled
                    } else {
                        RunEnd::Timeout
                    }))
                );
                assert_eq!(tab.subscribers(), (0, 0));
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_closing_the_tab_or_ending_the_run_releases_every_subscription() {
        for kind in KINDS {
            for closed in [false, true] {
                let mut tab = FakeTab::new();
                tab.probe = Step::ready(None);
                let run = Run::new(500.0);
                let (result, ()) = tokio::join!(wait(kind, &tab, &run.progress), async {
                    time::sleep(Duration::from_millis(10)).await;
                    if closed {
                        run.closed.cancel();
                    } else {
                        run.scope.end(RunEnd::Settled);
                    }
                });
                if closed {
                    assert_error(result, ErrorName::TargetClosedError);
                } else {
                    assert_eq!(result, Err(CallError::RunEnded(RunEnd::Settled)));
                }
                assert_eq!(tab.subscribers(), (0, 0));
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_already_satisfied_waits_still_reject_a_closed_tab() {
        for kind in KINDS {
            let tab = FakeTab::new();
            tab.emit(Event::Seed(MATCH));
            tab.emit(Event::Load);
            let run = Run::new(500.0);
            run.closed.cancel();
            assert_error(
                wait(kind, &tab, &run.progress).await,
                ErrorName::TargetClosedError,
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_no_wait_needs_the_tabs_input_lock() {
        for kind in KINDS {
            let mut tab = FakeTab::new();
            tab.probe.script([Ok(None), Ok(Some(matched(true)))]);
            let _input = tab.input.lock().await;
            let run = Run::new(500.0);
            let (result, ()) = tokio::join!(wait(kind, &tab, &run.progress), async {
                time::sleep(Duration::from_millis(20)).await;
                tab.emit(Event::Commit(MATCH));
                tab.emit(Event::Load);
            });
            assert_eq!(result, Ok(()), "{kind:?}");
            assert_eq!(tab.subscribers(), (0, 0));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_current_url_and_fired_load_resolve_without_parking() {
        let tab = FakeTab::new();
        tab.emit(Event::Seed(MATCH));
        tab.emit(Event::Load);
        let run = Run::new(100.0);
        let start = Instant::now();
        assert_eq!(
            wait_for_url(&tab, &url(LifecycleEvent::Load), &run.progress).await,
            Ok(())
        );
        assert_eq!(start.elapsed(), Duration::ZERO);
        assert_eq!(tab.subscribers(), (0, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_url_waits_resolve_after_unmatched_commits() {
        let tab = FakeTab::new();
        let run = Run::new(500.0);
        let (result, ()) = tokio::join!(
            async { wait_for_url(&tab, &url(LifecycleEvent::Load), &run.progress).await },
            async {
                time::sleep(Duration::from_millis(20)).await;
                tab.emit(Event::Commit(START));
                time::sleep(Duration::from_millis(20)).await;
                tab.emit(Event::Commit(MATCH));
                time::sleep(Duration::from_millis(20)).await;
                tab.emit(Event::Load);
            }
        );
        assert_eq!(result, Ok(()));
        assert_eq!(tab.subscribers(), (0, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_load_waits_resolve_for_already_fired_and_new_events() {
        for state in [
            LifecycleEvent::Load,
            LifecycleEvent::Domcontentloaded,
            LifecycleEvent::Commit,
        ] {
            let tab = FakeTab::new();
            tab.emit(Event::Load);
            tab.emit(Event::Dom);
            let run = Run::new(100.0);
            assert_eq!(
                wait_for_load_state(&tab, &load(state), &run.progress).await,
                Ok(())
            );
            assert_eq!(tab.subscribers(), (0, 0));
        }
        for (state, event) in [
            (LifecycleEvent::Load, Event::Load),
            (LifecycleEvent::Domcontentloaded, Event::Dom),
        ] {
            let tab = FakeTab::new();
            let run = Run::new(100.0);
            let (result, ()) = tokio::join!(
                async { wait_for_load_state(&tab, &load(state), &run.progress).await },
                async {
                    time::sleep(Duration::from_millis(20)).await;
                    tab.emit(event);
                }
            );
            assert_eq!(result, Ok(()));
            assert_eq!(tab.subscribers(), (0, 0));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_navigation_prechecks_precede_selector_description() {
        let mut tab = FakeTab::new();
        tab.describe = Step::ready(SelectorFacts {
            locator: "bad".to_string(),
            invalid: Some("invalid selector".to_string()),
        });
        tab.emit(Event::Pending);
        let run = Run::new(100.0);
        let start = Instant::now();
        let (result, ()) = tokio::join!(
            async {
                wait_for_selector(&tab, &selector(SelectorState::Visible), &run.progress).await
            },
            async {
                time::sleep(Duration::from_millis(20)).await;
                assert_eq!(tab.descriptions.load(Ordering::Relaxed), 0);
                tab.emit(Event::Settled);
            }
        );
        assert_error(result, ErrorName::Error);
        assert_eq!(start.elapsed(), Duration::from_millis(20));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_navigation_waits_retry_disconnected_preparation() {
        for kind in [Kind::Url, Kind::Load] {
            let mut tab = FakeTab::new();
            tab.prepare.script([Err(PageError::NotConnected), Ok(())]);
            tab.emit(Event::Seed(MATCH));
            tab.emit(Event::Load);
            let run = Run::new(100.0);
            assert_eq!(wait(kind, &tab, &run.progress).await, Ok(()), "{kind:?}");
            assert_eq!(tab.subscribers(), (0, 0));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_repeated_preparation_disconnects_keep_the_original_deadline() {
        for kind in [Kind::Url, Kind::Load] {
            let mut tab = FakeTab::new();
            tab.prepare.script([Err(PageError::NotConnected)]);
            let run = Run::new(100.0);
            let start = Instant::now();
            assert_error(
                wait(kind, &tab, &run.progress).await,
                ErrorName::TimeoutError,
            );
            assert_eq!(start.elapsed(), Duration::from_millis(100));
            assert_eq!(tab.subscribers(), (0, 0));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_watch_updates_before_commit_broadcasts_cannot_lose_a_match() {
        for load_before_broadcast in [false, true] {
            let tab = FakeTab::new();
            let run = Run::new(500.0);
            let (result, ()) = tokio::join!(
                async { wait_for_url(&tab, &url(LifecycleEvent::Load), &run.progress).await },
                async {
                    time::sleep(Duration::from_millis(20)).await;
                    let mut navigation = None;
                    tab.state.send_modify(|state| {
                        state.url = Some(MATCH.to_string());
                        state.lifecycle.load = false;
                        state.lifecycle.dom_content_loaded = false;
                        navigation = Some(Navigation {
                            document: Some(MATCH.to_string()),
                            url: MATCH.to_string(),
                            lifecycle: state.lifecycle,
                        });
                    });
                    time::sleep(Duration::from_millis(20)).await;
                    if load_before_broadcast {
                        tab.emit(Event::Load);
                    }
                    time::sleep(Duration::from_millis(20)).await;
                    let _ = tab.commits.send(navigation.expect("commit"));
                    if !load_before_broadcast {
                        tab.emit(Event::Load);
                    }
                }
            );
            assert_eq!(result, Ok(()));
            assert_eq!(tab.subscribers(), (0, 0));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_url_waits_recover_lag_from_state_and_retained_commits() {
        for matching_state in [false, true] {
            let tab = FakeTab::new();
            let run = Run::new(500.0);
            let (result, ()) = tokio::join!(
                async { wait_for_url(&tab, &url(LifecycleEvent::Load), &run.progress).await },
                async {
                    time::sleep(Duration::from_millis(20)).await;
                    for _ in 0..32 {
                        tab.emit(Event::Commit(START));
                    }
                    tab.emit(Event::Commit(MATCH));
                    tab.emit(Event::Load);
                    if matching_state {
                        for _ in 0..32 {
                            tab.emit(Event::Hash(MATCH));
                        }
                    } else {
                        tab.emit(Event::Commit(START));
                    }
                }
            );
            assert_eq!(result, Ok(()));
            assert_eq!(tab.subscribers(), (0, 0));
        }
    }

    #[test]
    fn tests_that_the_delay_follows_nodes_set_timeout_rule() {
        for wait_timeout in [
            0.0,
            -5.0,
            0.5,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            2_147_483_648.0,
            1e300,
        ] {
            assert_eq!(
                timer(wait_timeout).delay,
                Duration::from_millis(1),
                "{wait_timeout}"
            );
        }
        for (wait_timeout, ms) in [
            (1.0, 1),
            (2.5, 2),
            (300.9, 300),
            (2_147_483_647.0, 2_147_483_647),
        ] {
            assert_eq!(
                timer(wait_timeout).delay,
                Duration::from_millis(ms),
                "{wait_timeout}"
            );
        }
    }

    /// The only page call is `prepare`, and the tab's input stays free for others meanwhile.
    #[tokio::test(start_paused = true)]
    async fn tests_that_the_timer_resolves_at_its_delay_after_one_prepare() {
        let tab = FakeTab::new();
        let _input = tab.input.lock().await;
        let run = untimed_run();
        let start = Instant::now();
        assert_eq!(
            wait_for_timeout(&tab, &timer(300.0), &run.progress).await,
            Ok(())
        );
        assert_eq!(start.elapsed(), Duration::from_millis(300));
        assert_eq!(tab.preparations.load(Ordering::Relaxed), 1);
        assert_eq!(tab.descriptions.load(Ordering::Relaxed), 0);
        assert_eq!(tab.probes.load(Ordering::Relaxed), 0);
    }

    /// The delay counts from the call's start: a slow `prepare` is part of it, and one slower
    /// than the delay is abandoned when the delay ends.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_slow_prepare_never_stretches_the_delay() {
        for prepare in [Duration::from_millis(60), Duration::from_secs(5)] {
            let mut tab = FakeTab::new();
            tab.prepare.delay = prepare;
            let run = untimed_run();
            let start = Instant::now();
            assert_eq!(
                wait_for_timeout(&tab, &timer(100.0), &run.progress).await,
                Ok(()),
                "{prepare:?}"
            );
            assert_eq!(start.elapsed(), Duration::from_millis(100), "{prepare:?}");
        }
    }

    /// Dispatch attaches a tab's session before `serve` runs the wait; that time is part of the
    /// delay, and a delay already spent resolves at once.
    #[tokio::test(start_paused = true)]
    async fn tests_that_time_before_the_wait_runs_counts_against_its_delay() {
        for (before, resolves_at) in [(60, 100), (150, 150)] {
            let tab = FakeTab::new();
            let run = untimed_run();
            let arrived = Instant::now();
            time::sleep(Duration::from_millis(before)).await;
            assert_eq!(
                wait_for_timeout(&tab, &timer(100.0), &run.progress).await,
                Ok(()),
                "{before} ms before"
            );
            assert_eq!(
                arrived.elapsed(),
                Duration::from_millis(resolves_at),
                "{before} ms before"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_closing_the_tab_mid_delay_ends_the_timer_at_once() {
        let tab = FakeTab::new();
        let run = untimed_run();
        let options = timer(1000.0);
        let start = Instant::now();
        let (result, ()) = tokio::join!(wait_for_timeout(&tab, &options, &run.progress), async {
            time::sleep(Duration::from_millis(200)).await;
            run.closed.cancel();
        });
        assert_error(result, ErrorName::TargetClosedError);
        assert_eq!(start.elapsed(), Duration::from_millis(200));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_prepare_ends_the_timer_only_for_a_closed_tab() {
        let mut tab = FakeTab::new();
        tab.prepare.script([Err(PageError::Closed)]);
        let run = untimed_run();
        let start = Instant::now();
        assert_error(
            wait_for_timeout(&tab, &timer(1000.0), &run.progress).await,
            ErrorName::TargetClosedError,
        );
        assert_eq!(start.elapsed(), Duration::ZERO);
        for error in [
            PageError::NotConnected,
            PageError::Contract("bad reply".to_string()),
            PageError::Threw("Error: thrown".to_string()),
        ] {
            let mut tab = FakeTab::new();
            tab.prepare.script([Err(error.clone())]);
            let run = untimed_run();
            let start = Instant::now();
            assert_eq!(
                wait_for_timeout(&tab, &timer(400.0), &run.progress).await,
                Ok(()),
                "{error:?}"
            );
            assert_eq!(start.elapsed(), Duration::from_millis(400), "{error:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_the_run_ending_stops_the_timer() {
        for end in [RunEnd::Settled, RunEnd::Cancelled, RunEnd::Timeout] {
            let tab = FakeTab::new();
            let run = Run::with_deadline(0.0, 300);
            let options = timer(5000.0);
            let (result, ()) =
                tokio::join!(wait_for_timeout(&tab, &options, &run.progress), async {
                    time::sleep(Duration::from_millis(50)).await;
                    match end {
                        RunEnd::Settled => run.scope.end(RunEnd::Settled),
                        RunEnd::Cancelled => run.cancel.cancel(),
                        // The run's own 300 ms deadline.
                        RunEnd::Timeout => {}
                    }
                });
            assert_eq!(result, Err(CallError::RunEnded(end)), "{end:?}");
        }
    }
}
