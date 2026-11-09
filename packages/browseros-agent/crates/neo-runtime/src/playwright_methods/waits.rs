//! Selector, URL and lifecycle waits behind one loop and an input-free tab seam, in the main
//! frame or a child frame.
//!
//! ```text
//! wait_for_selector ── SelectorCondition ── check: prepare, pre-checks, selectorState
//!                      │                   changed: loop A's backoff timer
//! wait_for_url ─────┐   │
//! wait_for_load_state ─ NavigationCondition ─ check: URL/flags/lifecycle counts
//!                      │                    changed: buffered commits + watched state
//!                      └─ wait_until: check → changed → check (Progress::race on each)
//! ```
//!
//! No wait acquires or holds the tab's input lock, including during probes and sleeps.
//! [`WaitTab`] has no input operation: a concurrent click or key press may satisfy a wait.
//! All awaits use the call's existing [`Progress`], so its deadline never restarts. Dropping
//! the condition drops every subscription and timer; waits spawn no tasks. Commit queues
//! retain intermediate URLs, and lifecycle counters retain events across watch coalescing.
//!
//! A child frame's URL and lifecycle waits run the same [`NavigationCondition`] over a
//! [`FrameObservation`]: the frame's feed (`page.rs` `FrameFeeds`), which the tab's watcher and
//! the watcher of the OOPIF session hosting the frame write. The observation finds that host on
//! its first `next`, and again after the frame's document moves to another process or its host
//! session goes; a removed frame ends the wait with `Frame was detached`.

use std::future::{self, Future};
use std::sync::Arc;

use neo_runtime_spec::injected::{
    InjectedSelectorElementParams, InjectedSelectorStateParams, SelectorMatch,
};
use neo_runtime_spec::serve::{
    FrameWaitForSelectorParams, FrameWaitForSelectorParamsState as SelectorState, LifecycleEvent,
};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch};
use tokio::time;

use crate::browser_representations::locator_handlers;
use crate::browser_representations::page::{
    FrameState, FrameTab, NavigationObservation, PageSession,
};
use crate::chromium_bridge::frame_selector;
use crate::chromium_bridge::frames::{FrameScope, Host};
use crate::chromium_bridge::playwright_injected_methods;
use crate::playwright_methods::click::{LoopAWaits, PageError, SelectorFacts, Tab};
use crate::playwright_methods::evaluate::{self, CreatedHandle, Handles};
use crate::playwright_methods::navigate::{
    Lifecycle, Navigation, NavigationState, NavigationTab, Observation,
};
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::url_match::{SerializedUrlMatch, UrlMatcher};

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
    /// Starts recording the page's navigation requests (`Network.enable`), once per session, so
    /// a navigation whose request fails rejects a URL wait, as in Playwright.
    fn track_network(&self) -> impl Future<Output = Result<(), PageError>> + Send;
    fn pending_navigation(&self) -> Option<String>;
    fn navigation_settled(&self) -> impl Future<Output = Result<(), PageError>> + Send;
    fn selector_state(
        &self,
        selector: &str,
        strict: bool,
    ) -> impl Future<Output = Result<Option<SelectorMatch>, PageError>> + Send;

    /// `selector_state` for a wait that returns its element: when the first match meets `state`
    /// (attached, or visible), also that element adopted into the frame's main world as a new
    /// handle of the run (Playwright's `element._adoptTo(mainContext)`). An element that could
    /// not be adopted reads as none, so the wait keeps polling. Only the real tab can adopt.
    fn selector_element(
        &self,
        selector: &str,
        strict: bool,
        state: SelectorState,
        handles: &Handles,
    ) -> impl Future<Output = Result<(Option<SelectorMatch>, Option<CreatedHandle>), PageError>> + Send
    {
        let _ = (selector, strict, state, handles);
        future::ready(Err(PageError::Contract(
            "this tab cannot return a selector wait's element".to_string(),
        )))
    }

    /// The tab's locator-handler checkpoint (`locator_handlers.rs`), between the two navigation
    /// checks of Playwright's `performActionPreChecks`; none on a tab without handlers.
    fn locator_handlers_checkpoint(
        &self,
        progress: &Progress,
    ) -> impl Future<Output = Result<(), CallError>> + Send {
        let _ = progress;
        future::ready(Ok(()))
    }
}

