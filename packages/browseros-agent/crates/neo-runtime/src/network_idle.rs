//! Playwright: `server/frames.ts` `Frame._recalculateNetworkIdle`
//! Playwright: `server/frames.ts` `Frame._startNetworkIdleTimer`
//! Playwright: `server/frames.ts` `Frame._stopNetworkIdleTimer`
//! Playwright: `server/frames.ts` `Frame._onClearLifecycle`
//! Playwright: `server/frames.ts` `Frame.onLifecycleEvent`
//! Playwright: `server/frames.ts` `FrameManager._inflightRequestStarted`
//! Playwright: `server/frames.ts` `FrameManager._inflightRequestFinished`
//!
//! The `networkidle` lifecycle event of one tab's frames, which Playwright's server computes
//! itself (Chromium sends no such event to it): a frame is network idle once none of its own
//! requests (event streams aside) has been in flight for 500 ms, and it fires `networkidle` once
//! it and every child frame are. The tab's watcher (`page.rs` `watch_events`) feeds every frame
//! event and request of the tab's session here in socket order and drives the timers, and writes
//! what changes into the main frame's state and the child frames' feeds, where `goto`,
//! `waitForLoadState`, `waitForURL`, the history steps and `waitForNavigation` wait for it.
//!
//! Pure: no CDP, no clock of its own. Every call takes the watcher's `now`, and
//! [`NetworkIdle::deadline`] says when the next timer is due.
//!
//! Deviation: `server/frames.ts` `FrameManager.requestStarted`: `this._inflightRequestStarted(request);`
//! Playwright's server sees every request of a page from its first moment; Neo sees a tab's
//! requests only after the run's `Network.enable`, so [`NetworkIdle::restart`] starts every
//! frame's quiet period at that moment, and a frame whose document moved to another process
//! (`Page.frameDetached` with reason `swap`) leaves the tree with its requests, since that
//! process reports its requests on a session the run may never attach.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::Instant;

use crate::network::HopId;

/// Playwright: `server/frames.ts` `Frame._startNetworkIdleTimer`: `}, 500);`
const NETWORK_IDLE_QUIET: Duration = Duration::from_millis(500);

/// A frame's `networkidle` lifecycle event, added or removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum IdleChange {
    Fired(String),
    Lost(String),
}

/// One tab's [`NetworkIdle`], shared by the tab's watcher, which feeds it and fires its timers, and
/// `PageSession::enable_network`, which seeds the frame tree and restarts it.
pub(crate) struct SharedIdle {
    idle: Mutex<NetworkIdle>,
    /// Wakes the watcher after a change from outside it moved the next deadline.
    moved: Notify,
}

/// Neo: the tracker's lock and the watcher's wake, which Playwright's single-threaded server needs
/// neither of.
impl SharedIdle {
    pub(crate) fn new(main: &str, now: Instant) -> Self {
        Self {
            idle: Mutex::new(NetworkIdle::new(main, now)),
            moved: Notify::new(),
        }
    }

    /// Runs `change` on the tracker; `None` if its lock is poisoned.
    pub(crate) fn with<T>(&self, change: impl FnOnce(&mut NetworkIdle) -> T) -> Option<T> {
        self.idle.lock().ok().map(|mut idle| change(&mut idle))
    }

    /// The next deadline moved under the watcher: it recomputes its sleep.
    pub(crate) fn moved(&self) {
        self.moved.notify_one();
    }

    /// Resolves once [`Self::moved`] was called (at once for a call made meanwhile).
    pub(crate) async fn wait_moved(&self) {
        self.moved.notified().await;
    }
}

/// Every frame of one tab the tab's session reports, as Playwright's `Frame` holds its idle state.
pub(crate) struct NetworkIdle {
    main: String,
    frames: HashMap<String, Frame>,
    /// Each request in flight, by hop, and its frame.
    requests: HashMap<HopId, String>,
}

/// Playwright: `server/frames.ts` `Frame`
#[derive(Default)]
struct Frame {
    parent: Option<String>,
    /// `_inflightRequests`.
    inflight: HashSet<HopId>,
    /// `_networkIdleRequests`: `inflight` without event streams.
    idle_requests: HashSet<HopId>,
    /// `_networkIdleTimer`: when it fires.
    timer: Option<Instant>,
    /// `_firedNetworkIdleSelf`.
    fired_self: bool,
    /// `_firedLifecycleEvents`: `networkidle`, and the `domcontentloaded` and `load` events of the
    /// current document, which recalculate only the first time each fires.
    networkidle: bool,
    lifecycle: HashSet<String>,
}

