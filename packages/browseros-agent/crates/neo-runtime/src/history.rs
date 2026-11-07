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
//!  waitForNavigation({url: predicate})     ─ neo:nextNavigation × n: one navigation per call,
//!  waitForURL(predicate)                     whose URL the realm tests; then neo:finishNavigation:
//!                                            waitUntil → Response | null
//! ```
//!
//! Every wait subscribes to its frame's navigation events before it prepares the page, enables
//! the network log or acts, so no navigation it should see is lost, and every step races the
//! call's [`Progress`], so its deadline and the page's closure end it. A navigation's `Response`
//! is the network log's answer for its document, as `goto`'s (`network.rs`); it is `null` for a
//! same-document navigation, and for a document whose request the watcher never saw start: a
//! back-forward cache restore, or a request that started before this session's `Network.enable`.
//!
//! A URL predicate cannot cross into Rust, so its wait is a cursor: the first `neo:nextNavigation`
//! subscribes and parks the subscription in the run's [`Cursors`] between calls, so navigations
//! that happen while the realm runs the predicate stay queued. A cursor ends with its
//! `neo:finishNavigation`, a failed step (its timeout included), or the run.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use browseros_cdp::CdpError;
use neo_runtime_spec::serve::{
    LifecycleEvent, PageGoBackParams, PageGoForwardParams, PageReloadParams, ResponseChannel,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::click::PageError;
use crate::navigate::{self, Lifecycle, Navigation, NavigationTab, Observation};
use crate::page::FrameTab;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::url_match::{SerializedUrlMatch, UrlMatcher};
use crate::waits::{self, WaitObservation, WaitTab};
use crate::world;

/// Deviation: `client/frame.ts` `Frame.waitForNavigation` the most URL-predicate waits one run
/// may hold open between their calls; Playwright's waiter has no cap. A wait whose predicate threw
/// never finishes, so past the cap the oldest idle cursor is dropped, and its next call rejects.
const MAX_CURSORS: usize = 64;

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
    pub(crate) fn reload(params: &PageReloadParams) -> Result<Self, PlaywrightError> {
        Self::new(HistoryStep::Reload, params.wait_until)
    }

    pub(crate) fn back(params: &PageGoBackParams) -> Result<Self, PlaywrightError> {
        Self::new(HistoryStep::Back, params.wait_until)
    }

    pub(crate) fn forward(params: &PageGoForwardParams) -> Result<Self, PlaywrightError> {
        Self::new(HistoryStep::Forward, params.wait_until)
    }

    // Deviation: `server/page.ts` `Page.reload` Neo refuses `waitUntil: 'networkidle'`, as its
    // goto does: it records no subresource requests to tell network idleness by.
    fn new(step: HistoryStep, wait_until: Option<LifecycleEvent>) -> Result<Self, PlaywrightError> {
        Ok(Self {
            step,
            wait_until: waits::supported_lifecycle(wait_until, "waitUntil")?,
        })
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
/// `url` (or none). Keep in sync with neo-sdk's `NeoFrameCommands` and the dispatch tests.
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
            wait_until: waits::supported_lifecycle(params.wait_until, "waitUntil")?,
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
    let mut events = tab.observe().map_err(navigate::page_error)?;
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
/// answers the next navigation it observes. `response` says what its finish answers: the
/// navigation's `Response` (`waitForNavigation`) or nothing (`waitForURL`).
///
/// Deviation: `client/frame.ts` `Frame.waitForNavigation` Playwright tests a URL predicate inside
/// one API call, on its client's `navigated` events; Neo makes one call per navigation the
/// predicate tests, each in the run's call records.
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

/// `neo:nextNavigation`'s reply.
#[derive(Debug, Serialize)]
pub(crate) struct NeoNextNavigationResult {
    pub(crate) cursor: u64,
    pub(crate) url: String,
}

/// `neo:finishNavigation`: waits for `waitUntil` since the cursor's last navigation and answers
/// as `neo:waitForNavigation` does; the cursor ends.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NeoFinishNavigationParams {
    pub(crate) cursor: u64,
    pub(crate) wait_until: Option<LifecycleEvent>,
}

/// A decoded `neo:finishNavigation`.
#[derive(Debug)]
pub(crate) struct FinishNavigationOptions {
    pub(crate) cursor: u64,
    wait_until: LifecycleEvent,
}

/// Neo: decodes Neo's own `neo:finishNavigation`; Playwright has no such command.
impl FinishNavigationOptions {
    pub(crate) fn from_params(params: &NeoFinishNavigationParams) -> Result<Self, PlaywrightError> {
        Ok(Self {
            cursor: params.cursor,
            wait_until: waits::supported_lifecycle(params.wait_until, "waitUntil")?,
        })
    }
}

/// One predicate wait's subscription and what its finish needs, kept between its calls.
pub(crate) struct Cursor<T, O> {
    tab: T,
    events: O,
    /// The network log's lag count when the wait began, for its response lookup.
    lags: u64,
    respond: bool,
    /// The navigation the realm saw last, which a finish settles.
    last: Option<Navigation>,
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
            events,
            lags,
            respond,
            last: None,
        })
    }

    /// The URL of the next navigation; a failed navigation rejects.
    pub(crate) async fn next(&mut self, progress: &Progress) -> Result<String, CallError> {
        let navigation = next_navigation(&mut self.events, progress, |_| Ok(true)).await?;
        let url = navigation.url.clone();
        self.last = Some(navigation);
        Ok(url)
    }

    /// `waitUntil` since the last navigation, then its `Response` if this wait answers one.
    pub(crate) async fn finish(
        mut self,
        wait_until: LifecycleEvent,
        progress: &Progress,
    ) -> Result<Option<ResponseChannel>, CallError> {
        let Some(navigation) = self.last.take() else {
            return Err(finish_before_next().into());
        };
        settle(&mut self.events, navigation.lifecycle, wait_until, progress).await?;
        if !self.respond {
            return Ok(None);
        }
        response_of(&self.tab, &navigation, self.lags, progress).await
    }
}

