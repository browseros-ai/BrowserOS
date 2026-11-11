//! The navigations a click's input asked for, and the wait for each to commit (design D11).
//!
//! Upstream's click waits for every navigation its input scheduled to commit or be cancelled
//! (`waitForSignalsCreatedBy` with `waitAfter`), and waits for nothing else: no load state, no
//! settle. Neo reads the same fact from CDP:
//!
//! ```text
//!  watch (before the burst) ── Page.frameRequestedNavigation, disposition currentTab
//!                               on the tab's root session or the element's frame session
//!  settle (after the verdict) ── per requested frame: Page.frameNavigated (committed),
//!                               Page.navigatedWithinDocument, Page.frameStoppedLoading
//!                               (cancelled, or a download), Page.frameDetached
//! ```
//!
//! The same watch tells the hit-target verdict to stop waiting: while a requested navigation
//! is pending, the page answers nothing (`RequestedNavigations::pending`).
//!
//! Core enables the Page domain on page sessions only, so a navigation an out-of-process
//! iframe requests for itself is not seen and not waited for; one it requests for the top
//! frame (`target=_top`) is.

use std::collections::HashSet;
use std::future;

use browseros_cdp::CdpEvent;
use browseros_core::SessionId;
use serde::Deserialize;
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::op_scope::{Expired, OpScope};

/// The CDP events of one click's tab, from just before its input.
pub(crate) struct RequestedNavigations {
    events: Receiver<CdpEvent>,
    /// The sessions whose frames the click can navigate.
    sessions: Vec<SessionId>,
    /// Frames with a navigation requested and not yet committed or cancelled.
    pending: HashSet<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Requested {
    frame_id: String,
    disposition: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfFrame {
    frame_id: String,
}

#[derive(Deserialize)]
struct Navigated {
    frame: NavigatedFrame,
}

#[derive(Deserialize)]
struct NavigatedFrame {
    id: String,
}

impl RequestedNavigations {
    /// Starts listening. Take it before the first input event, so no request is missed.
    pub(crate) fn watch(events: Receiver<CdpEvent>, sessions: Vec<SessionId>) -> Self {
        Self {
            events,
            sessions,
            pending: HashSet::new(),
        }
    }

    /// Resolves while one of its frames has a navigation requested and not yet committed or
    /// cancelled: upstream's pending document (`Frame._pendingDocument`), which makes it give up
    /// on reading the page (`raceAgainstEvaluationStallingEvents`). Chromium answers no call
    /// bound for a renderer that is starting such a navigation until it commits or fails
    /// (measured: a link's own click blocks `Runtime.evaluate` that long), so a read raced with
    /// this ends instead of waiting on a page that may never answer. Never resolves once the
    /// events are lost.
    pub(crate) async fn pending(&mut self) {
        loop {
            self.drain();
            if !self.pending.is_empty() {
                return;
            }
            match self.events.recv().await {
                Ok(event) => self.apply(&event),
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return future::pending().await,
            }
        }
    }

    /// Applies every event already received.
    fn drain(&mut self) {
        loop {
            match self.events.try_recv() {
                Ok(event) => self.apply(&event),
                Err(TryRecvError::Lagged(_)) => {}
                Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            }
        }
    }

    /// Waits until every navigation requested so far committed or was cancelled, within the
    /// op's timeout. A request that reaches CDP after this starts is not waited for, as with
    /// upstream's barrier, which also closes when the action returns.
    pub(crate) async fn settle(mut self, scope: &OpScope<'_>) -> Result<(), Expired> {
        self.drain();
        while !self.pending.is_empty() {
            match scope.within(self.events.recv()).await? {
                Ok(event) => self.apply(&event),
                // Events were dropped, the ends perhaps among them: nothing left to wait on
                // that could be told apart from a hang.
                Err(RecvError::Lagged(_) | RecvError::Closed) => return Ok(()),
            }
        }
        Ok(())
    }

    fn apply(&mut self, event: &CdpEvent) {
        if !event
            .session_id
            .as_ref()
            .is_some_and(|session| self.sessions.contains(session))
        {
            return;
        }
        let params = event.params.clone();
        match event.method.as_str() {
            "Page.frameRequestedNavigation" => {
                if let Ok(requested) = serde_json::from_value::<Requested>(params)
                    && requested.disposition == "currentTab"
                {
                    self.pending.insert(requested.frame_id);
                }
            }
            "Page.frameNavigated" => {
                if let Ok(navigated) = serde_json::from_value::<Navigated>(params) {
                    self.pending.remove(&navigated.frame.id);
                }
            }
            "Page.navigatedWithinDocument" | "Page.frameStoppedLoading" | "Page.frameDetached" => {
                if let Ok(frame) = serde_json::from_value::<OfFrame>(params) {
                    self.pending.remove(&frame.frame_id);
                }
            }
            _ => {}
        }
    }
}