/// Decoded once by the route.
#[derive(Debug)]
pub(crate) struct WaitForSelectorOptions {
    pub(crate) selector: String,
    pub(crate) strict: bool,
    pub(crate) state: SelectorState,
    /// Playwright's `!omitReturnValue`: an attached or visible wait answers its element, adopted
    /// into the main world (`frames.ts` `waitForSelector`), as `locator.evaluate` needs.
    pub(crate) return_element: bool,
    /// Playwright's `performActionPreChecksAndLog`: each attempt first runs the page's action
    /// pre-checks (a pending navigation, the locator handlers, a navigation again). Every served
    /// wait does, as Playwright's dispatcher passes `true`; the locator-handler checkpoint's own
    /// wait for its selector to hide does not.
    pub(crate) pre_checks: bool,
}

impl WaitForSelectorOptions {
    /// A served wait, which Playwright's dispatcher sends with `performActionPreChecksAndLog`
    /// set, so its attempts run the pre-checks.
    ///
    /// Playwright: `server/frames.ts` `Frame.waitForSelector`: `if (performActionPreChecksAndLog)`
    pub(crate) fn from_params(params: FrameWaitForSelectorParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
            state: params.state.unwrap_or(SelectorState::Visible),
            return_element: params.omit_return_value != Some(true),
            pre_checks: true,
        }
    }
}

/// Neo extension on the Frame guid: the SDK sends one call because its bridge has no event
/// channel. Keep this shape in sync with neo-quickjs-injected's `NeoFrameCommands` and the wire
/// tests.
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
        let wait_until = params.wait_until.unwrap_or(LifecycleEvent::Load);
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
    /// Every lifecycle event Playwright waits for is served, `load` when absent.
    pub(crate) fn from_params(params: NeoWaitForLoadStateParams) -> Self {
        Self {
            state: params.state.unwrap_or(LifecycleEvent::Load),
        }
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
    async fn check(&mut self, progress: &Progress) -> Result<bool, CallError>;
    async fn changed(&mut self) -> Result<(), PlaywrightError>;
}

/// Preparation can lose its execution context to a commit. Retry through the same loop and
/// deadline as a DOM probe, before subscribing to navigation events.
pub(crate) async fn prepare_tab<T: WaitTab>(tab: &T, progress: &Progress) -> Result<(), CallError> {
    let mut backoff = LoopAWaits::new(progress.timeout_ms());
    let _ = backoff.next_wait();
    wait_until(&mut PrepareCondition { tab, backoff }, progress).await
}

struct PrepareCondition<'a, T> {
    tab: &'a T,
    backoff: LoopAWaits,
}

impl<T: WaitTab> Condition for PrepareCondition<'_, T> {
    async fn check(&mut self, _progress: &Progress) -> Result<bool, CallError> {
        match self.tab.prepare().await {
            Ok(()) => Ok(true),
            Err(error) => polled(error),
        }
    }

    async fn changed(&mut self) -> Result<(), PlaywrightError> {
        time::sleep(self.backoff.next_wait()).await;
        Ok(())
    }
}

/// [`wait_for_selector_element`] without the element, as the policy tests drive it.
#[cfg(test)]
pub(crate) async fn wait_for_selector<T: WaitTab>(
    tab: &T,
    options: &WaitForSelectorOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    wait_for_selector_element(tab, options, progress)
        .await
        .map(|_| ())
}

/// Playwright's `Frame.waitForSelector`: waits for `options.state`, answering the element, adopted
/// into the main world, when `options.return_element` asks for it and the state is attached or
/// visible.
pub(crate) async fn wait_for_selector_element<T: WaitTab>(
    tab: &T,
    options: &WaitForSelectorOptions,
    progress: &Progress,
) -> Result<Option<CreatedHandle>, CallError> {
    name_selector(progress, &options.selector, options.state);
    let mut backoff = LoopAWaits::new(progress.timeout_ms());
    // wait_until checks immediately; consume loop A's initial zero before parking.
    let _ = backoff.next_wait();
    let mut condition = SelectorCondition {
        tab,
        options,
        named: false,
        backoff,
        element: None,
    };
    wait_until(&mut condition, progress).await?;
    Ok(condition.element)
}

