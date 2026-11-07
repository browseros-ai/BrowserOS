//! Playwright: `server/frames.ts` `Frame.waitForNavigation`: `const navigationEvent: NavigationEvent = await helper.waitForEvent(progress, this, Frame.Events.InternalNavigation, (event: NavigationEvent) => {`
//! Playwright: `server/page.ts` `Page.reload`: `this.mainFrame().waitForNavigation(progress, true /* requiresNewDocument */, options),`
//! Playwright: `server/page.ts` `Page.goBack`: `const result = await progress.race(this.delegate.goBack());`
//! Playwright: `server/page.ts` `Page.goForward`: `const result = await progress.race(this.delegate.goForward());`
//! Playwright: `server/chromium/crPage.ts` `CRPage._go`: `const entry = history.entries[history.currentIndex + delta];`
//! Playwright: `client/frame.ts` `Frame.waitForNavigation`: `return urlMatches(this._page?.context()._options.baseURL, event.url, options.url);`
//!
//! Playwright's server `Frame.waitForNavigation` and what waits on it: the history navigations
//! `Page.reload`, `goBack` and `goForward`, and Neo's frame commands behind the client's
//! `Frame.waitForNavigation`, which Playwright's client builds on `navigated` events that Neo's
//! call/reply bridge cannot carry.
//!
//! ```text
//!  page.reload()     ─ Page.reload ────┐
//!  page.goBack()     ─ Page.goBack ────┤ subscribe → act → next navigation → waitUntil
//!  page.goForward()  ─ Page.goForward ─┘                                   → Response | null
//!  waitForNavigation({url: glob | RegExp}) ─ neo:waitForNavigation: the same wait, URL matched here
//!  waitForNavigation({url: predicate})     ─ neo:nextNavigation × n: one navigation per call, one
//!  waitForURL(predicate)                     always waiting; the realm tests each URL on its own;
//!                                            neo:finishNavigation names the first it accepted:
//!                                            waitUntil → Response | null (or neo:releaseNavigation)
//! ```
//!
//! Every wait subscribes to its frame's navigation events before it prepares the page, enables
//! the network log or acts, so no navigation it should see is lost, and every step races the
//! call's [`Progress`], so its deadline and the page's closure end it. A navigation's `Response`
//! is the network log's answer for its document, as `goto`'s (`network.rs`); it is `null` for a
//! same-document navigation, and for a document whose request the watcher never saw start: a
//! back-forward cache restore, or a request that started before this session's `Network.enable`.
//!
//! A URL predicate cannot cross into Rust, so its wait is a cursor in the run's [`Cursors`]: the
//! first `neo:nextNavigation` subscribes, and the realm keeps one such call waiting at all times,
//! so a navigation reaches it while earlier predicates are still pending (Playwright's waiter tests
//! each navigation's predicate on its own; the first to accept wins), and the page's closure or a
//! failed navigation rejects the wait even with no timeout. A cursor ends with its
//! `neo:finishNavigation`, its `neo:releaseNavigation` (the realm's wait ended otherwise), a failed
//! step (its timeout included), or the run.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use browseros_cdp::CdpError;
use neo_runtime_spec::serve::{
    LifecycleEvent, PageGoBackParams, PageGoForwardParams, PageReloadParams, ResponseChannel,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::browser_representations::page::FrameTab;
use crate::chromium_bridge::frames::FrameScope;
use crate::chromium_bridge::playwright_injected_methods;
use crate::playwright_methods::click::PageError;
use crate::playwright_methods::navigate::{
    self, Lifecycle, Navigation, NavigationTab, Observation,
};
use crate::playwright_methods::waits::{self, WaitObservation, WaitTab};
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::url_match::{SerializedUrlMatch, UrlMatcher};

/// Deviation: `client/frame.ts` `Frame.waitForNavigation` the most URL-predicate waits one run
/// may run at once; Playwright's waiter has no cap. Every wait ends its cursor however it ends
/// (accepted, released by the realm, failed), so only live waits count, and one past the cap is
/// refused rather than displacing a live one. A wait stays live until its finish or release has
/// replied; the dispatcher admits those two outside its in-flight caps, so this cap bounds them.
const MAX_CURSORS: usize = 64;

/// Deviation: `client/frame.ts` `Frame.waitForNavigation` the navigations one predicate wait
/// keeps for its finish to name, the most recent first; Playwright's waiter keeps every event its
/// pending predicates hold. A predicate still pending this many navigations later cannot win.
const MAX_ANSWERED: usize = 4096;

/// The history step a `Page` command takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoryStep {
    Reload,
    Back,
    Forward,
}

/// A decoded `Page.reload`, `goBack` or `goForward`.
#[derive(Debug)]
pub(crate) struct HistoryOptions {
    step: HistoryStep,
    wait_until: LifecycleEvent,
}

/// Playwright: `server/frames.ts` `Frame.waitForNavigation`: `const waitUntil = verifyLifecycle('waitUntil', options.waitUntil === undefined ? 'load' : options.waitUntil);`
impl HistoryOptions {
    pub(crate) fn reload(params: &PageReloadParams) -> Self {
        Self::new(HistoryStep::Reload, params.wait_until)
    }

    pub(crate) fn back(params: &PageGoBackParams) -> Self {
        Self::new(HistoryStep::Back, params.wait_until)
    }

    pub(crate) fn forward(params: &PageGoForwardParams) -> Self {
        Self::new(HistoryStep::Forward, params.wait_until)
    }

    fn new(step: HistoryStep, wait_until: Option<LifecycleEvent>) -> Self {
        Self {
            step,
            wait_until: wait_until.unwrap_or(LifecycleEvent::Load),
        }
    }
}

/// The page operations a history navigation adds to [`NavigationTab`]'s.
///
/// Playwright: `server/chromium/crPage.ts` `CRPage._go`: `await this._mainFrameSession._client.send('Page.navigateToHistoryEntry', { entryId: entry.id });`
pub(crate) trait HistoryTab: NavigationTab {
    /// `Page.reload`, as Playwright's `CRPage.reload` sends it.
    async fn reload(&self) -> Result<(), PlaywrightError>;
    /// Playwright's `CRPage._go`: navigates to the history entry `delta` steps from the current
    /// one, or answers `false` when there is none.
    async fn go(&self, delta: i64) -> Result<bool, PlaywrightError>;
}