/// The run's open cursors by id, shared by its call tasks; a cursor is out of the map while a
/// call uses it, so a second call naming it meanwhile finds nothing.
pub(crate) struct Cursors<C> {
    next: AtomicU64,
    open: Mutex<BTreeMap<u64, C>>,
}

/// Neo: an empty cursor table at run start; no script sees it.
impl<C> Default for Cursors<C> {
    fn default() -> Self {
        Self {
            next: AtomicU64::new(1),
            open: Mutex::new(BTreeMap::new()),
        }
    }
}

/// Neo: the run's cursor table between a predicate wait's calls; its cap is `MAX_CURSORS`'s.
impl<C> Cursors<C> {
    /// A fresh cursor id.
    pub(crate) fn mint(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    /// Takes cursor `id` out for one call.
    pub(crate) fn take(&self, id: u64) -> Result<C, PlaywrightError> {
        self.open
            .lock()
            .map_err(|_| cursors_poisoned())?
            .remove(&id)
            .ok_or_else(|| {
                PlaywrightError::error(format!(
                    "neo-runtime: navigation wait {id} is not open: it finished, failed, or is \
                     already waiting"
                ))
            })
    }

    /// Parks cursor `id` until its next call, dropping the oldest idle one past [`MAX_CURSORS`].
    pub(crate) fn put(&self, id: u64, cursor: C) -> Result<(), PlaywrightError> {
        let mut open = self.open.lock().map_err(|_| cursors_poisoned())?;
        open.insert(id, cursor);
        while open.len() > MAX_CURSORS {
            open.pop_first();
        }
        Ok(())
    }
}

/// `neo:nextNavigation` on `tab`'s frame: opens a cursor there (subscribed before the page is
/// prepared) or continues the one `params` names, and parks it again once a navigation answered.
///
/// Neo: the dispatcher's glue from a frame call to its cursor; what a script sees is `Cursor`'s.
pub(crate) async fn next_frame_navigation(
    cursors: &Cursors<FrameCursor>,
    tab: FrameTab,
    params: &NeoNextNavigationParams,
    progress: &Progress,
) -> Result<NeoNextNavigationResult, CallError> {
    let (id, mut cursor) = if let Some(id) = params.cursor {
        (id, cursors.take(id)?)
    } else {
        let events = WaitTab::observe(&tab).map_err(navigate::page_error)?;
        waits::prepare_tab(&tab, progress).await?;
        let cursor = Cursor::open(tab, events, params.response, progress).await?;
        (cursors.mint(), cursor)
    };
    let url = cursor.next(progress).await?;
    cursors.put(id, cursor)?;
    Ok(NeoNextNavigationResult { cursor: id, url })
}

/// `neo:finishNavigation`: ends the cursor `options` names.
///
/// Neo: the dispatcher's glue from a frame call to its cursor; what a script sees is `Cursor`'s.
pub(crate) async fn finish_frame_navigation(
    cursors: &Cursors<FrameCursor>,
    options: &FinishNavigationOptions,
    progress: &Progress,
) -> Result<NeoNavigationResult, CallError> {
    let cursor = cursors.take(options.cursor)?;
    let response = cursor.finish(options.wait_until, progress).await?;
    Ok(NeoNavigationResult { response })
}

/// Neo: an SDK that finishes a cursor before any navigation answered; no script can.
fn finish_before_next() -> PlaywrightError {
    PlaywrightError::error(
        "neo-runtime: neo:finishNavigation before any neo:nextNavigation answered",
    )
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
        error if world::classify(error) == PageError::Closed => PlaywrightError::target_closed(),
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
    use crate::navigate::{NavigateReply, NavigationState};
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
        }));
        let run = RunScope::new(
            Deadline::new(Duration::from_mins(1), CancellationToken::new()),
            TaskTracker::new(),
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
        let mut cursor = Cursor::open(tab.clone(), events, true, &progress)
            .await
            .expect("open");
        // Both happen before the realm asks for the second one.
        tab.emit(Event::SameDocument("https://a.test/start#a"));
        tab.emit(Event::Commit("D", "https://a.test/b", true));
        assert_eq!(
            cursor.next(&progress).await,
            Ok("https://a.test/start#a".to_string())
        );
        assert_eq!(
            cursor.next(&progress).await,
            Ok("https://a.test/b".to_string())
        );
        tab.emit(Event::Load);
        assert_eq!(
            cursor.finish(LifecycleEvent::Load, &progress).await,
            Ok(Some(response("response@T:D:0")))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_url_only_cursor_answers_nothing_and_a_failure_rejects_its_step() {
        let tab = FakeTab::new([]);
        let progress = progress(1_000.0);
        let events = tab.observe().expect("observe");
        let mut cursor = Cursor::open(tab.clone(), events, false, &progress)
            .await
            .expect("open");
        tab.emit(Event::Commit("D", "https://a.test/b", true));
        cursor.next(&progress).await.expect("a navigation");
        tab.emit(Event::Load);
        assert_eq!(
            cursor.finish(LifecycleEvent::Load, &progress).await,
            Ok(None)
        );
        let events = tab.observe().expect("observe");
        let mut failing = Cursor::open(tab.clone(), events, true, &progress)
            .await
            .expect("open");
        tab.emit(Event::Failed("net::ERR_ABORTED; maybe frame was detached?"));
        assert_eq!(
            rejection(failing.next(&progress).await).message,
            "net::ERR_ABORTED; maybe frame was detached?"
        );
    }

    #[test]
    fn tests_that_cursors_are_taken_once_and_the_oldest_idle_one_goes_past_the_cap() {
        let cursors = Cursors::<u64>::default();
        let ids: Vec<u64> = (0..=MAX_CURSORS).map(|_| cursors.mint()).collect();
        for id in &ids {
            cursors.put(*id, *id).expect("put");
        }
        assert!(cursors.take(ids[0]).is_err(), "evicted past the cap");
        assert_eq!(cursors.take(ids[1]), Ok(ids[1]));
        assert!(cursors.take(ids[1]).is_err(), "out while in use");
    }

    #[test]
    fn tests_that_networkidle_is_refused_before_any_step() {
        let error = HistoryOptions::back(&PageGoBackParams {
            wait_until: Some(LifecycleEvent::Networkidle),
        })
        .unwrap_err();
        assert!(error.message.contains("networkidle"));
    }
}
