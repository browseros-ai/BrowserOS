//! Neo: the document-wide frame calls of slice B3, each in the receiving frame's document:
//! `Frame.content`, `Frame.setContent` and `Frame.resolveSelector` (`locator.normalize`).
//! `Frame.waitForTimeout` is a wait (`waits.rs`); `Frame.ariaSnapshotJSON` is a read
//! (`reads.rs`).
//!
//! ```text
//!  Frame.content {includeShadow?}      prepare → documentContent in the frame's utility world
//!  Frame.setContent {html, waitUntil?} prepare → subscribe to the frame session's events
//!                                        → writeContent (document.open, write, close)
//!                                        → init, then DOMContentLoaded or load
//!  Frame.resolveSelector {selector}    locate (not strict) → generateSelector for the match
//!                                        → generateOwnerSelector for each iframe on the way
//!                                        → joined by enter-frame hops, main frame first
//! ```

use neo_playwright_protocol::injected::{
    InjectedDocumentContentParams, InjectedGenerateOwnerSelectorParams,
    InjectedGenerateSelectorParams, InjectedWriteContentParams,
};
use neo_playwright_protocol::serve::{
    FrameContentResult, FrameResolveSelectorResult, FrameSetContentParams, LifecycleEvent,
};

use std::collections::{BTreeSet, HashMap};
use std::pin::pin;
use std::sync::{Arc, Mutex};

use browseros_cdp::SessionId;
use tokio::sync::Notify;
use tokio::sync::broadcast::error::RecvError;

use crate::browser_representations::page::FrameTab;
use crate::chromium_bridge::cdp;
use crate::chromium_bridge::frame_selector;
use crate::playwright_methods::navigate::{self, Observation};
use crate::playwright_methods::tab::{PageError, Tab};
use crate::playwright_methods::waits::{WaitObservation, WaitTab};
use crate::progress::{CallError, PlaywrightError, Progress};

/// Neo: Playwright's `Frame._content` error, word for word, for a document that changed under
/// the read.
const NAVIGATING: &str =
    "Unable to retrieve content because the page is navigating and changing the content.";

/// Deviation: `server/frames.ts` `Frame.setContent`: `const lifecyclePromise = progress.race(tagPromise).then(() => this.waitForLoadState(progress, waitUntil));`
/// Playwright waits for an out-of-process frame's `networkidle` as for any frame's. Neo's idle
/// tracker follows the tab's own session only, which such a frame left (`network_idle.rs`), so
/// the wait could only time out: it is refused before the write.
const OOPIF_NETWORKIDLE: &str =
    "Neo does not support waitUntil: \"networkidle\" in an out-of-process frame yet";

/// Neo: the hop between two frame chunks, as Playwright's `resolveSelector` joins them.
const ENTER_FRAME: &str = " >> internal:control=enter-frame >> ";

/// Neo: one `Frame.setContent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SetContentOptions {
    pub(crate) html: String,
    pub(crate) wait_until: LifecycleEvent,
}

/// Neo: the run's `setContent` writes into each frame, in the order the calls arrived. Playwright
/// sends each write's `document.open()` in call order, so of overlapping writes the later one's
/// document stays; Neo's calls run as tasks, so each waits until every earlier write into its
/// frame that is still live (waiting or writing) is gone. The order is taken when the call
/// arrives (`Dispatcher::call`, synchronous); a call that stops while it waits just leaves it.
#[derive(Debug, Default)]
pub(crate) struct WriteOrder {
    /// Per frame guid, its writes.
    frames: Mutex<HashMap<String, Arc<FrameWrites>>>,
}

/// Neo: one frame's live writes, by the ticket each took at arrival, and the wake for a change.
#[derive(Debug, Default)]
struct FrameWrites {
    live: Mutex<Tickets>,
    changed: Notify,
}

#[derive(Debug, Default)]
struct Tickets {
    next: u64,
    live: BTreeSet<u64>,
}

/// Neo: one write's place in its frame's order. It may write once no earlier ticket is live;
/// dropping it (written, failed, stopped, or cancelled while it waits) leaves the order, so a
/// later write still waits for every earlier one that is live.
#[derive(Debug)]
pub(crate) struct WriteTurn {
    frame: Arc<FrameWrites>,
    ticket: u64,
}

impl WriteOrder {
    /// The next place in `frame`'s order, taken in arrival order.
    pub(crate) fn turn(&self, frame: &str) -> WriteTurn {
        let writes = self
            .frames
            .lock()
            .map(|mut frames| Arc::clone(frames.entry(frame.to_string()).or_default()))
            .unwrap_or_default();
        let ticket = writes.live.lock().map_or(0, |mut tickets| {
            let ticket = tickets.next;
            tickets.next += 1;
            tickets.live.insert(ticket);
            ticket
        });
        WriteTurn {
            frame: writes,
            ticket,
        }
    }
}