struct SelectorCondition<'a, T> {
    tab: &'a T,
    options: &'a WaitForSelectorOptions,
    named: bool,
    backoff: LoopAWaits,
    /// The adopted element of the probe that met the wait, when the caller wants it.
    element: Option<CreatedHandle>,
}

impl<T: WaitTab> SelectorCondition<'_, T> {
    /// Playwright's `performActionPreChecks` for one attempt, after the tab is prepared (its
    /// watcher knows of pending navigations): settle a pending navigation, run the locator
    /// handlers, then settle any navigation a handler started, all before resolving the selector
    /// (its first description and validity check included).
    ///
    /// Playwright: `server/page.ts` `Page.performActionPreChecks`: `await this._performLocatorHandlersCheckpoint(progress);`
    async fn pre_checks(&mut self, progress: &Progress) -> Result<bool, CallError> {
        if let Err(error) = self.settle_navigation(progress).await {
            return polled(error);
        }
        self.tab.locator_handlers_checkpoint(progress).await?;
        match self.settle_navigation(progress).await {
            Ok(()) => Ok(true),
            Err(error) => polled(error),
        }
    }

    /// `performActionPreChecks`' navigation half (`page.ts` `_performWaitForNavigationCheck`):
    /// a requested main-frame navigation that has not settled is logged and waited for.
    async fn settle_navigation(&self, progress: &Progress) -> Result<(), PageError> {
        if let Some(url) = self.tab.pending_navigation() {
            progress.log(format!("  waiting for \"{url}\" navigation to finish..."));
            self.tab.navigation_settled().await?;
        }
        Ok(())
    }

    async fn probe(&mut self, progress: &Progress) -> Result<bool, PageError> {
        if !self.named {
            let facts = self.tab.describe(&self.options.selector).await?;
            name_selector(progress, &facts.locator, self.options.state);
            if let Some(invalid) = facts.invalid {
                return Err(PageError::Threw(invalid));
            }
            self.named = true;
        }
        // Hidden and detached waits answer no element (`frames.ts`: `state === 'attached' ||
        // state === 'visible' ? … : null`).
        let wants_element = self.options.return_element
            && matches!(
                self.options.state,
                SelectorState::Attached | SelectorState::Visible
            );
        let (matched, element) = if wants_element {
            self.tab
                .selector_element(
                    &self.options.selector,
                    self.options.strict,
                    self.options.state,
                    progress.handles(),
                )
                .await?
        } else {
            let matched = self
                .tab
                .selector_state(&self.options.selector, self.options.strict)
                .await?;
            (matched, None)
        };
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
        let met = match self.options.state {
            SelectorState::Attached => matched.is_some(),
            SelectorState::Detached => matched.is_none(),
            SelectorState::Visible => visible,
            SelectorState::Hidden => !visible,
        };
        if met && wants_element {
            // An element that could not be adopted keeps the wait polling.
            // Playwright: `server/frames.ts` `Frame.waitForSelector`: `return await progress.race(element._adoptTo(mainContext));`
            self.element = element;
            return Ok(self.element.is_some());
        }
        Ok(met)
    }
}

impl<T: WaitTab> Condition for SelectorCondition<'_, T> {
    async fn check(&mut self, progress: &Progress) -> Result<bool, CallError> {
        if let Err(error) = self.tab.prepare().await {
            return polled(error);
        }
        // Playwright: `server/frames.ts` `Frame.waitForSelector`: `await this._page.performActionPreChecks(progress);`
        if self.options.pre_checks && !self.pre_checks(progress).await? {
            return Ok(false);
        }
        match self.probe(progress).await {
            Ok(ready) => Ok(ready),
            Err(error) => polled(error),
        }
    }

    async fn changed(&mut self) -> Result<(), PlaywrightError> {
        time::sleep(self.backoff.next_wait()).await;
        Ok(())
    }
}