/// Playwright's server `Page.reload`, `goBack` and `goForward`: the main frame's
/// `waitForNavigation` subscribed before the CDP step, which a reload must answer with a new
/// document, and `null` without waiting when there is no history entry to go to.
///
/// Playwright: `server/page.ts` `Page.goBack`: `const waitPromise = this.mainFrame().waitForNavigation(progress, false /* requiresNewDocument */, options).catch(e => {`
pub(crate) async fn navigate(
    tab: &impl HistoryTab,
    options: &HistoryOptions,
    progress: &Progress,
) -> Result<Option<ResponseChannel>, CallError> {
    let mut events = tab.observe().map_err(navigate::page_error)?;
    progress
        .race(tab.prepare())
        .await?
        .map_err(navigate::page_error)?;
    let lags = progress
        .race(tab.track_network())
        .await?
        .map_err(navigate::page_error)?;
    progress.log(format!(
        "waiting for navigation until \"{}\"",
        waits::lifecycle_name(options.wait_until)
    ));
    match options.step {
        HistoryStep::Reload => progress.race(tab.reload()).await??,
        step => {
            let delta = if step == HistoryStep::Back { -1 } else { 1 };
            if !progress.race(tab.go(delta)).await?? {
                return Ok(None);
            }
        }
    }
    // Playwright: `server/page.ts` `Page.reload`: `// Reload must be a new document, and should not be confused with a stray pushState.`
    let new_document = options.step == HistoryStep::Reload;
    let navigation = next_navigation(&mut events, progress, |navigation| {
        Ok(!new_document || navigation.document.is_some())
    })
    .await?;
    settle(
        &mut events,
        navigation.lifecycle,
        options.wait_until,
        progress,
    )
    .await?;
    response_of(tab, &navigation, lags, progress).await
}

/// Neo extension on the Frame guid for the client's `waitForNavigation` with a glob or `RegExp`
/// `url` (or none). Keep in sync with neo-quickjs-injected's `NeoFrameCommands` and the dispatch
/// tests.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NeoWaitForNavigationParams {
    pub(crate) url: SerializedUrlMatch,
    pub(crate) wait_until: Option<LifecycleEvent>,
}

/// A decoded `neo:waitForNavigation`.
#[derive(Debug)]
pub(crate) struct WaitForNavigationOptions {
    matcher: UrlMatcher,
    wait_until: LifecycleEvent,
}

/// Playwright: `client/frame.ts` `Frame.waitForNavigation`: `const waitUntil = verifyLoadState('waitUntil', options.waitUntil === undefined ? 'load' : options.waitUntil);`
impl WaitForNavigationOptions {
    pub(crate) fn from_params(
        params: &NeoWaitForNavigationParams,
    ) -> Result<Self, PlaywrightError> {
        Ok(Self {
            wait_until: params.wait_until.unwrap_or(LifecycleEvent::Load),
            matcher: UrlMatcher::new(&params.url)?,
        })
    }
}

/// Playwright's client `Frame.waitForNavigation` for the main frame: the first navigation after
/// the call whose URL matches (a same-document one counts), a failed navigation rejecting first;
/// then `waitUntil`; then that navigation's `Response`, or `null`.
///
/// Playwright: `client/frame.ts` `Frame.waitForNavigation`: `return urlMatches(this._page?.context()._options.baseURL, event.url, options.url);`
pub(crate) async fn wait_for_navigation(
    tab: &impl NavigationTab,
    options: &WaitForNavigationOptions,
    progress: &Progress,
) -> Result<Option<ResponseChannel>, CallError> {
    let events = tab.observe().map_err(navigate::page_error)?;
    wait_for_navigation_on(tab, events, options, progress).await
}

/// [`wait_for_navigation`] over `events`, a child frame's navigations (`waits::FrameObservation`)
/// or the main frame's: subscribed by the caller before anything here runs. A child frame's
/// `Response` is the network log's answer for its document, as the main frame's: Neo records
/// every frame's requests (`network.rs`).
///
/// Playwright: `server/frames.ts` `Frame.waitForNavigation`: `if (!this._firedLifecycleEvents.has(waitUntil))`
pub(crate) async fn wait_for_navigation_on(
    tab: &impl NavigationTab,
    mut events: impl Observation,
    options: &WaitForNavigationOptions,
    progress: &Progress,
) -> Result<Option<ResponseChannel>, CallError> {
    progress
        .race(tab.prepare())
        .await?
        .map_err(navigate::page_error)?;
    let lags = progress
        .race(tab.track_network())
        .await?
        .map_err(navigate::page_error)?;
    let to_url = options
        .matcher
        .glob()
        .filter(|glob| !glob.is_empty())
        .map_or_else(String::new, |glob| format!(" to \"{glob}\""));
    progress.log(format!(
        "waiting for navigation{to_url} until \"{}\"",
        waits::lifecycle_name(options.wait_until)
    ));
    let navigation = next_navigation(&mut events, progress, |navigation| {
        options.matcher.matches(&navigation.url)
    })
    .await?;
    settle(
        &mut events,
        navigation.lifecycle,
        options.wait_until,
        progress,
    )
    .await?;
    response_of(tab, &navigation, lags, progress).await
}

/// `neo:nextNavigation`: opens a cursor on the frame (`cursor` absent) or continues one, and
/// answers the next navigation it observes, while the realm may still be testing earlier ones.
/// `response` says what its finish answers: the navigation's `Response` (`waitForNavigation`) or
/// nothing (`waitForURL`).
///
/// Deviation: `client/frame.ts` `Frame.waitForNavigation` Playwright tests a URL predicate inside
/// one API call, on its client's `navigated` events; Neo makes one call per navigation the
/// predicate tests, plus the one still waiting when the wait ends, each in the run's call records.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NeoNextNavigationParams {
    pub(crate) cursor: Option<u64>,
    pub(crate) response: bool,
}

