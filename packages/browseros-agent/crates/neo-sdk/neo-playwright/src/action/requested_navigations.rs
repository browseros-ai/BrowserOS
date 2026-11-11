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
use browseros_core::{FrameId, SessionId, TargetId};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::events::{EventHub, PageEvent, PageEventKind};
use crate::op_scope::{Expired, OpScope};
use crate::{ErrorCode, PlaywrightError};

/// The CDP events of one click's tab, from just before its input.
pub(crate) struct RequestedNavigations {
    events: Receiver<CdpEvent>,
    /// The sessions whose frames the click can navigate.
    sessions: Vec<SessionId>,
    /// Frames with a navigation requested and not yet committed or cancelled.
    pending: HashSet<String>,
    /// Actual requested commits retained until the public journal catches up.
    commits: Vec<Committed>,
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
#[serde(rename_all = "camelCase")]
struct NavigatedFrame {
    id: String,
    #[serde(default)]
    loader_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithinDocument {
    frame_id: String,
    url: String,
}

/// The public event cut caused by this input, so JS can publish its waiting consumers first.
#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct NavigationCut {
    pub(crate) seq: u64,
    pub(crate) at: u64,
}

/// Correlates each actual commit rather than borrowing an older event from the same frame.
enum Committed {
    Document {
        frame: String,
        loader: Option<String>,
    },
    SameDocument {
        frame: String,
        url: String,
    },
}

impl Committed {
    fn matches(&self, id: &FrameId, loader_id: Option<&str>, url: &str) -> bool {
        match self {
            Self::Document { frame, loader } => {
                id.as_str() == frame && loader_id == loader.as_deref()
            }
            Self::SameDocument {
                frame,
                url: expected,
            } => id.as_str() == frame && url == expected,
        }
    }
}

impl RequestedNavigations {
    /// Starts listening. Take it before the first input event, so no request is missed.
    pub(crate) fn watch(events: Receiver<CdpEvent>, sessions: Vec<SessionId>) -> Self {
        Self {
            events,
            sessions,
            pending: HashSet::new(),
            commits: Vec::new(),
        }
    }