/// Neo: a page step's error within a wait's attempt: a document that went away polls again, as
/// Playwright retries a context destroyed under it; anything else ends the wait.
fn polled(error: PageError) -> Result<bool, CallError> {
    match error {
        PageError::NotConnected => Ok(false),
        error => Err(page_error(error).into()),
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
    // `networkidle` is told from the page's requests, which Neo sees once Network is enabled.
    if options.state == LifecycleEvent::Networkidle {
        progress
            .race(tab.track_network())
            .await?
            .map_err(page_error)?;
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

/// Neo: the delay Node's `setTimeout` waits for Playwright's `waitForTimeout(ms)`: none for a
/// negative or NaN delay, and [`OVERFLOWED_TIMER`] past the 32-bit timer range, as Node sets
/// an overflowing delay to 1 ms.
pub(crate) fn timer_delay(ms: f64) -> time::Duration {
    if ms > MAX_TIMER_MS {
        return OVERFLOWED_TIMER;
    }
    time::Duration::try_from_secs_f64(ms.max(0.0) / 1000.0).unwrap_or_default()
}

/// Neo: Node's largest timer delay, `2 ** 31 - 1` ms.
const MAX_TIMER_MS: f64 = 2_147_483_647.0;

/// Neo: what Node waits instead of a delay past [`MAX_TIMER_MS`].
const OVERFLOWED_TIMER: time::Duration = time::Duration::from_millis(1);

/// Serves `Frame.waitForTimeout`: the wait, ended early when the frame goes or the page closes.
///
/// Playwright: `server/frames.ts` `Frame.waitForTimeout`: `// Make sure we react to page close or frame detach.`
pub(crate) async fn wait_for_timeout<T: WaitTab>(
    tab: &T,
    wait: time::Duration,
    progress: &Progress,
) -> Result<(), CallError> {
    let mut events = tab.observe().map_err(page_error)?;
    let gone = async {
        loop {
            if let Err(error) = events.next().await {
                return error;
            }
        }
    };
    tokio::select! {
        waited = progress.wait(wait) => waited,
        error = gone => Err(page_error(error).into()),
    }
}

pub(crate) async fn wait_for_url<T: WaitTab>(
    tab: &T,
    options: &WaitForUrlOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    // Preparation can itself observe commits; keep them queued before it starts.
    let events = tab.observe().map_err(page_error)?;
    prepare_tab(tab, progress).await?;
    progress
        .race(tab.track_network())
        .await?
        .map_err(page_error)?;
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
        // client/frame.ts waitForNavigation, which waitForURL runs: a navigation that fails
        // before the URL matched rejects the wait.
        if matches!(self.phase, Phase::Matching)
            && self.matcher.is_some()
            && let Some(error) = navigation
                .as_ref()
                .and_then(|navigation| navigation.error.clone())
        {
            return Err(PlaywrightError::error(error));
        }
        let navigation = navigation.filter(|navigation| navigation.error.is_none());
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
    fn check(&mut self, progress: &Progress) -> impl Future<Output = Result<bool, CallError>> {
        // Matching runs synchronously inside check, bounded by the matcher's evaluation budget.
        future::ready(self.satisfied(progress).map_err(CallError::from))
    }

    async fn changed(&mut self) -> Result<(), PlaywrightError> {
        self.navigation = self.events.next().await.map_err(page_error)?;
        self.check_current = self.navigation.is_none();
        Ok(())
    }
}

pub(crate) fn lifecycle_name(state: LifecycleEvent) -> &'static str {
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
        LifecycleEvent::Networkidle => lifecycle.network_idle,
    }
}

pub(crate) fn lifecycle_reached(
    since: Lifecycle,
    current: Lifecycle,
    state: LifecycleEvent,
) -> bool {
    match state {
        LifecycleEvent::Commit => true,
        LifecycleEvent::Domcontentloaded => {
            since.dom_content_loaded
                || current.dom_content_loaded_count > since.dom_content_loaded_count
        }
        LifecycleEvent::Load => since.load || current.load_count > since.load_count,
        // Playwright: `server/frames.ts` `Frame.waitForLoadState`: `if (!this._firedLifecycleEvents.has(waitUntil))`
        LifecycleEvent::Networkidle => {
            since.network_idle || current.network_idle_count > since.network_idle_count
        }
    }
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

/// What one wait observes: the main frame's commits and state, or one child frame's.
pub(crate) enum WaitObservation {
    Main(NavigationObservation),
    Frame(FrameObservation),
}

impl Observation for WaitObservation {
    fn state(&self) -> NavigationState {
        match self {
            Self::Main(main) => main.state(),
            Self::Frame(frame) => frame.state(),
        }
    }

    async fn next(&mut self) -> Result<Option<Navigation>, PageError> {
        match self {
            Self::Main(main) => main.next().await,
            Self::Frame(frame) => frame.next().await,
        }
    }
}

/// A child frame's navigation for one wait: the frame's feed, subscribed when the wait starts so
/// no later commit is lost, and the host session whose events reach it. `next` finds that host
/// first (and waits for an OOPIF host's watcher to seed the feed), then reports the frame's
/// commits and state changes; after the frame's document moves to another process, or its host
/// session goes, the next `next` finds the host again. Each `next` is raced by the wait's
/// `Progress`, so finding a host never outlives the call.
pub(crate) struct FrameObservation {
    tab: FrameTab,
    frame: String,
    state: watch::Receiver<FrameState>,
    navigations: broadcast::Receiver<Navigation>,
    /// The host found last; `None` until the first `next`.
    host: Option<Arc<Host>>,
    /// The feed's swap count when `host` was found.
    swaps: u64,
}

/// Why [`FrameObservation::next`] woke.
enum Woke {
    Navigation(Result<Navigation, RecvError>),
    Changed(bool),
    HostGone,
}

impl FrameObservation {
    fn new(tab: FrameTab, frame: &str) -> Result<Self, PageError> {
        let (state, navigations) = tab.subscribe_frame(frame)?;
        Ok(Self {
            tab,
            frame: frame.to_string(),
            state,
            navigations,
            host: None,
            swaps: 0,
        })
    }

    /// The host found last, while it still reports this frame.
    fn current_host(&self) -> Option<Arc<Host>> {
        let swaps = self.state.borrow().swaps;
        self.host
            .as_ref()
            .filter(|host| !host.gone().is_cancelled() && swaps == self.swaps)
            .cloned()
    }

    /// Finds the frame in the tab's live tree, watches its host and seeds its URL from the tree,
    /// retrying with loop A's backoff while the tree or a host is mid-change.
    async fn locate(&mut self) -> Result<(), PageError> {
        let mut backoff = LoopAWaits::new(0.0);
        let _ = backoff.next_wait();
        loop {
            match self.try_locate().await {
                Err(PageError::NotConnected) => time::sleep(backoff.next_wait()).await,
                result => return result,
            }
        }
    }

    async fn try_locate(&mut self) -> Result<(), PageError> {
        let swaps = self.state.borrow().swaps;
        let tree = self.tab.frames.tree().await?;
        let Some(info) = tree.iter().find(|info| info.id == self.frame) else {
            return Err(detached());
        };
        let context = self
            .tab
            .frames
            .cached(&self.frame)
            .ok_or(PageError::NotConnected)?;
        self.tab.watch_host(&context.host).await?;
        self.tab.seed_frame(&self.frame, &info.url);
        self.host = Some(Arc::clone(&context.host));
        self.swaps = swaps;
        Ok(())
    }
}

impl Observation for FrameObservation {
    fn state(&self) -> NavigationState {
        let state = self.state.borrow();
        NavigationState {
            pending_url: state.page.pending_url.clone(),
            url: state.page.committed_url.clone(),
            lifecycle: state.page.lifecycle(),
        }
    }

    async fn next(&mut self) -> Result<Option<Navigation>, PageError> {
        if self.state.borrow().detached {
            return Err(detached());
        }
        let Some(host) = self.current_host() else {
            self.locate().await?;
            // The host's seed may have changed the state: the wait checks it again.
            return Ok(None);
        };
        let woke = tokio::select! {
            biased;
            navigation = self.navigations.recv() => Woke::Navigation(navigation),
            changed = self.state.changed() => Woke::Changed(changed.is_ok()),
            () = host.gone().cancelled() => Woke::HostGone,
        };
        match woke {
            Woke::Navigation(Ok(navigation)) => Ok(Some(navigation)),
            // The feed only closes when its frame is removed.
            Woke::Navigation(Err(RecvError::Closed)) | Woke::Changed(false) => Err(detached()),
            Woke::Changed(true) if self.state.borrow().detached => Err(detached()),
            // A lagging queue, a state change or a lost host: the wait rechecks the state, and
            // the next `next` finds the host again if it went.
            Woke::Navigation(Err(RecvError::Lagged(_))) | Woke::Changed(true) | Woke::HostGone => {
                Ok(None)
            }
        }
    }
}

/// Neo's error for a frame that is gone, as a direct scope reports it (`frames.rs`).
fn detached() -> PageError {
    PageError::Threw("Frame was detached".to_string())
}

impl WaitTab for FrameTab {
    type Observation = WaitObservation;

    async fn prepare(&self) -> Result<(), PageError> {
        PageSession::prepare(self)
            .await
            .map_err(|error| playwright_injected_methods::classify(&error))
    }

    async fn describe(&self, selector: &str) -> Result<SelectorFacts, PageError> {
        Tab::describe(self, selector).await
    }

    fn observe(&self) -> Result<Self::Observation, PageError> {
        match &self.scope {
            FrameScope::Frame(frame) if *frame != self.frames.main().id => Ok(
                WaitObservation::Frame(FrameObservation::new(self.clone(), frame)?),
            ),
            _ => Ok(WaitObservation::Main(self.observe_waits())),
        }
    }

    async fn track_network(&self) -> Result<(), PageError> {
        NavigationTab::track_network(self).await.map(|_| ())
    }
    fn pending_navigation(&self) -> Option<String> {
        PageSession::pending_navigation(self)
    }

    async fn navigation_settled(&self) -> Result<(), PageError> {
        PageSession::navigation_settled(self).await.map(|_| ())
    }

    async fn locator_handlers_checkpoint(&self, progress: &Progress) -> Result<(), CallError> {
        locator_handlers::checkpoint(self, progress).await
    }

    async fn selector_element(
        &self,
        selector: &str,
        strict: bool,
        state: SelectorState,
        handles: &Handles,
    ) -> Result<(Option<SelectorMatch>, Option<CreatedHandle>), PageError> {
        let Some(located) = frame_selector::locate(self, selector, strict).await? else {
            return Ok((None, None));
        };
        let group = self.group(&located.frame, "wait-element");
        let found = located
            .frame
            .call(
                Some(group.group()),
                &InjectedSelectorElementParams {
                    selector: located.selector,
                    strict,
                },
            )
            .await?;
        let (Some(element), Some(preview)) = (found.element, found.preview) else {
            return Ok((None, None));
        };
        let matched = SelectorMatch {
            count: found.count,
            visible: found.visible && located.frame_visible,
            preview,
        };
        if state == SelectorState::Visible && !matched.visible {
            return Ok((Some(matched), None));
        }
        let created = match evaluate::element_handle_for(
            self,
            &located.frame,
            &element,
            &matched.preview,
            handles,
        )
        .await
        {
            Ok(created) => Some(created),
            Err(PageError::NotConnected) => None,
            Err(error) => return Err(error),
        };
        Ok((Some(matched), created))
    }

    async fn selector_state(
        &self,
        selector: &str,
        strict: bool,
    ) -> Result<Option<SelectorMatch>, PageError> {
        let Some(located) = frame_selector::locate(self, selector, strict).await? else {
            return Ok(None);
        };
        let mut matched = located
            .frame
            .call(
                None,
                &InjectedSelectorStateParams {
                    selector: located.selector,
                    strict,
                },
            )
            .await?
            .matched;
        if let Some(matched) = &mut matched {
            matched.visible &= located.frame_visible;
        }
        Ok(matched)
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
    use crate::playwright_methods::navigate::{Navigation, NavigationState, Observation};
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
                        request: matches!(event, Event::Commit(_)),
                        error: None,
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
            for event in self.on_prepare.lock().expect("prepare hook").drain(..) {
                self.emit(event);
            }
            self.prepare.answer().await
        }
        async fn describe(&self, _: &str) -> Result<SelectorFacts, PageError> {
            self.descriptions.fetch_add(1, Ordering::Relaxed);
            self.describe.answer().await
        }
        fn track_network(&self) -> impl Future<Output = Result<(), PageError>> {
            future::ready(Ok(()))
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
            _: bool,
        ) -> Result<Option<SelectorMatch>, PageError> {
            assert!(
                self.state.borrow().pending_url.is_none(),
                "probe must wait for navigation"
            );
            self.probes.fetch_add(1, Ordering::Relaxed);
            self.probe.answer().await
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
            return_element: false,
            pre_checks: true,
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
                            request: true,
                            error: None,
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
}