impl FrameWrites {
    /// Whether no ticket before `ticket` is live (a poisoned lock orders nothing).
    fn first(&self, ticket: u64) -> bool {
        self.live.lock().map_or(true, |tickets| {
            tickets.live.first().is_none_or(|first| *first >= ticket)
        })
    }
}

impl WriteTurn {
    /// Waits until every earlier live write into the frame is gone, and holds this one's place.
    pub(crate) async fn ready(self) -> Self {
        let frame = Arc::clone(&self.frame);
        loop {
            // Registered before the check, so a release between the two is not missed.
            let mut changed = pin!(frame.changed.notified());
            changed.as_mut().enable();
            if frame.first(self.ticket) {
                return self;
            }
            changed.await;
        }
    }
}

/// Neo: a write leaves its frame's order however it ends, and wakes the writes behind it.
impl Drop for WriteTurn {
    fn drop(&mut self) {
        if let Ok(mut tickets) = self.frame.live.lock() {
            tickets.live.remove(&self.ticket);
        }
        self.frame.changed.notify_waiters();
    }
}

/// Neo: decodes `Frame.setContent`.
impl SetContentOptions {
    pub(crate) fn from_params(params: FrameSetContentParams) -> Self {
        Self {
            html: params.html,
            // Playwright: `server/frames.ts` `Frame.setContent`: `const waitUntil = options.waitUntil === undefined ? 'load' : options.waitUntil;`
            wait_until: params.wait_until.unwrap_or(LifecycleEvent::Load),
        }
    }
}

/// Serves `Frame.content`: the frame's markup, its doctype first, through Playwright's
/// `documentContent`. A document that goes away under the read is Playwright's navigating error.
///
/// Playwright: `server/frames.ts` `Frame._content`: `return await injected.evaluate((injected, includeShadow) => injected.documentContent(includeShadow), !!options.includeShadow);`
pub(crate) async fn content(
    tab: &FrameTab,
    include_shadow: bool,
    progress: &Progress,
) -> Result<FrameContentResult, CallError> {
    progress
        .race(Tab::prepare(tab))
        .await?
        .map_err(read_error)?;
    let frame = progress.race(tab.context()).await?.map_err(read_error)?;
    let read = progress
        .race(frame.call(None, &InjectedDocumentContentParams { include_shadow }))
        .await?
        .map_err(read_error)?;
    Ok(FrameContentResult { value: read.value })
}

/// Serves `Frame.setContent`: the content written into the frame's document, then the wait for
/// `waitUntil` among the lifecycle events the reopened document fires, read from the frame's own
/// session: `document.open()` fires `init`, and only a `DOMContentLoaded` or `load` after it
/// counts. Chromium reads `readyState` as `complete` at `document.close()`, before the content's
/// images load, so the events decide. At that `init` the page watcher clears the frame's shared
/// lifecycle too, as Playwright's `_onClearLifecycle` (for its own `setContent` only), so a later
/// `waitForLoadState` waits for the written document. `networkidle`, which Playwright computes
/// itself, is the frame's own state after this write's reopen (`network_idle.rs`), which the
/// watcher clears at that `init` too. A removed frame or a closed page ends the wait, as
/// Playwright's `raceNavigationAction` rejects.
///
/// Playwright: `server/frames.ts` `Frame.setContent`: `document.write(html);`
pub(crate) async fn set_content(
    tab: &FrameTab,
    options: &SetContentOptions,
    turn: Option<WriteTurn>,
    progress: &Progress,
) -> Result<(), CallError> {
    // Overlapping writes into one frame go in call order: this one waits for the previous.
    let turn = match turn {
        Some(turn) => Some(progress.race(turn.ready()).await?),
        None => None,
    };
    progress.log(format!(
        "setting frame content, waiting until \"{}\"",
        lifecycle_name(options.wait_until)
    ));
    progress
        .race(Tab::prepare(tab))
        .await?
        .map_err(call_error)?;
    let frame = progress.race(tab.context()).await?.map_err(call_error)?;
    // An out-of-process frame's lifecycle events flow only once its host's watcher has enabled
    // them, which every frame wait does first (`FrameObservation::try_locate`). Before the write,
    // so the reopened document's `init` is sent, and before subscribing below, so the replay of
    // the old document's lifecycle that the enable causes is not taken for the new one's.
    progress
        .race(tab.watch_host(&frame.host))
        .await?
        .map_err(call_error)?;
    // Before the write: the reopened document fires its first events while the write is in
    // flight, and the frame may go while it waits.
    let events = frame.host.events();
    let mut observation = WaitTab::observe(tab).map_err(call_error)?;
    if options.wait_until == LifecycleEvent::Networkidle {
        if frame.host.is_child() {
            return Err(PlaywrightError::error(OOPIF_NETWORKIDLE).into());
        }
        // `networkidle` is told from the page's requests, which Neo sees once Network is
        // enabled, as `waitForLoadState`'s: before the write, so the written document's first
        // requests count.
        progress
            .race(WaitTab::track_network(tab))
            .await?
            .map_err(call_error)?;
    }
    // Playwright: `server/frames.ts` `Frame.setContent`: `// Clear lifecycle right after document.open() - see 'tag' below.`
    let reopen = tab.page.expect_reopen(&frame.id, true);
    let written = progress
        .race(frame.call(
            None,
            &InjectedWriteContentParams {
                html: options.html.clone(),
            },
        ))
        .await;
    // A write the page refused never opened the document: it gives its clear back. A stopped
    // call may still be writing, and a contract error came after the write, so those keep it.
    if let Ok(Err(error)) = &written
        && !matches!(error, PageError::Contract(_))
    {
        tab.page.expect_reopen(&frame.id, false);
    }
    // The write answered: the frame's next write may go.
    drop(turn);
    written?.map_err(call_error)?;
    if options.wait_until == LifecycleEvent::Commit {
        return Ok(());
    }
    if options.wait_until == LifecycleEvent::Networkidle {
        return progress
            .race(idle_after_reopen(&mut observation, reopen))
            .await?
            .map_err(call_error);
    }
    // A removed frame (or a closed page) ends the wait, as Playwright's rejects at detach; a
    // navigation is no end: the next document's events count.
    let gone = async {
        loop {
            if let Err(error) = observation.next().await {
                return error;
            }
        }
    };
    let reopened = reopened(
        events,
        frame.host.session_id().clone(),
        frame.id.clone(),
        options.wait_until,
    );
    progress
        .race(async {
            tokio::select! {
                reopened = reopened => reopened,
                error = gone => Err(error),
            }
        })
        .await?
        .map_err(call_error)
}