    /// Resolves while `frame` has a navigation requested and not yet committed or cancelled:
    /// upstream's pending document (`Frame._pendingDocument`), which makes it give up on reading
    /// that frame (`raceAgainstEvaluationStallingEvents`). Chromium answers no call bound for a
    /// renderer that is starting such a navigation until it commits or fails (measured: a link's
    /// own click blocks `Runtime.evaluate` that long), so a read of `frame` raced with this ends
    /// instead of waiting on a page that may never answer. Another frame's navigation does not
    /// count, as upstream's does not. Never resolves once the events are lost.
    pub(crate) async fn pending_in(&mut self, frame: &FrameId) {
        loop {
            self.drain();
            if self.pending.contains(frame.as_str()) {
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
    pub(crate) async fn settle(mut self, scope: &OpScope<'_>) -> Result<Self, Expired> {
        self.drain();
        while !self.pending.is_empty() {
            match scope.within(self.events.recv()).await? {
                Ok(event) => self.apply(&event),
                // Events were dropped, the ends perhaps among them: nothing left to wait on
                // that could be told apart from a hang.
                Err(RecvError::Lagged(_) | RecvError::Closed) => return Ok(self),
            }
        }
        Ok(self)
    }

    /// Raw CDP and the public journal have different consumers. Wait for the same committed
    /// documents to reach the journal before returning a cut; no timer guesses at readiness.
    /// Loss or closure with unmatched commits is an observation failure after input, never
    /// an absent/partial cut that could incorrectly report successful publication.
    pub(crate) async fn publication(
        &self,
        scope: &OpScope<'_>,
        hub: &EventHub,
        target: &TargetId,
        after: u64,
    ) -> Result<Option<NavigationCut>, PlaywrightError> {
        let mut pending: Vec<_> = self.commits.iter().collect();
        let mut cut = None;
        while !pending.is_empty() {
            let changed = hub.changed();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let read = hub.read_after(after);
            for record in read.records {
                if matches!(&record.event, PageEvent::Closed(closed) if closed == target) {
                    return Err(scope
                        .fail(
                            ErrorCode::TargetClosed,
                            "Page closed before navigation publication",
                        )
                        .after_input());
                }
                if let PageEvent::FrameLifecycle(event) = &record.event
                    && event.target == *target
                    && event.kind == PageEventKind::FrameNavigated
                {
                    let before = pending.len();
                    pending.retain(|commit| {
                        !commit.matches(
                            &event.frame.id,
                            event.frame.loader_id.as_deref(),
                            &event.frame.url,
                        )
                    });
                    if pending.len() != before {
                        cut = Some(NavigationCut {
                            seq: record.seq,
                            at: record.at_ms,
                        });
                    }
                }
            }
            if pending.is_empty() {
                return Ok(cut);
            }
            if read.lost > 0 {
                return Err(scope
                    .fail(
                        ErrorCode::Internal,
                        "Navigation events were lost before click completion",
                    )
                    .after_input());
            }
            scope.within(changed).await.map_err(|_expired| {
                scope
                    .timed_out(ErrorCode::NavigationObservationTimeout)
                    .after_input()
            })?;
        }
        Ok(cut)
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
                if let Ok(navigated) = serde_json::from_value::<Navigated>(params)
                    && self.pending.remove(&navigated.frame.id)
                {
                    self.commits.push(Committed::Document {
                        frame: navigated.frame.id,
                        loader: navigated.frame.loader_id,
                    });
                }
            }
            "Page.navigatedWithinDocument" => {
                if let Ok(frame) = serde_json::from_value::<WithinDocument>(params)
                    && self.pending.remove(&frame.frame_id)
                {
                    self.commits.push(Committed::SameDocument {
                        frame: frame.frame_id,
                        url: frame.url,
                    });
                }
            }
            "Page.frameStoppedLoading" | "Page.frameDetached" => {
                if let Ok(frame) = serde_json::from_value::<OfFrame>(params) {
                    self.pending.remove(&frame.frame_id);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Duration;

    use browseros_cdp::CdpEvent;
    use browseros_core::{FrameId, SessionId, TargetId};
    use serde_json::json;
    use tokio::sync::broadcast;
    use tokio::time;

    use super::{Committed, RequestedNavigations};
    use crate::events::event_hub::LOG_CAPACITY;
    use crate::events::{EventHub, PageEvent};
    use crate::op_scope::OpScope;
    use crate::target::CallHeader;
    use crate::test_env::TestEnv;
    use crate::{Answer, ErrorCode, Op};

    const SESSION: &str = "root";

    fn event(method: &str, params: serde_json::Value) -> CdpEvent {
        CdpEvent {
            method: method.to_owned(),
            params,
            session_id: Some(SessionId::from(SESSION.to_owned())),
        }
    }

    fn requested(frame: &str) -> CdpEvent {
        event(
            "Page.frameRequestedNavigation",
            json!({ "frameId": frame, "disposition": "currentTab", "reason": "anchorClick", "url": "https://a.test/" }),
        )
    }

    fn watch() -> (broadcast::Sender<CdpEvent>, RequestedNavigations) {
        let (events, receiver) = broadcast::channel(16);
        let watch =
            RequestedNavigations::watch(receiver, vec![SessionId::from(SESSION.to_owned())]);
        (events, watch)
    }

    /// The element's frame starting a navigation ends a verdict's wait (AW-007).
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_pending_navigation_of_the_frame_resolves() -> Result<(), Box<dyn Error>> {
        let (events, mut watch) = watch();
        events.send(requested("main"))?;
        let frame = FrameId::from("main".to_owned());
        time::timeout(Duration::from_secs(1), watch.pending_in(&frame)).await?;
        Ok(())
    }

    /// Another frame's navigation does not count, as upstream's pending document is its own
    /// frame's; nor does one that already committed.
    #[tokio::test(start_paused = true)]
    async fn tests_that_other_frames_and_committed_navigations_do_not_resolve()
    -> Result<(), Box<dyn Error>> {
        let (events, mut watch) = watch();
        events.send(requested("child"))?;
        events.send(requested("main"))?;
        events.send(event(
            "Page.frameNavigated",
            json!({ "frame": { "id": "main" } }),
        ))?;
        let frame = FrameId::from("main".to_owned());
        let waited = time::timeout(Duration::from_secs(1), watch.pending_in(&frame)).await;
        assert!(
            waited.is_err(),
            "neither the child's nor a committed navigation counts"
        );
        Ok(())
    }

    #[test]
    fn tests_that_publication_matches_the_committed_document_not_an_older_frame_event() {
        let document = Committed::Document {
            frame: "main".to_owned(),
            loader: Some("new-loader".to_owned()),
        };
        let main = FrameId::from("main".to_owned());
        let child = FrameId::from("child".to_owned());
        assert!(!document.matches(&main, Some("old-loader"), "https://a.test/"));
        assert!(!document.matches(&child, Some("new-loader"), "https://a.test/"));
        assert!(document.matches(&main, Some("new-loader"), "https://a.test/"));
        let same_document = Committed::SameDocument {
            frame: "main".to_owned(),
            url: "https://a.test/#next".to_owned(),
        };
        assert!(!same_document.matches(&main, None, "https://a.test/"));
        assert!(same_document.matches(&main, None, "https://a.test/#next"));
    }

    #[test]
    fn tests_that_only_requested_commits_need_publication() {
        let (_events, mut watch) = watch();
        watch.apply(&event(
            "Page.frameNavigated",
            json!({ "frame": { "id": "unrelated", "loaderId": "other" } }),
        ));
        watch.apply(&requested("cancelled"));
        watch.apply(&event(
            "Page.frameStoppedLoading",
            json!({ "frameId": "cancelled" }),
        ));
        watch.apply(&requested("removed"));
        watch.apply(&event(
            "Page.frameDetached",
            json!({ "frameId": "removed" }),
        ));
        assert!(watch.commits.is_empty());
        assert!(watch.pending.is_empty());
        watch.apply(&requested("main"));
        watch.apply(&event(
            "Page.frameNavigated",
            json!({ "frame": { "id": "main", "loaderId": "new" } }),
        ));
        assert_eq!(watch.commits.len(), 1);
        assert!(watch.commits[0].matches(
            &FrameId::from("main".to_owned()),
            Some("new"),
            "https://a.test/"
        ));
    }

    #[tokio::test]
    async fn tests_that_journal_loss_cannot_silently_discard_an_unpublished_commit()
    -> Result<(), Box<dyn Error>> {
        let (hub, mut writer) = EventHub::new();
        for _ in 0..=LOG_CAPACITY {
            writer.record(PageEvent::Closed(TargetId("other".to_owned())));
        }
        let (_events, mut watch) = watch();
        watch.apply(&requested("main"));
        watch.apply(&event(
            "Page.frameNavigated",
            json!({ "frame": { "id": "main", "loaderId": "new" } }),
        ));
        let env = TestEnv::new();
        let op = Op::from_name("locator.click").ok_or("click is registered")?;
        let scope = OpScope::new(&env, op, CallHeader::default());
        let failure = watch
            .publication(&scope, &hub, &TargetId("main".to_owned()), 0)
            .await;
        let Err(error) = failure else {
            panic!("a lost commit cannot publish successfully")
        };
        assert_eq!(error.code(), ErrorCode::Internal.as_str());
        assert_eq!(error.answer(), Answer::Ambiguous);
        Ok(())
    }
}