/// What `neo:waitForNavigation` and `neo:finishNavigation` answer: Playwright's
/// `{response?: Response}`, `{}` for `null`.
#[derive(Debug, Serialize)]
pub(crate) struct NeoNavigationResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response: Option<ResponseChannel>,
}

/// `neo:nextNavigation`'s reply: the navigation's index in its cursor and its URL, or neither once
/// the wait ended under the call (its finish or release stopped it).
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct NeoNextNavigationResult {
    pub(crate) cursor: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) index: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
}

/// `neo:finishNavigation`: ends the cursor with the navigation `index` names, the first the
/// predicate accepted: waits for `waitUntil` since it and answers as `neo:waitForNavigation` does.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NeoFinishNavigationParams {
    pub(crate) cursor: u64,
    pub(crate) index: u64,
    pub(crate) wait_until: Option<LifecycleEvent>,
}

/// A decoded `neo:finishNavigation`.
#[derive(Debug)]
pub(crate) struct FinishNavigationOptions {
    pub(crate) cursor: u64,
    index: u64,
    wait_until: LifecycleEvent,
}

/// Neo: decodes Neo's own `neo:finishNavigation`; Playwright has no such command.
impl FinishNavigationOptions {
    pub(crate) fn from_params(params: &NeoFinishNavigationParams) -> Self {
        Self {
            cursor: params.cursor,
            index: params.index,
            wait_until: params.wait_until.unwrap_or(LifecycleEvent::Load),
        }
    }
}

/// `neo:releaseNavigation`: ends a cursor whose wait ended in the realm (its predicate threw, its
/// deadline passed); a `neo:nextNavigation` still waiting on it answers at once.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NeoReleaseNavigationParams {
    pub(crate) cursor: u64,
}

/// One predicate wait's subscription and what its finish needs, shared by the wait's calls: a
/// `neo:nextNavigation` waits on the frame's queue while the realm tests earlier navigations, and
/// a finish or release from another call ends the wait, stopping that next.
pub(crate) struct Cursor<T, O> {
    tab: T,
    /// The frame's queue: a next holds it while it waits, a finish takes it to settle the load.
    events: AsyncMutex<O>,
    /// The network log's lag count when the wait began, for its response lookup.
    lags: u64,
    respond: bool,
    /// The navigations answered so far, for a finish to name by index.
    answered: Mutex<Answered>,
    /// Cancelled when the wait ends: a next still waiting answers that it ended.
    ended: CancellationToken,
}

/// A cursor's answered navigations: `navigations[i]` has index `first + i`.
#[derive(Default)]
struct Answered {
    first: u64,
    navigations: VecDeque<Navigation>,
}

/// The cursor the dispatcher keeps for a frame call.
pub(crate) type FrameCursor = Cursor<FrameTab, WaitObservation>;

/// Playwright: `client/frame.ts` `Frame.waitForNavigation`: `const response = request ? await waiter.waitForPromise(request._finalRequest()._internalResponse()) : null;`
impl<T: NavigationTab, O: Observation> Cursor<T, O> {
    /// The first step of a predicate wait, on a prepared page whose `events` the caller
    /// subscribed before preparing it: starts the network log.
    pub(crate) async fn open(
        tab: T,
        events: O,
        respond: bool,
        progress: &Progress,
    ) -> Result<Self, CallError> {
        let lags = progress
            .race(tab.track_network())
            .await?
            .map_err(navigate::page_error)?;
        Ok(Self {
            tab,
            events: AsyncMutex::new(events),
            lags,
            respond,
            answered: Mutex::default(),
            ended: CancellationToken::new(),
        })
    }

    /// The next navigation's index and URL, or `None` once the wait ended; a failed navigation
    /// rejects.
    pub(crate) async fn next(
        &self,
        progress: &Progress,
    ) -> Result<Option<(u64, String)>, CallError> {
        let mut events = tokio::select! {
            biased;
            () = self.ended.cancelled() => return Ok(None),
            events = self.events.lock() => events,
        };
        let navigation = tokio::select! {
            biased;
            () = self.ended.cancelled() => return Ok(None),
            navigation = next_navigation(&mut *events, progress, |_| Ok(true)) => navigation?,
        };
        let url = navigation.url.clone();
        let mut answered = self.answered.lock().map_err(|_| cursors_poisoned())?;
        let held = u64::try_from(answered.navigations.len()).unwrap_or(u64::MAX);
        let index = answered.first.saturating_add(held);
        answered.navigations.push_back(navigation);
        if answered.navigations.len() > MAX_ANSWERED {
            answered.navigations.pop_front();
            answered.first = answered.first.saturating_add(1);
        }
        Ok(Some((index, url)))
    }

    /// Ends the wait with navigation `index`: `waitUntil` since it, then its `Response` if this
    /// wait answers one. A next still waiting answers that the wait ended.
    pub(crate) async fn finish(
        &self,
        index: u64,
        wait_until: LifecycleEvent,
        progress: &Progress,
    ) -> Result<Option<ResponseChannel>, CallError> {
        self.end();
        let navigation = {
            let answered = self.answered.lock().map_err(|_| cursors_poisoned())?;
            index
                .checked_sub(answered.first)
                .and_then(|offset| usize::try_from(offset).ok())
                .and_then(|offset| answered.navigations.get(offset))
                .cloned()
        }
        .ok_or_else(|| unanswered(index))?;
        let mut events = progress.race(self.events.lock()).await?;
        settle(&mut *events, navigation.lifecycle, wait_until, progress).await?;
        if !self.respond {
            return Ok(None);
        }
        response_of(&self.tab, &navigation, self.lags, progress).await
    }

    /// Ends the wait: a next still waiting answers that it ended.
    pub(crate) fn end(&self) {
        self.ended.cancel();
    }
}

/// The run's open cursors by id, shared by its call tasks. A cursor leaves the table when its
/// wait ends: on its finish, its release, or a failed step.
pub(crate) struct Cursors<C> {
    next: AtomicU64,
    table: Mutex<Table<C>>,
}