/// Neo: waits until the frame `frame` on session `session` fires `wait_until` after the `init`
/// of its reopened document.
async fn reopened(
    mut events: cdp::Events,
    session: SessionId,
    frame: String,
    wait_until: LifecycleEvent,
) -> Result<(), PageError> {
    let mut opened = false;
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(RecvError::Closed) => return Err(PageError::Closed),
            Err(RecvError::Lagged(_)) => {
                return Err(PageError::Contract(
                    "neo-runtime: lifecycle events were dropped while setContent waited"
                        .to_string(),
                ));
            }
        };
        if !event.on_session(&session) {
            continue;
        }
        let Some(lifecycle) = event.decode::<cdp::page::LifecycleEvent>() else {
            continue;
        };
        if lifecycle.frame_id != frame {
            continue;
        }
        match lifecycle.name.as_str() {
            "init" => opened = true,
            "DOMContentLoaded" if opened && wait_until == LifecycleEvent::Domcontentloaded => {
                return Ok(());
            }
            "load" if opened => return Ok(()),
            _ => {}
        }
    }
}

/// Neo: waits until the frame's `networkidle` fires after this write's reopen, the frame's
/// `reopen`th: the watcher counts a reopen as it clears the frame's lifecycle at its `init`, in
/// socket order before any request the written document starts, so only an idleness the written
/// document reached counts, never the old document's. A removed frame or a closed page ends the
/// wait; a navigation does not, the next document's idleness counts, as Playwright's wait goes on.
///
/// Playwright: `server/frames.ts` `Frame.setContent`: `const lifecyclePromise = progress.race(tagPromise).then(() => this.waitForLoadState(progress, waitUntil));`
async fn idle_after_reopen(
    observation: &mut WaitObservation,
    reopen: u64,
) -> Result<(), PageError> {
    loop {
        let lifecycle = observation.state().lifecycle;
        if lifecycle.reopens >= reopen && lifecycle.network_idle {
            return Ok(());
        }
        let _navigation = observation.next().await?;
    }
}

