//! No view display while a private tab is in front of Neo's one screen (decision 34). A private
//! tab gets no screenshots and no screencast, but a view shows Xvnc's raw pixels, which no capture
//! guard sees. So this module keeps the rule by refusing and closing the views themselves:
//!
//! ```text
//!  admit (a view) ─ registered, then the front read ─ a private tab in front: DisplayPaused (409)
//!  watch, while any view is open:
//!    tab.private (a flag committed), or a lag ─┐
//!    every 250 ms (FRONT_POLL) ────────────────┴─▶ the front read ─ a private tab in front, or
//!                                                  no answer ─▶ every view closes (4001)
//!  the front read: Browser.getTabs' active tabs ─▶ Privacy::is_private
//! ```
//!
//! **Why a poll.** The agent's own move of a private tab closes every view before the activation
//! is sent (`Front`, decision 46). But the browser sends no event when a tab comes to the front,
//! so a private tab that something else brings forward (the owner, a page's popup, the tab in
//! front closing) is seen only at the next read. A view can show it for up to one poll interval
//! and a `Browser.getTabs` round trip before it closes. A tab that goes private while it is in
//! front closes the views as soon as its flag commits, through the trace writer's
//! `tab.private`. A poll can catch it sooner, because `Privacy` sets the flag in memory before
//! the commit. Follow never brings a private tab forward itself (follow.rs).
//!
//! The watch runs only while a view display is open. The first view starts it, and the last
//! one's end stops it ([`crate::Displays`]'s table). Two views share one watch.

use crate::limits::FRONT_POLL;
use crate::{BrowserTabs, Displays, PartError, follow};
use browseros_core::TabId;
use neo_trace::{Privacy, TraceRecordEvent};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

/// The private tab in front of the screen, if any. Neo has one window; should several windows
/// each report an active tab, any private one counts, which only over-protects.
///
/// # Errors
/// `Browser` when the browser cannot list its tabs. Callers fail closed on it.
pub(crate) async fn private_in_front(
    browser: &BrowserTabs,
    privacy: &Privacy,
) -> Result<Option<TabId>, PartError> {
    Ok(browser
        .front_tabs()
        .await?
        .into_iter()
        .find(|&tab| privacy.is_private(tab).is_some()))
}

/// Closes every view display whenever a private tab is in front, or what is in front cannot be
/// read, until `stop` is cancelled. `events` is the trace writer's, subscribed by the caller.
pub(crate) async fn watch(
    displays: Arc<Displays>,
    stop: CancellationToken,
    events: broadcast::Receiver<TraceRecordEvent>,
) {
    let mut events = Some(events);
    // The view that started this watch reads the front itself as it is admitted.
    let mut poll = tokio::time::interval_at(Instant::now() + FRONT_POLL, FRONT_POLL);
    poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        let woken_by = tokio::select! {
            biased;
            () = stop.cancelled() => return,
            event = follow::next_event(&mut events) => match event {
                Ok(TraceRecordEvent::TabPrivate { .. }) => "tab.private",
                // It may have dropped a `tab.private`.
                Err(RecvError::Lagged(_)) => "a lagged trace stream",
                Ok(_) => continue,
                Err(RecvError::Closed) => {
                    events = None;
                    continue;
                }
            },
            _ = poll.tick() => "the poll",
        };
        let read = tokio::select! {
            biased;
            () = stop.cancelled() => return,
            read = private_in_front(&displays.browser, &displays.privacy) => read,
        };
        match read {
            Ok(None) => {}
            Ok(Some(tab)) => {
                tracing::info!(
                    tab = tab.0,
                    woken_by,
                    "a private tab is in front: every view display closes"
                );
                displays.close_views();
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    woken_by,
                    "the tab in front could not be read: every view display closes"
                );
                displays.close_views();
            }
        }
    }
}