/// Neo: the waits a run holds. A wait is live while its cursor is open or a finish or release
/// that claimed it ([`Cursors::begin_ending`]) has not replied; live waits count against
/// [`MAX_CURSORS`].
struct Table<C> {
    /// The open cursors, by id.
    open: BTreeMap<u64, Arc<C>>,
    /// The ids a finish or release claimed and has not replied for.
    ending: BTreeSet<u64>,
}

/// Neo: how many live waits the table holds.
impl<C> Table<C> {
    /// Every open cursor, and every claimed one already taken out of `open`.
    fn live(&self) -> usize {
        let taken = self
            .ending
            .iter()
            .filter(|id| !self.open.contains_key(id))
            .count();
        self.open.len() + taken
    }
}

/// Neo: an empty cursor table at run start; no script sees it.
impl<C> Default for Cursors<C> {
    fn default() -> Self {
        Self {
            next: AtomicU64::new(1),
            table: Mutex::new(Table {
                open: BTreeMap::new(),
                ending: BTreeSet::new(),
            }),
        }
    }
}

/// Neo: the run's cursor table for its live predicate waits; its cap is `MAX_CURSORS`'s.
impl<C> Cursors<C> {
    /// Opens `cursor` under a fresh id, refusing one past [`MAX_CURSORS`] live waits.
    pub(crate) fn insert(&self, cursor: C) -> Result<(u64, Arc<C>), PlaywrightError> {
        let mut table = self.table.lock().map_err(|_| cursors_poisoned())?;
        if table.live() >= MAX_CURSORS {
            return Err(PlaywrightError::error(format!(
                "neo-runtime: a run may wait on at most {MAX_CURSORS} URL predicates at once"
            )));
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let cursor = Arc::new(cursor);
        table.open.insert(id, Arc::clone(&cursor));
        Ok((id, cursor))
    }

    /// Cursor `id`, while it is open.
    pub(crate) fn get(&self, id: u64) -> Result<Arc<C>, PlaywrightError> {
        self.table
            .lock()
            .map_err(|_| cursors_poisoned())?
            .open
            .get(&id)
            .cloned()
            .ok_or_else(|| {
                PlaywrightError::error(format!(
                    "neo-runtime: navigation wait {id} is not open: it finished, failed or was \
                     released"
                ))
            })
    }

    /// Takes cursor `id` out of the open ones; its wait has ended, or ends with its finish.
    pub(crate) fn remove(&self, id: u64) -> Option<Arc<C>> {
        self.table.lock().ok()?.open.remove(&id)
    }

    /// Claims open cursor `id` for the one finish or release call that ends it, which the
    /// dispatcher then admits outside its in-flight caps. `None` if `id` is not open or another
    /// call claimed it. The wait stays live until the claim drops, so claimed calls never
    /// outnumber [`MAX_CURSORS`].
    pub(crate) fn begin_ending(self: &Arc<Self>, id: u64) -> Option<Ending<C>> {
        let mut table = self.table.lock().ok()?;
        if !table.open.contains_key(&id) || !table.ending.insert(id) {
            return None;
        }
        Some(Ending {
            cursors: Arc::clone(self),
            id,
        })
    }
}

/// Neo: a finish or release call's claim on its wait ([`Cursors::begin_ending`]), held until the
/// call replies or its task ends.
pub(crate) struct Ending<C> {
    cursors: Arc<Cursors<C>>,
    id: u64,
}

/// Neo: gives the claimed wait's place under `MAX_CURSORS` back.
impl<C> Drop for Ending<C> {
    fn drop(&mut self) {
        if let Ok(mut table) = self.cursors.table.lock() {
            table.ending.remove(&self.id);
        }
    }
}

/// `neo:nextNavigation` on `tab`'s frame: opens a cursor there (subscribed before the page is
/// prepared) or continues the one `params` names. A failed step ends the wait.
///
/// Neo: the dispatcher's glue from a frame call to its cursor; what a script sees is `Cursor`'s.
pub(crate) async fn next_frame_navigation(
    cursors: &Cursors<FrameCursor>,
    tab: FrameTab,
    params: &NeoNextNavigationParams,
    progress: &Progress,
) -> Result<NeoNextNavigationResult, CallError> {
    let (id, cursor) = if let Some(id) = params.cursor {
        (id, cursors.get(id)?)
    } else {
        // The main frame's queue, as `wait_for_navigation` reads it, rejects on an overflow
        // rather than skip a navigation the predicate never saw; a child frame's rechecks.
        let events = if matches!(tab.scope, FrameScope::Main) {
            WaitObservation::Main(NavigationTab::observe(&tab).map_err(navigate::page_error)?)
        } else {
            WaitTab::observe(&tab).map_err(navigate::page_error)?
        };
        waits::prepare_tab(&tab, progress).await?;
        let cursor = Cursor::open(tab, events, params.response, progress).await?;
        cursors.insert(cursor)?
    };
    match cursor.next(progress).await {
        Ok(next) => {
            let (index, url) = next.unzip();
            Ok(NeoNextNavigationResult {
                cursor: id,
                index,
                url,
            })
        }
        Err(error) => {
            if let Some(cursor) = cursors.remove(id) {
                cursor.end();
            }
            Err(error)
        }
    }
}

/// `neo:finishNavigation`: ends the cursor `options` names with the navigation it names.
///
/// Neo: the dispatcher's glue from a frame call to its cursor; what a script sees is `Cursor`'s.
pub(crate) async fn finish_frame_navigation(
    cursors: &Cursors<FrameCursor>,
    options: &FinishNavigationOptions,
    progress: &Progress,
) -> Result<NeoNavigationResult, CallError> {
    let cursor = cursors.remove(options.cursor).ok_or_else(|| {
        PlaywrightError::error(format!(
            "neo-runtime: navigation wait {} is not open: it finished, failed or was released",
            options.cursor
        ))
    })?;
    let response = cursor
        .finish(options.index, options.wait_until, progress)
        .await?;
    Ok(NeoNavigationResult { response })
}

/// `neo:releaseNavigation`: ends the cursor `params` names, if its wait is still open.
///
/// Neo: the dispatcher's glue from a frame call to its cursor; no script sees it.
pub(crate) fn release_frame_navigation(
    cursors: &Cursors<FrameCursor>,
    params: &NeoReleaseNavigationParams,
) -> NeoNavigationResult {
    if let Some(cursor) = cursors.remove(params.cursor) {
        cursor.end();
    }
    NeoNavigationResult { response: None }
}

/// Neo: a finish naming a navigation its cursor never answered or no longer holds.
fn unanswered(index: u64) -> PlaywrightError {
    PlaywrightError::error(format!(
        "neo-runtime: navigation {index} of this wait was never answered or is no longer held"
    ))
}

/// Neo: a `Page.getNavigationHistory` reply Chromium never sends.
fn unreadable_history(error: &serde_json::Error) -> PlaywrightError {
    PlaywrightError::error(format!("Page.getNavigationHistory answered: {error}"))
}

/// Neo: a broken invariant (a poisoned lock) as an error instead of a panic, `crates/NEO_STYLE.md`.
fn cursors_poisoned() -> PlaywrightError {
    PlaywrightError::error("neo-runtime: the run's navigation waits cannot be read")
}

/// Playwright's server `Frame.waitForNavigation` from its subscription on: the first navigation
/// event `accept` takes, where a failed navigation rejects first.
///
/// Playwright: `server/frames.ts` `Frame.waitForNavigation`: `if (requiresNewDocument && !event.newDocument)`
async fn next_navigation<O: Observation>(
    events: &mut O,
    progress: &Progress,
    mut accept: impl FnMut(&Navigation) -> Result<bool, PlaywrightError>,
) -> Result<Navigation, CallError> {
    loop {
        let Some(navigation) = progress
            .race(events.next())
            .await?
            .map_err(navigate::page_error)?
        else {
            continue;
        };
        // Playwright: `server/frames.ts` `Frame.waitForNavigation`: `// Any failed navigation results in a rejection.`
        if let Some(error) = navigation.error {
            return Err(PlaywrightError::error(error).into());
        }
        if accept(&navigation)? {
            progress.log(format!("  navigated to \"{}\"", navigation.url));
            return Ok(navigation);
        }
    }
}

/// Waits until `wait_until` fired for the document a navigation left, given its lifecycle when
/// it happened: at once if it already had, else on the next document's.
///
/// Playwright: `server/frames.ts` `Frame.waitForNavigation`: `if (!this._firedLifecycleEvents.has(waitUntil))`
async fn settle<O: Observation>(
    events: &mut O,
    since: Lifecycle,
    wait_until: LifecycleEvent,
    progress: &Progress,
) -> Result<(), CallError> {
    loop {
        if waits::lifecycle_reached(since, events.state().lifecycle, wait_until) {
            return Ok(());
        }
        progress
            .race(events.next())
            .await?
            .map_err(navigate::page_error)?;
    }
}

/// The navigation's `Response`: `request._finalRequest().response()` of its new document, `null`
/// for a same-document navigation or a document without a request Neo saw.
///
/// Deviation: `server/frames.ts` `Frame.waitForNavigation`: `const request = navigationEvent.newDocument ? navigationEvent.newDocument.request : undefined;`
/// Playwright records every request; Neo starts recording a session's requests with its first
/// navigation wait or goto, so a document whose request started before that answers `null`.
async fn response_of<T: NavigationTab>(
    tab: &T,
    navigation: &Navigation,
    lags: u64,
    progress: &Progress,
) -> Result<Option<ResponseChannel>, CallError> {
    match &navigation.document {
        Some(document) if navigation.request => Ok(progress
            .race(tab.navigation_response(document, lags))
            .await??),
        _ => Ok(None),
    }
}

/// `Page.getNavigationHistory`'s reply, as `CRPage._go` reads it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NavigationHistory {
    current_index: i64,
    entries: Vec<HistoryEntry>,
}