/// Serves `Frame.resolveSelector`: Playwright's simple selector for the first match, behind the
/// simple selector of each iframe on the way from the main frame.
///
/// Playwright: `server/frames.ts` `Frame.resolveSelector`: `return injected.generateSelectorSimple(node as unknown as Element);`
pub(crate) async fn resolve_selector(
    tab: &FrameTab,
    selector: &str,
    progress: &Progress,
) -> Result<FrameResolveSelectorResult, CallError> {
    progress
        .race(Tab::prepare(tab))
        .await?
        .map_err(call_error)?;
    let located = progress
        .race(frame_selector::locate(tab, selector, false))
        .await?
        .map_err(call_error)?;
    let Some(located) = located else {
        return Err(no_element(selector));
    };
    let generated = progress
        .race(located.frame.call(
            None,
            &InjectedGenerateSelectorParams {
                selector: located.selector.clone(),
            },
        ))
        .await?
        .map_err(call_error)?;
    let Some(element) = generated.generated else {
        return Err(no_element(selector));
    };
    let mut parts = Vec::with_capacity(located.owners.len() + 1);
    for owner in &located.owners {
        let generated = progress
            .race(owner.frame.call(
                None,
                &InjectedGenerateOwnerSelectorParams {
                    element: owner.element.clone(),
                },
            ))
            .await?
            .map_err(call_error)?;
        parts.push(generated.generated);
    }
    parts.push(element);
    if parts.iter().any(String::is_empty) {
        return Err(
            PlaywrightError::error(format!("Unable to generate locator for {selector}")).into(),
        );
    }
    Ok(FrameResolveSelectorResult {
        resolved_selector: parts.join(ENTER_FRAME),
        locator_code: None,
    })
}

/// Playwright: `server/frames.ts` `Frame.resolveSelector`: `if (!element)`
fn no_element(selector: &str) -> CallError {
    PlaywrightError::error(format!("No element matching {selector}")).into()
}

/// Neo: the name Playwright's log gives a lifecycle event.
fn lifecycle_name(state: LifecycleEvent) -> &'static str {
    match state {
        LifecycleEvent::Commit => "commit",
        LifecycleEvent::Domcontentloaded => "domcontentloaded",
        LifecycleEvent::Load => "load",
        LifecycleEvent::Networkidle => "networkidle",
    }
}

/// Neo: a content read's page error; a document that changed under it is Playwright's
/// navigating error, as is a read the page made throw.
fn read_error(error: PageError) -> CallError {
    match error {
        PageError::Closed => PlaywrightError::target_closed().into(),
        PageError::Contract(message) => PlaywrightError::error(message).into(),
        PageError::NotConnected | PageError::Threw(_) => PlaywrightError::error(NAVIGATING).into(),
    }
}

/// Neo: any other page error, as the page reported it.
fn call_error(error: PageError) -> CallError {
    navigate::page_error(error).into()
}

#[cfg(test)]
mod tests {
    //! The decoding steps and the write order; the page side is covered by the real-browser
    //! tests.

    use std::time::Duration;

    use tokio::time::timeout;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn review_cancelled_middle_write_keeps_the_first_write_ahead_of_the_last() {
        let order = WriteOrder::default();
        let first = order.turn("frame@A").ready().await;
        let middle = order.turn("frame@A");
        let last = order.turn("frame@A");
        let mut middle = Box::pin(middle.ready());
        assert!(
            timeout(Duration::from_millis(1), &mut middle)
                .await
                .is_err()
        );
        // A stopped call waiting in the chain releases its own place, but the first
        // call may still be preparing its write and must remain ahead of the last.
        drop(middle);
        let mut last = Box::pin(last.ready());
        assert!(
            timeout(Duration::from_millis(1), &mut last).await.is_err(),
            "the last write passed a live first write after the middle call stopped"
        );
        drop(first);
        let _last = timeout(Duration::from_millis(1), last)
            .await
            .expect("cancellation must not leave the later write stuck");
    }

    #[tokio::test]
    async fn tests_that_writes_into_one_frame_go_in_arrival_order() {
        let order = WriteOrder::default();
        let first = order.turn("frame@A");
        let second = order.turn("frame@A");
        let other = order.turn("frame@B");
        let other = timeout(Duration::from_millis(100), other.ready())
            .await
            .expect("another frame's write goes at once");
        let mut waiting = Box::pin(second.ready());
        assert!(
            timeout(Duration::from_millis(100), &mut waiting)
                .await
                .is_err(),
            "the second write waits for the first"
        );
        drop(first.ready().await);
        let _second = timeout(Duration::from_secs(1), waiting)
            .await
            .expect("the second write goes once the first answered");
        drop(other);
    }

    #[test]
    fn tests_that_set_content_waits_for_load_unless_told() {
        let params = |wait_until| FrameSetContentParams {
            html: "<p>x</p>".to_string(),
            wait_until,
        };
        assert_eq!(
            SetContentOptions::from_params(params(None)).wait_until,
            LifecycleEvent::Load
        );
        assert_eq!(
            SetContentOptions::from_params(params(Some(LifecycleEvent::Commit))).wait_until,
            LifecycleEvent::Commit
        );
        assert_eq!(
            SetContentOptions::from_params(params(Some(LifecycleEvent::Networkidle))).wait_until,
            LifecycleEvent::Networkidle,
            "networkidle is served after the write's reopen (network_idle.rs)"
        );
    }
}