/// Playwright: `server/frames.ts` `Frame`
impl NetworkIdle {
    /// The tab's main frame, created now: its timer starts as a new `Frame`'s does.
    pub(crate) fn new(main: &str, now: Instant) -> Self {
        let mut idle = Self {
            main: main.to_string(),
            frames: HashMap::new(),
            requests: HashMap::new(),
        };
        idle.frames.insert(main.to_string(), Frame::default());
        idle.start_timer(main, now);
        idle
    }

    /// Whether `frame` has fired `networkidle` for its current document.
    pub(crate) fn fired(&self, frame: &str) -> bool {
        self.frames
            .get(frame)
            .is_some_and(|frame| frame.networkidle)
    }

    /// When the earliest pending timer fires.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.frames.values().filter_map(|frame| frame.timer).min()
    }

    /// `FrameManager.frameAttached` for a child: a new `Frame`, whose timer starts now. A frame
    /// already known, or one whose parent is gone, is left alone (`CRPage._onFrameAttached`).
    pub(crate) fn attach(&mut self, frame: &str, parent: &str, now: Instant) {
        if self.frames.contains_key(frame) || !self.frames.contains_key(parent) {
            return;
        }
        self.frames.insert(
            frame.to_string(),
            Frame {
                parent: Some(parent.to_string()),
                ..Frame::default()
            },
        );
        self.start_timer(frame, now);
    }

    /// `FrameManager.frameDetached`, and a document moving to another process (Neo's Deviation
    /// in the header): the frame and its subtree go, with their requests.
    pub(crate) fn detach(&mut self, frame: &str) -> Vec<IdleChange> {
        if frame == self.main || !self.frames.contains_key(frame) {
            return Vec::new();
        }
        self.remove_recursively(frame);
        self.recalculate(None)
    }

    /// `FrameManager.frameCommittedNewDocumentNavigation` for `frame`'s new `document` (its
    /// loader id): its child frames go, its requests other than the document's own end
    /// (`_failInflightRequests`), and its lifecycle clears ([`Self::clear_lifecycle`]).
    pub(crate) fn commit(&mut self, frame: &str, document: &str, now: Instant) -> Vec<IdleChange> {
        let Some(children) = self.frames.get(frame).map(|_| self.children(frame)) else {
            return Vec::new();
        };
        for child in children {
            self.remove_recursively(&child);
        }
        if let Some(state) = self.frames.get_mut(frame) {
            let left: Vec<HopId> = state
                .inflight
                .iter()
                .filter(|request| request.request != document)
                .cloned()
                .collect();
            for request in &left {
                state.inflight.remove(request);
                self.requests.remove(request);
            }
        }
        self.clear_lifecycle(frame, document, now)
    }

    /// `Frame._onClearLifecycle`, at a commit and right after a `setContent` write's
    /// `document.open()`: `frame`'s fired events clear, and its quiet period starts over without
    /// its requests other than its current document's own (`current_document`, the loader id,
    /// which names the document's request), then `commit` fires. Those requests stay in flight,
    /// their ends no longer counting for the frame's idleness; its child frames stay.
    pub(crate) fn clear_lifecycle(
        &mut self,
        frame: &str,
        current_document: &str,
        now: Instant,
    ) -> Vec<IdleChange> {
        let mut changes = Vec::new();
        let Some(state) = self.frames.get_mut(frame) else {
            return changes;
        };
        // Playwright: `server/frames.ts` `Frame._onClearLifecycle`: `this._networkIdleRequests = new Set(Array.from(this._networkIdleRequests).filter(request => request === this._currentDocument.request));`
        state
            .idle_requests
            .retain(|request| request.request == current_document);
        if state.networkidle {
            changes.push(IdleChange::Lost(frame.to_string()));
        }
        state.networkidle = false;
        state.lifecycle.clear();
        self.stop_timer(frame);
        if self
            .frames
            .get(frame)
            .is_some_and(|state| state.idle_requests.is_empty())
        {
            self.start_timer(frame, now);
        }
        changes.extend(self.recalculate(Some(frame)));
        changes.extend(self.lifecycle_event(frame, "commit"));
        changes
    }

    /// `Frame.onLifecycleEvent` for `domcontentloaded` or `load` (CDP's `DOMContentLoaded` and
    /// `load`): the first of each per document recalculates.
    pub(crate) fn lifecycle_event(&mut self, frame: &str, event: &str) -> Vec<IdleChange> {
        let Some(state) = self.frames.get_mut(frame) else {
            return Vec::new();
        };
        if !state.lifecycle.insert(event.to_string()) {
            return Vec::new();
        }
        self.recalculate(None)
    }

    /// `FrameManager._inflightRequestStarted`: a request of `frame` started; an event stream
    /// never holds the frame's idleness.
    pub(crate) fn started(&mut self, request: HopId, frame: &str, event_stream: bool) {
        let Some(state) = self.frames.get_mut(frame) else {
            return;
        };
        state.inflight.insert(request.clone());
        self.requests.insert(request.clone(), frame.to_string());
        if event_stream {
            return;
        }
        state.idle_requests.insert(request);
        if state.idle_requests.len() == 1 {
            self.stop_timer(frame);
        }
    }

    /// `FrameManager._inflightRequestFinished`: a request finished or failed; the frame's last
    /// one starts its quiet period.
    pub(crate) fn ended(&mut self, request: &HopId, now: Instant) {
        let Some(frame) = self.requests.remove(request) else {
            return;
        };
        let Some(state) = self.frames.get_mut(&frame) else {
            return;
        };
        if !state.inflight.remove(request) {
            return;
        }
        if state.idle_requests.remove(request) && state.idle_requests.is_empty() {
            self.start_timer(&frame, now);
        }
    }

    /// Fires every timer due by `now`, then recalculates.
    pub(crate) fn tick(&mut self, now: Instant) -> Vec<IdleChange> {
        let mut fired = false;
        for state in self.frames.values_mut() {
            if state.timer.is_some_and(|at| at <= now) {
                state.timer = None;
                state.fired_self = true;
                fired = true;
            }
        }
        if fired {
            self.recalculate(None)
        } else {
            Vec::new()
        }
    }

    /// Neo's `Network.enable` on the tab answered: what the frames fired before was judged without
    /// their requests, so every frame starts over, its quiet period from `now`.
    pub(crate) fn restart(&mut self, now: Instant) -> Vec<IdleChange> {
        let mut changes = Vec::new();
        let frames: Vec<String> = self.frames.keys().cloned().collect();
        for frame in frames {
            self.stop_timer(&frame);
            if let Some(state) = self.frames.get_mut(&frame) {
                if state.networkidle {
                    changes.push(IdleChange::Lost(frame.clone()));
                }
                state.networkidle = false;
            }
            if self
                .frames
                .get(&frame)
                .is_some_and(|state| state.idle_requests.is_empty())
            {
                self.start_timer(&frame, now);
            }
        }
        changes
    }

    /// The watcher lost events (its queue overflowed): any request's end may be among them, so
    /// every request is forgotten and the frames start over from `now`, as after a
    /// [`Self::restart`], rather than hold `networkidle` for an end that never comes.
    pub(crate) fn forget_requests(&mut self, now: Instant) -> Vec<IdleChange> {
        self.requests.clear();
        for state in self.frames.values_mut() {
            state.inflight.clear();
            state.idle_requests.clear();
        }
        self.restart(now)
    }

    fn children(&self, frame: &str) -> Vec<String> {
        self.frames
            .iter()
            .filter(|(_, state)| state.parent.as_deref() == Some(frame))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// `FrameManager._removeFramesRecursively`: its subtree first, then the frame, whose timer
    /// stops (`_onDetached`) and whose requests are forgotten (`_failInflightRequests`).
    fn remove_recursively(&mut self, frame: &str) {
        for child in self.children(frame) {
            self.remove_recursively(&child);
        }
        if let Some(state) = self.frames.remove(frame) {
            for request in state.inflight {
                self.requests.remove(&request);
            }
        }
    }

    /// `Frame._startNetworkIdleTimer`: none for a frame that fired `networkidle` already.
    fn start_timer(&mut self, frame: &str, now: Instant) {
        if let Some(state) = self.frames.get_mut(frame)
            && !state.networkidle
            && state.timer.is_none()
        {
            state.timer = Some(now + NETWORK_IDLE_QUIET);
        }
    }

    /// `Frame._stopNetworkIdleTimer`.
    fn stop_timer(&mut self, frame: &str) {
        if let Some(state) = self.frames.get_mut(frame) {
            state.timer = None;
            state.fired_self = false;
        }
    }

    /// `Frame._recalculateNetworkIdle` from the main frame: a frame fires `networkidle` once it
    /// is idle itself and every child fired; one that fired and is no longer idle loses it,
    /// except the frame whose lifecycle is being cleared, `allow_removing`'s.
    fn recalculate(&mut self, allow_removing: Option<&str>) -> Vec<IdleChange> {
        let mut changes = Vec::new();
        let main = self.main.clone();
        self.recalculate_frame(&main, allow_removing, &mut changes);
        changes
    }

    fn recalculate_frame(
        &mut self,
        frame: &str,
        allow_removing: Option<&str>,
        changes: &mut Vec<IdleChange>,
    ) {
        let Some(mut idle) = self.frames.get(frame).map(|state| state.fired_self) else {
            return;
        };
        for child in self.children(frame) {
            self.recalculate_frame(&child, allow_removing, changes);
            // Playwright: `server/frames.ts` `Frame._recalculateNetworkIdle`: `if (!child._firedLifecycleEvents.has('networkidle'))`
            if !self.fired(&child) {
                idle = false;
            }
        }
        let Some(state) = self.frames.get_mut(frame) else {
            return;
        };
        if idle && !state.networkidle {
            state.networkidle = true;
            changes.push(IdleChange::Fired(frame.to_string()));
        }
        if allow_removing != Some(frame) && state.networkidle && !idle {
            state.networkidle = false;
            changes.push(IdleChange::Lost(frame.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUIET: Duration = NETWORK_IDLE_QUIET;

    fn hop(id: &str) -> HopId {
        HopId {
            request: id.to_string(),
            hop: 0,
        }
    }

    fn fired(frame: &str) -> IdleChange {
        IdleChange::Fired(frame.to_string())
    }

    fn lost(frame: &str) -> IdleChange {
        IdleChange::Lost(frame.to_string())
    }

    #[test]
    fn tests_that_a_quiet_frame_fires_after_its_quiet_period_only() {
        let start = Instant::now();
        let mut idle = NetworkIdle::new("M", start);
        assert_eq!(idle.deadline(), Some(start + QUIET));
        assert!(
            idle.tick(start + QUIET - Duration::from_millis(1))
                .is_empty()
        );
        assert_eq!(idle.tick(start + QUIET), [fired("M")]);
        assert!(idle.fired("M"));
        assert_eq!(idle.deadline(), None, "a fired frame starts no timer");
    }

    #[test]
    fn tests_that_a_request_holds_idleness_until_quiet_after_it_ends() {
        let start = Instant::now();
        let mut idle = NetworkIdle::new("M", start);
        idle.started(hop("1"), "M", false);
        assert_eq!(idle.deadline(), None, "a request stops the timer");
        assert!(idle.tick(start + QUIET * 4).is_empty());
        let end = start + QUIET * 4;
        idle.ended(&hop("1"), end);
        assert_eq!(idle.deadline(), Some(end + QUIET));
        assert_eq!(idle.tick(end + QUIET), [fired("M")]);
    }

    #[test]
    fn tests_that_an_event_stream_never_holds_idleness() {
        let start = Instant::now();
        let mut idle = NetworkIdle::new("M", start);
        idle.started(hop("events"), "M", true);
        assert_eq!(idle.tick(start + QUIET), [fired("M")]);
    }

    #[test]
    fn tests_that_a_parent_waits_for_every_child_frame() {
        let start = Instant::now();
        let mut idle = NetworkIdle::new("M", start);
        idle.attach("C", "M", start);
        idle.started(hop("c1"), "C", false);
        assert!(
            idle.tick(start + QUIET).is_empty(),
            "the child is busy: neither fires"
        );
        let end = start + QUIET * 2;
        idle.ended(&hop("c1"), end);
        assert_eq!(idle.tick(end + QUIET), [fired("C"), fired("M")]);
    }

    #[test]
    fn tests_that_detaching_a_busy_child_frees_its_parent() {
        let start = Instant::now();
        let mut idle = NetworkIdle::new("M", start);
        idle.attach("C", "M", start);
        idle.attach("G", "C", start);
        idle.started(hop("g1"), "G", false);
        assert!(idle.tick(start + QUIET).is_empty());
        assert_eq!(idle.detach("C"), [fired("M")], "the subtree went");
        idle.ended(&hop("g1"), start + QUIET * 2);
    }

    #[test]
    fn tests_that_a_commit_clears_networkidle_but_keeps_the_documents_own_request() {
        let start = Instant::now();
        let mut idle = NetworkIdle::new("M", start);
        assert_eq!(idle.tick(start + QUIET), [fired("M")]);
        let at = start + QUIET * 2;
        idle.started(hop("DOC"), "M", false);
        idle.started(hop("old-image"), "M", false);
        let changes = idle.commit("M", "DOC", at);
        assert_eq!(changes, [lost("M")]);
        assert_eq!(idle.deadline(), None, "the document's request still loads");
        idle.ended(&hop("DOC"), at);
        assert_eq!(idle.tick(at + QUIET), [fired("M")]);
    }

    #[test]
    fn tests_that_a_cleared_lifecycle_judges_idleness_without_the_old_documents_requests() {
        let start = Instant::now();
        let mut idle = NetworkIdle::new("M", start);
        idle.attach("C", "M", start);
        assert_eq!(idle.tick(start + QUIET), [fired("C"), fired("M")]);
        // The old content's image still loads when setContent reopens the document.
        idle.started(hop("old-image"), "M", false);
        let at = start + QUIET * 2;
        assert_eq!(idle.clear_lifecycle("M", "DOC", at), [lost("M")]);
        assert_eq!(
            idle.deadline(),
            Some(at + QUIET),
            "the old request no longer holds the frame"
        );
        assert_eq!(idle.tick(at + QUIET), [fired("M")]);
        idle.ended(&hop("old-image"), at + QUIET * 2);
        assert!(idle.fired("M"), "the old request's end changes nothing");
        assert!(
            idle.fired("C"),
            "unlike a commit, a clear keeps the child frames"
        );
    }

    #[test]
    fn tests_that_a_cleared_lifecycle_waits_for_the_written_documents_requests() {
        let start = Instant::now();
        let mut idle = NetworkIdle::new("M", start);
        assert_eq!(idle.tick(start + QUIET), [fired("M")]);
        let at = start + QUIET * 2;
        assert_eq!(idle.clear_lifecycle("M", "DOC", at), [lost("M")]);
        // The written document's fetch starts within the quiet period.
        idle.started(hop("fetch"), "M", false);
        assert!(idle.tick(at + QUIET * 4).is_empty(), "the fetch holds it");
        let end = at + QUIET * 4;
        idle.ended(&hop("fetch"), end);
        assert_eq!(idle.tick(end + QUIET), [fired("M")]);
        assert!(
            idle.clear_lifecycle("UNKNOWN", "DOC", end).is_empty(),
            "a frame the tracker does not know changes nothing"
        );
    }

    #[test]
    fn tests_that_a_commit_drops_the_frames_child_frames() {
        let start = Instant::now();
        let mut idle = NetworkIdle::new("M", start);
        idle.attach("C", "M", start);
        idle.started(hop("c1"), "C", false);
        idle.commit("M", "DOC", start);
        assert_eq!(
            idle.tick(start + QUIET),
            [fired("M")],
            "the old document's busy frame went with it"
        );
    }

    #[test]
    fn tests_that_a_new_request_loses_networkidle_at_the_next_recalculation() {
        let start = Instant::now();
        let mut idle = NetworkIdle::new("M", start);
        idle.attach("C", "M", start);
        assert_eq!(idle.tick(start + QUIET), [fired("C"), fired("M")]);
        idle.started(hop("1"), "M", false);
        assert!(idle.fired("M"), "a start alone recalculates nothing");
        assert_eq!(
            idle.lifecycle_event("C", "load"),
            [lost("M")],
            "the next recalculation finds the main frame busy"
        );
        assert!(
            idle.lifecycle_event("C", "load").is_empty(),
            "a lifecycle event recalculates once per document"
        );
    }

    #[test]
    fn tests_that_a_restart_judges_idleness_from_now() {
        let start = Instant::now();
        let mut idle = NetworkIdle::new("M", start);
        assert_eq!(idle.tick(start + QUIET), [fired("M")]);
        let enabled = start + QUIET * 3;
        assert_eq!(idle.restart(enabled), [lost("M")]);
        assert_eq!(idle.deadline(), Some(enabled + QUIET));
        assert_eq!(idle.tick(enabled + QUIET), [fired("M")]);
    }

    #[test]
    fn tests_that_a_lag_forgets_requests_whose_end_may_be_lost() {
        let start = Instant::now();
        let mut idle = NetworkIdle::new("M", start);
        idle.started(hop("1"), "M", false);
        assert_eq!(idle.deadline(), None);
        let lagged = start + QUIET;
        assert!(idle.forget_requests(lagged).is_empty());
        assert_eq!(idle.tick(lagged + QUIET), [fired("M")]);
        idle.ended(&hop("1"), lagged + QUIET * 2);
        assert!(
            idle.fired("M"),
            "a late end of a forgotten request changes nothing"
        );
    }

    #[test]
    fn tests_that_unknown_frames_and_requests_change_nothing() {
        let start = Instant::now();
        let mut idle = NetworkIdle::new("M", start);
        idle.started(hop("oopif"), "REMOTE", false);
        idle.attach("ORPHAN", "GONE", start);
        idle.ended(&hop("never-started"), start);
        assert_eq!(idle.tick(start + QUIET), [fired("M")]);
        assert!(!idle.fired("ORPHAN"));
    }
}