#[derive(Deserialize)]
struct HistoryEntry {
    id: i64,
}

/// Playwright: `server/chromium/crPage.ts` `CRPage._go`: `const history = await this._mainFrameSession._client.send('Page.getNavigationHistory');`
impl HistoryTab for FrameTab {
    async fn reload(&self) -> Result<(), PlaywrightError> {
        self.send("Page.reload", json!({}))
            .await
            .map(|_| ())
            .map_err(|error| history_error("Page.reload", &error))
    }

    async fn go(&self, delta: i64) -> Result<bool, PlaywrightError> {
        let reply = self
            .send("Page.getNavigationHistory", json!({}))
            .await
            .map_err(|error| history_error("Page.getNavigationHistory", &error))?;
        let history: NavigationHistory =
            serde_json::from_value(reply).map_err(|error| unreadable_history(&error))?;
        let Some(entry) = history
            .current_index
            .checked_add(delta)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| history.entries.get(index))
        else {
            return Ok(false);
        };
        self.send(
            "Page.navigateToHistoryEntry",
            json!({ "entryId": entry.id }),
        )
        .await
        .map_err(|error| history_error("Page.navigateToHistoryEntry", &error))?;
        Ok(true)
    }
}

/// A CDP failure of a history step as the script sees it: the page's closure, or Playwright's
/// `Protocol error (<method>): <message>` for the browser's refusal.
///
/// Playwright: `server/protocolError.ts` `ProtocolError.setMessage`: `setMessage(message: string) {`
fn history_error(method: &str, error: &CdpError) -> PlaywrightError {
    match error {
        CdpError::Protocol { message, .. } => {
            PlaywrightError::error(format!("Protocol error ({method}): {message}"))
        }
        error if playwright_injected_methods::classify(error) == PageError::Closed => {
            PlaywrightError::target_closed()
        }
        error => PlaywrightError::error(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;
    use std::future;
    use std::ops::Deref;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use serde_json::json;
    use tokio::sync::{broadcast, watch};
    use tokio::time;
    use tokio_util::sync::CancellationToken;
    use tokio_util::task::TaskTracker;

    use super::*;
    use crate::deadline::Deadline;
    use crate::host::RunEmitter;
    use crate::playwright_methods::navigate::{NavigateReply, NavigationState};
    use crate::progress::{CallEntry, CallStart, ErrorName, RunScope};

    const START: &str = "https://a.test/start";

    /// What the page does, as the watcher reports it.
    #[derive(Clone, Copy)]
    enum Event {
        /// A new document with or without a request Neo saw start.
        Commit(&'static str, &'static str, bool),
        SameDocument(&'static str),
        Load,
        /// A document restored from the back-forward cache: loaded, without a request.
        Restore(&'static str),
        Failed(&'static str),
    }

    /// A scripted page shared by a test and the cursor that owns a clone: its navigation feed,
    /// its history (current index, entries), the events its CDP step causes, and the network
    /// log's answer.
    #[derive(Clone)]
    struct FakeTab(Arc<Page>);

    struct Page {
        state: watch::Sender<NavigationState>,
        navigations: broadcast::Sender<Navigation>,
        on_step: Mutex<Vec<Event>>,
        history: Mutex<(i64, i64)>,
        response: Result<Option<ResponseChannel>, PlaywrightError>,
        calls: Mutex<Vec<String>>,
    }

    impl Deref for FakeTab {
        type Target = Page;

        fn deref(&self) -> &Page {
            &self.0
        }
    }

    impl FakeTab {
        fn new(steps: impl IntoIterator<Item = Event>) -> Self {
            let (state, _) = watch::channel(NavigationState {
                url: Some(START.to_string()),
                lifecycle: Lifecycle {
                    dom_content_loaded: true,
                    load: true,
                    dom_content_loaded_count: 1,
                    load_count: 1,
                    ..Lifecycle::default()
                },
                ..NavigationState::default()
            });
            Self(Arc::new(Page {
                state,
                navigations: broadcast::channel(16).0,
                on_step: Mutex::new(steps.into_iter().collect()),
                history: Mutex::new((1, 3)),
                response: Ok(Some(ResponseChannel {
                    guid: "response@T:D:0".to_string(),
                })),
                calls: Mutex::default(),
            }))
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("calls").clone()
        }

        fn record(&self, call: impl Into<String>) {
            self.calls.lock().expect("calls").push(call.into());
        }

        /// Plays the events the CDP step causes on a task of their own, as the watcher would.
        fn play(&self) {
            let events: Vec<Event> = self.on_step.lock().expect("steps").drain(..).collect();
            let page = self.clone();
            tokio::spawn(async move {
                for event in events {
                    time::sleep(Duration::from_millis(5)).await;
                    page.emit(event);
                }
            });
        }

        fn emit(&self, event: Event) {
            let mut navigation = None;
            self.state.send_modify(|state| {
                let commit = |state: &mut NavigationState, document: &str, url: &str| Navigation {
                    document: Some(document.to_string()),
                    url: url.to_string(),
                    lifecycle: state.lifecycle,
                    request: false,
                    error: None,
                };
                match event {
                    Event::Commit(document, url, request) => {
                        state.url = Some(url.to_string());
                        state.lifecycle.dom_content_loaded = false;
                        state.lifecycle.load = false;
                        navigation = Some(Navigation {
                            request,
                            ..commit(state, document, url)
                        });
                    }
                    Event::Restore(document) => {
                        state.url = Some(START.to_string());
                        state.lifecycle.dom_content_loaded_count += 1;
                        state.lifecycle.load_count += 1;
                        navigation = Some(commit(state, document, START));
                    }
                    Event::SameDocument(url) => {
                        state.url = Some(url.to_string());
                        navigation = Some(Navigation {
                            document: None,
                            ..commit(state, "", url)
                        });
                    }
                    Event::Load => {
                        state.lifecycle.dom_content_loaded = true;
                        state.lifecycle.load = true;
                        state.lifecycle.dom_content_loaded_count += 1;
                        state.lifecycle.load_count += 1;
                    }
                    Event::Failed(error) => {
                        let url = state.url.clone().unwrap_or_default();
                        navigation = Some(Navigation {
                            error: Some(error.to_string()),
                            ..commit(state, "FAILED", &url)
                        });
                    }
                }
            });
            if let Some(navigation) = navigation {
                let _ = self.navigations.send(navigation);
            }
        }
    }

    struct Events {
        state: watch::Receiver<NavigationState>,
        navigations: broadcast::Receiver<Navigation>,
    }

    impl Observation for Events {
        fn state(&self) -> NavigationState {
            self.state.borrow().clone()
        }

        async fn next(&mut self) -> Result<Option<Navigation>, PageError> {
            tokio::select! {
                biased;
                navigation = self.navigations.recv() => Ok(navigation.ok()),
                changed = self.state.changed() => {
                    changed.map(|()| None).map_err(|_| PageError::Closed)
                }
            }
        }
    }

    impl NavigationTab for FakeTab {
        type Observation = Events;

        fn prepare(&self) -> impl Future<Output = Result<(), PageError>> {
            future::ready(Ok(()))
        }

        fn track_network(&self) -> impl Future<Output = Result<u64, PageError>> {
            self.record("track");
            future::ready(Ok(3))
        }

        fn navigation_response(
            &self,
            document: &str,
            lags: u64,
        ) -> impl Future<Output = Result<Option<ResponseChannel>, PlaywrightError>> {
            self.record(format!("response of {document} since {lags}"));
            future::ready(self.response.clone())
        }

        fn observe(&self) -> Result<Events, PageError> {
            self.record("observe");
            Ok(Events {
                state: self.state.subscribe(),
                navigations: self.navigations.subscribe(),
            })
        }

        fn navigate(
            &self,
            _: &str,
            _: Option<&str>,
        ) -> impl Future<Output = Result<NavigateReply, PlaywrightError>> {
            future::ready(Err(PlaywrightError::error(
                "history never sends Page.navigate",
            )))
        }

        fn read_title(&self) -> impl Future<Output = Result<String, PageError>> {
            future::ready(Err(PageError::Contract(
                "history never reads a title".to_string(),
            )))
        }
    }

    impl HistoryTab for FakeTab {
        fn reload(&self) -> impl Future<Output = Result<(), PlaywrightError>> {
            self.record("reload");
            self.play();
            future::ready(Ok(()))
        }

        fn go(&self, delta: i64) -> impl Future<Output = Result<bool, PlaywrightError>> {
            self.record(format!("go {delta}"));
            let (current, entries) = *self.history.lock().expect("history");
            let moved = (0..entries).contains(&(current + delta));
            if moved {
                self.play();
            }
            future::ready(Ok(moved))
        }
    }

    fn progress(timeout: f64) -> Progress {
        let entry = Arc::new(CallEntry::new(CallStart {
            call_id: 1,
            guid: "page@T".to_string(),
            method: "goBack".to_string(),
            title: Some("Go back"),
            params: json!({}),
            location: None,
            input: false,
            page: None,
            snapshot: false,
        }));
        let run = RunScope::new(
            Deadline::new(Duration::from_mins(1), CancellationToken::new()),
            TaskTracker::new(),
            RunEmitter::default(),
        );
        Progress::new(entry, timeout, run, CancellationToken::new())
    }

    fn history(step: HistoryStep, wait_until: LifecycleEvent) -> HistoryOptions {
        HistoryOptions { step, wait_until }
    }

    fn url_wait(glob: &str) -> WaitForNavigationOptions {
        WaitForNavigationOptions {
            matcher: UrlMatcher::new(&SerializedUrlMatch {
                glob: Some(glob.to_string()),
                ..SerializedUrlMatch::default()
            })
            .expect("a valid glob"),
            wait_until: LifecycleEvent::Load,
        }
    }

    fn rejection<T: Debug>(result: Result<T, CallError>) -> PlaywrightError {
        match result {
            Err(CallError::Playwright(error)) => error,
            other => panic!("expected a browser error, got {other:?}"),
        }
    }

    fn response(guid: &str) -> ResponseChannel {
        ResponseChannel {
            guid: guid.to_string(),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_reload_answers_its_new_documents_response_after_load() {
        let tab = FakeTab::new([
            Event::SameDocument("https://a.test/start#stray"),
            Event::Commit("D", START, true),
            Event::Load,
        ]);
        let answer = navigate(
            &tab,
            &history(HistoryStep::Reload, LifecycleEvent::Load),
            &progress(1_000.0),
        )
        .await;
        assert_eq!(answer, Ok(Some(response("response@T:D:0"))));
        assert_eq!(
            tab.calls(),
            ["observe", "track", "reload", "response of D since 3"],
            "subscribed and recording before the reload; a stray same-document commit is not it"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_going_nowhere_answers_null_without_waiting() {
        for (current, step) in [(0, HistoryStep::Back), (2, HistoryStep::Forward)] {
            let tab = FakeTab::new([]);
            *tab.history.lock().expect("history") = (current, 3);
            let answer = navigate(
                &tab,
                &history(step, LifecycleEvent::Load),
                &progress(1_000.0),
            )
            .await;
            assert_eq!(answer, Ok(None), "{step:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_history_steps_answer_by_the_document_they_reach() {
        for (steps, expected) in [
            (
                vec![Event::Commit("D", "https://a.test/back", true), Event::Load],
                Some(response("response@T:D:0")),
            ),
            // A same-document entry: `null`, and the loaded document needs no wait.
            (vec![Event::SameDocument("https://a.test/start#back")], None),
            // A back-forward cache restore: loaded already, no request, `null`.
            (vec![Event::Restore("CACHED")], None),
            // A new document whose request Neo never saw start: `null`.
            (
                vec![
                    Event::Commit("E", "https://a.test/early", false),
                    Event::Load,
                ],
                None,
            ),
        ] {
            let tab = FakeTab::new(steps);
            let answer = navigate(
                &tab,
                &history(HistoryStep::Back, LifecycleEvent::Load),
                &progress(1_000.0),
            )
            .await;
            assert_eq!(answer, Ok(expected.clone()));
            assert_eq!(
                tab.calls()
                    .iter()
                    .any(|call| call.starts_with("response of")),
                expected.is_some(),
                "only a document with a request is looked up"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_commit_resolves_before_load_and_load_waits_for_it() {
        for (wait_until, done) in [
            (LifecycleEvent::Commit, true),
            (LifecycleEvent::Load, false),
        ] {
            let tab = FakeTab::new([Event::Commit("D", "https://a.test/next", true)]);
            let answer = navigate(
                &tab,
                &history(HistoryStep::Forward, wait_until),
                &progress(100.0),
            )
            .await;
            if done {
                assert_eq!(answer, Ok(Some(response("response@T:D:0"))));
            } else {
                assert_eq!(rejection(answer).name, ErrorName::TimeoutError);
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_failed_navigation_rejects_with_its_error() {
        let tab = FakeTab::new([Event::Failed("net::ERR_EMPTY_RESPONSE")]);
        let error = rejection(
            navigate(
                &tab,
                &history(HistoryStep::Reload, LifecycleEvent::Load),
                &progress(1_000.0),
            )
            .await,
        );
        assert_eq!(
            (error.name, error.message.as_str()),
            (ErrorName::Error, "net::ERR_EMPTY_RESPONSE")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_url_wait_skips_navigations_that_do_not_match() {
        let tab = FakeTab::new([]);
        let options = url_wait("**/done");
        let progress = progress(1_000.0);
        let wait = wait_for_navigation(&tab, &options, &progress);
        let page = async {
            time::sleep(Duration::from_millis(1)).await;
            tab.emit(Event::SameDocument("https://a.test/start#not-yet"));
            tab.emit(Event::Commit("D", "https://a.test/done", true));
            tab.emit(Event::Load);
        };
        let (answer, ()) = tokio::join!(wait, page);
        assert_eq!(answer, Ok(Some(response("response@T:D:0"))));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_same_document_wait_answers_null_and_no_navigation_times_out() {
        let tab = FakeTab::new([]);
        let options = url_wait("");
        let progress = self::progress(100.0);
        let wait = wait_for_navigation(&tab, &options, &progress);
        let page = async {
            time::sleep(Duration::from_millis(1)).await;
            tab.emit(Event::SameDocument("https://a.test/start#hash"));
        };
        let (answer, ()) = tokio::join!(wait, page);
        assert_eq!(answer, Ok(None));
        let idle = FakeTab::new([]);
        let progress = self::progress(100.0);
        let error = rejection(wait_for_navigation(&idle, &options, &progress).await);
        assert_eq!(error.name, ErrorName::TimeoutError);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_cursor_keeps_navigations_queued_between_its_calls() {
        let tab = FakeTab::new([]);
        let events = tab.observe().expect("observe");
        let progress = progress(1_000.0);
        let cursor = Cursor::open(tab.clone(), events, true, &progress)
            .await
            .expect("open");
        // Both happen before the realm asks for the second one.
        tab.emit(Event::SameDocument("https://a.test/start#a"));
        tab.emit(Event::Commit("D", "https://a.test/b", true));
        assert_eq!(
            cursor.next(&progress).await,
            Ok(Some((0, "https://a.test/start#a".to_string())))
        );
        assert_eq!(
            cursor.next(&progress).await,
            Ok(Some((1, "https://a.test/b".to_string())))
        );
        tab.emit(Event::Load);
        assert_eq!(
            cursor.finish(1, LifecycleEvent::Load, &progress).await,
            Ok(Some(response("response@T:D:0")))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_finish_stops_a_waiting_next_and_settles_the_navigation_it_names() {
        let tab = FakeTab::new([]);
        let events = tab.observe().expect("observe");
        let progress = progress(1_000.0);
        let cursor = Cursor::open(tab.clone(), events, true, &progress)
            .await
            .expect("open");
        tab.emit(Event::Commit("D", "https://a.test/first", true));
        tab.emit(Event::SameDocument("https://a.test/first#later"));
        assert_eq!(
            cursor.next(&progress).await,
            Ok(Some((0, "https://a.test/first".to_string())))
        );
        assert_eq!(
            cursor.next(&progress).await,
            Ok(Some((1, "https://a.test/first#later".to_string())))
        );
        // The realm keeps a next waiting while its predicates run; the first navigation's
        // predicate accepts later, so the finish names navigation 0, not the latest.
        let waiting = cursor.next(&progress);
        let finished = async {
            time::sleep(Duration::from_millis(5)).await;
            tab.emit(Event::Load);
            cursor.finish(0, LifecycleEvent::Load, &progress).await
        };
        let (waiting, finished) = tokio::join!(waiting, finished);
        assert_eq!(waiting, Ok(None), "the finish ended the waiting next");
        assert_eq!(finished, Ok(Some(response("response@T:D:0"))));
        assert_eq!(
            cursor.next(&progress).await,
            Ok(None),
            "an ended wait answers no further navigation"
        );
        assert!(
            rejection(cursor.finish(7, LifecycleEvent::Load, &progress).await)
                .message
                .contains("never answered")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_url_only_cursor_answers_nothing_and_a_failure_rejects_its_step() {
        let tab = FakeTab::new([]);
        let progress = progress(1_000.0);
        let events = tab.observe().expect("observe");
        let cursor = Cursor::open(tab.clone(), events, false, &progress)
            .await
            .expect("open");
        tab.emit(Event::Commit("D", "https://a.test/b", true));
        cursor.next(&progress).await.expect("a navigation");
        tab.emit(Event::Load);
        assert_eq!(
            cursor.finish(0, LifecycleEvent::Load, &progress).await,
            Ok(None)
        );
        let events = tab.observe().expect("observe");
        let failing = Cursor::open(tab.clone(), events, true, &progress)
            .await
            .expect("open");
        tab.emit(Event::Failed("net::ERR_ABORTED; maybe frame was detached?"));
        assert_eq!(
            rejection(failing.next(&progress).await).message,
            "net::ERR_ABORTED; maybe frame was detached?"
        );
    }

    #[test]
    fn tests_that_cursors_refuse_one_past_the_cap_and_forget_an_ended_one() {
        let cursors = Cursors::<u64>::default();
        let ids: Vec<u64> = (0..MAX_CURSORS)
            .map(|value| cursors.insert(value as u64).expect("under the cap").0)
            .collect();
        assert!(
            cursors.insert(99).is_err(),
            "a live wait is never displaced"
        );
        assert_eq!(cursors.get(ids[0]).map(|cursor| *cursor), Ok(0));
        assert!(cursors.remove(ids[0]).is_some());
        assert!(cursors.get(ids[0]).is_err(), "an ended wait is gone");
        assert!(cursors.insert(99).is_ok(), "its slot is free again");
    }

    #[test]
    fn tests_that_a_claimed_wait_stays_live_until_its_claim_drops() {
        let cursors = Arc::new(Cursors::<u64>::default());
        let ids: Vec<u64> = (0..MAX_CURSORS)
            .map(|value| cursors.insert(value as u64).expect("under the cap").0)
            .collect();
        let finishing = cursors
            .begin_ending(ids[0])
            .expect("an open cursor can be claimed");
        assert!(
            cursors.begin_ending(ids[0]).is_none(),
            "one claim per cursor"
        );
        let releasing = cursors
            .begin_ending(ids[1])
            .expect("another open cursor can be claimed");
        assert!(
            cursors.insert(99).is_err(),
            "a claimed cursor still open counts once"
        );
        assert!(cursors.remove(ids[0]).is_some(), "its finish took it out");
        assert!(
            cursors.insert(99).is_err(),
            "a finishing wait is still live"
        );
        assert!(
            cursors.begin_ending(ids[0]).is_none(),
            "a cursor no longer open cannot be claimed"
        );
        drop(finishing);
        let (spare, _) = cursors.insert(99).expect("a finished wait's place is free");
        assert!(cursors.insert(100).is_err(), "the cap counts the new wait");
        drop(releasing);
        assert!(
            cursors.insert(100).is_err(),
            "a claim dropped before its release ran leaves the cursor open and live"
        );
        assert!(cursors.remove(spare).is_some());
        assert!(cursors.insert(100).is_ok());
    }

    #[test]
    fn tests_that_networkidle_is_a_wait_like_the_others() {
        let options = HistoryOptions::back(&PageGoBackParams {
            wait_until: Some(LifecycleEvent::Networkidle),
        });
        assert_eq!(options.wait_until, LifecycleEvent::Networkidle);
    }
}
