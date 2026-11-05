//! What one frame of a page did since a point in time: which documents committed and which load
//! states each reached, read from the CDP events of the session that hosts it.
//!
//! ```text
//!  DocumentWatch::new ── subscribe ── enable lifecycle events ── send the command (goto, ...)
//!        │                                                            │
//!        └── Page.frameNavigated, Page.navigatedWithinDocument, ──────┴─▶ WatchedFrame
//!            Page.lifecycleEvent (the watched frame on this session)       (states per loader)
//! ```
//!
//! Chromium answers `Page.navigate` before the new document commits (measured: the reply at
//! 27 ms, the commit at 28 ms), so a navigation waits here for its own loader's commit. The
//! subscription is taken before the command is sent, so an event that races the reply is never
//! missed. `Page.setLifecycleEventsEnabled` replays the load states the current document already
//! reached each time it is called (measured), which is how `waitForLoadState` learns them.

use std::collections::{HashMap, HashSet, VecDeque};
use std::iter;

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{BrowserSession, CoreError, FrameId, ProtocolSession, SessionId};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;

use crate::page::LoadState;

/// One op's view of one frame (the main frame, unless `frame.waitForLoadState` names another),
/// from just before its command to the op's end.
pub(crate) struct DocumentWatch {
    /// Taken in [`DocumentWatch::new`], before the op sends anything.
    events: Receiver<CdpEvent>,
    /// The session that hosts the frame, raw: the op already admitted the page.
    session: ProtocolSession,
    session_id: SessionId,
    /// The watched frame. A tab's main frame id is its target id.
    frame_id: FrameId,
    frame: WatchedFrame,
}

/// Why a watch could not answer.
#[derive(Debug)]
pub(crate) enum WatchError {
    /// CDP failed, or the connection went away.
    Core(CoreError),
    /// The frame is no longer in its session's frame tree.
    FrameDetached,
}

impl From<CoreError> for WatchError {
    fn from(error: CoreError) -> Self {
        Self::Core(error)
    }
}

/// What the watch learned about its frame.
#[derive(Debug, Default)]
struct WatchedFrame {
    /// Load states reached, per document (loader id). Holds only the documents of one op.
    reached: HashMap<String, HashSet<LoadState>>,
    /// Every document committed since the watch started, oldest first.
    commits: Vec<Commit>,
    /// The document the frame shows now: the latest commit, or the one read at the start.
    current: Option<String>,
    /// Navigations inside the current document (fragments, `pushState`, same-document history).
    same_document: usize,
    /// Times the frame's document was opened for writing (`document.open()`, setContent).
    opened: usize,
    /// Every navigation not yet taken by [`DocumentWatch::navigated`], oldest first.
    navigations: VecDeque<Navigated>,
    /// The frame's URL as last reported, for the prelude's synchronous `page.url()`.
    url: Option<String>,
}

/// A document the frame committed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Commit {
    /// Chromium's loader id: `Page.navigate` returns it for the navigation it starts.
    pub(crate) loader: String,
    /// The document's URL, fragment included.
    pub(crate) url: String,
    /// The URL that failed to load, when Chromium committed its error page instead.
    pub(crate) unreachable: Option<String>,
}

/// How the document a command started fared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CommitOutcome {
    /// It committed.
    Committed(Commit),
    /// Another document committed first, so the navigation was interrupted, as upstream reports
    /// it. Carries the URL that committed instead.
    Interrupted(String),
}

/// How a navigation the op did not start by loader id arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Arrival {
    /// A new document committed.
    NewDocument(Commit),
    /// The frame navigated inside its current document.
    SameDocument,
}

/// One navigation of the watched frame: a new document or a move inside the current one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Navigated {
    /// Where it went, fragment included.
    pub(crate) url: String,
    /// The URL that failed to load, when Chromium committed its error page instead.
    pub(crate) unreachable: Option<String>,
}

/// Which navigations count as the one a history step or a reload started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Expect {
    /// `reload`: only a new document, never a stray `pushState`.
    NewDocument,
    /// `goBack`, `goForward`: a same-document history entry counts too.
    AnyNavigation,
}

impl DocumentWatch {
    /// Subscribes to the events of `session`, which hosts `frame_id`. Build it before sending
    /// the command it watches.
    pub(crate) fn new(
        browser: &BrowserSession,
        session: ProtocolSession,
        session_id: SessionId,
        frame_id: FrameId,
    ) -> Self {
        Self {
            events: browser.cdp_events(),
            session,
            session_id,
            frame_id,
            frame: WatchedFrame::default(),
        }
    }

    /// The frame's URL as last reported, if the watch saw one.
    pub(crate) fn url(&self) -> Option<&str> {
        self.frame.url.as_deref()
    }

    /// Turns on `Page.lifecycleEvent` for the session. Chromium then replays the states the
    /// current document already reached, so a wait never misses one that happened earlier.
    pub(crate) async fn enable_lifecycle(&self) -> Result<(), WatchError> {
        self.session
            .send_value("Page.setLifecycleEventsEnabled", json!({ "enabled": true }))
            .await?;
        Ok(())
    }

    /// Reads which document the frame shows now, for waits that are about the current document
    /// rather than one the op starts.
    pub(crate) async fn read_current(&mut self) -> Result<(), WatchError> {
        let frame = self.read_frame().await?;
        // A commit seen meanwhile is newer than the tree's answer.
        if self.frame.commits.is_empty() {
            self.frame
                .reached
                .entry(frame.loader_id.clone())
                .or_default();
            self.frame.current = Some(frame.loader_id);
            self.frame.url = Some(with_fragment(frame.url, frame.url_fragment.as_deref()));
        }
        Ok(())
    }

    /// Waits for document `loader` to commit.
    pub(crate) async fn committed(&mut self, loader: &str) -> Result<CommitOutcome, WatchError> {
        loop {
            if let Some(first) = self.frame.commits.first() {
                return Ok(if first.loader == loader {
                    CommitOutcome::Committed(first.clone())
                } else {
                    CommitOutcome::Interrupted(first.url.clone())
                });
            }
            self.next().await?;
        }
    }

    /// Waits for the first navigation `expect` accepts.
    pub(crate) async fn arrived(&mut self, expect: Expect) -> Result<Arrival, WatchError> {
        loop {
            if let Some(first) = self.frame.commits.first() {
                return Ok(Arrival::NewDocument(first.clone()));
            }
            if expect == Expect::AnyNavigation && self.frame.same_document > 0 {
                return Ok(Arrival::SameDocument);
            }
            self.next().await?;
        }
    }

    /// Waits for the frame's next navigation since the watch started, a new document or a move
    /// inside the current one, each once and in order.
    pub(crate) async fn navigated(&mut self) -> Result<Navigated, WatchError> {
        loop {
            if let Some(navigated) = self.frame.navigations.pop_front() {
                return Ok(navigated);
            }
            self.next().await?;
        }
    }

    /// Waits until the frame's document was opened for writing since the watch started. The
    /// load states a reopened document reaches count only after it, and the event can still be
    /// queued when the write's reply arrives, so a load-state wait must come after this one.
    pub(crate) async fn opened(&mut self) -> Result<(), WatchError> {
        while self.frame.opened == 0 {
            self.next().await?;
        }
        Ok(())
    }

    /// Waits until the document the frame shows reaches `state`. A document that commits
    /// meanwhile becomes the one waited on, as upstream resets its fired states on a commit.
    pub(crate) async fn reached(&mut self, state: LoadState) -> Result<(), WatchError> {
        loop {
            if self.frame.has_reached(state) {
                return Ok(());
            }
            self.next().await?;
        }
    }

    /// Applies the next event of the watched frame.
    async fn next(&mut self) -> Result<(), WatchError> {
        match self.events.recv().await {
            Ok(event) => {
                if event.session_id.as_ref() == Some(&self.session_id) {
                    self.frame.apply(&event, &self.frame_id);
                }
                Ok(())
            }
            // Events were dropped, ours perhaps among them. Ask again: the frame tree names the
            // current document and the replay restores what it reached. A commit lost in the
            // gap is then the current document, which every wait above accepts.
            Err(RecvError::Lagged(_)) => {
                let frame = self.read_frame().await?;
                if self.frame.current.as_deref() != Some(frame.loader_id.as_str()) {
                    self.frame.commit(Commit {
                        loader: frame.loader_id,
                        url: with_fragment(frame.url, frame.url_fragment.as_deref()),
                        unreachable: frame.unreachable_url,
                    });
                }
                self.enable_lifecycle().await
            }
            Err(RecvError::Closed) => {
                Err(WatchError::Core(CoreError::Cdp(CdpError::ConnectionLost)))
            }
        }
    }

    /// The watched frame as `Page.getFrameTree` reports it now.
    async fn read_frame(&self) -> Result<FrameFields, WatchError> {
        let tree: FrameTreeReply = decode(
            self.session
                .send_value("Page.getFrameTree", json!({}))
                .await?,
        )?;
        tree.frame_tree
            .into_frames()
            .find(|frame| frame.id == self.frame_id.as_str())
            .ok_or(WatchError::FrameDetached)
    }
}

impl WatchedFrame {
    fn apply(&mut self, event: &CdpEvent, frame_id: &FrameId) {
        match event.method.as_str() {
            "Page.frameNavigated" => {
                if let Some(FrameNavigated { frame }) = parse(&event.params)
                    && frame.id == frame_id.as_str()
                {
                    self.commit(Commit {
                        loader: frame.loader_id,
                        url: with_fragment(frame.url, frame.url_fragment.as_deref()),
                        unreachable: frame.unreachable_url,
                    });
                }
            }
            // `document.open()` (setContent) keeps the loader but starts the document over:
            // Chromium fires `init`, `DOMContentLoaded` and `load` again under that loader
            // (measured), never `networkIdle`.
            "Page.documentOpened" => {
                if let Some(FrameNavigated { frame }) = parse(&event.params)
                    && frame.id == frame_id.as_str()
                {
                    self.reopen(frame.loader_id);
                }
            }
            "Page.navigatedWithinDocument" => {
                if let Some(WithinDocument { frame_id: id, url }) = parse(&event.params)
                    && id == frame_id.as_str()
                {
                    self.same_document += 1;
                    self.navigations.push_back(Navigated {
                        url: url.clone(),
                        unreachable: None,
                    });
                    self.url = Some(url);
                }
            }
            "Page.lifecycleEvent" => {
                if let Some(Lifecycle {
                    frame_id: id,
                    loader_id,
                    name,
                }) = parse(&event.params)
                    && id == frame_id.as_str()
                    && let Some(state) = LoadState::from_lifecycle(&name)
                {
                    self.reached.entry(loader_id).or_default().insert(state);
                }
            }
            _ => {}
        }
    }

    fn commit(&mut self, commit: Commit) {
        self.reached
            .entry(commit.loader.clone())
            .or_default()
            .insert(LoadState::Commit);
        self.current = Some(commit.loader.clone());
        self.url = Some(commit.url.clone());
        self.navigations.push_back(Navigated {
            url: commit.url.clone(),
            unreachable: commit.unreachable.clone(),
        });
        self.commits.push(commit);
    }

    /// The frame's document was opened for writing: the same loader, a new document's states.
    fn reopen(&mut self, loader: String) {
        self.reached
            .insert(loader.clone(), HashSet::from([LoadState::Commit]));
        self.current = Some(loader);
        self.opened += 1;
    }

    fn has_reached(&self, state: LoadState) -> bool {
        self.current
            .as_ref()
            .and_then(|loader| self.reached.get(loader))
            .is_some_and(|reached| reached.contains(&state))
    }
}

/// `Page.frameNavigated`, reduced to what the watch reads.
#[derive(Deserialize)]
struct FrameNavigated {
    frame: FrameFields,
}

/// The fields of CDP's `Page.Frame` the crate reads. CDP adds fields over time, so unknown ones
/// are ignored here, unlike in the prelude's inputs.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrameFields {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) parent_id: Option<String>,
    pub(crate) loader_id: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) url_fragment: Option<String>,
    #[serde(default)]
    pub(crate) unreachable_url: Option<String>,
}

/// `Page.getFrameTree`'s reply.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrameTreeReply {
    pub(crate) frame_tree: FrameTreeNode,
}

/// One frame and the same-process frames under it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrameTreeNode {
    pub(crate) frame: FrameFields,
    #[serde(default)]
    pub(crate) child_frames: Vec<FrameTreeNode>,
}

impl FrameTreeNode {
    /// Every frame of the tree, each parent before its children.
    pub(crate) fn into_frames(self) -> impl Iterator<Item = FrameFields> {
        let mut pending = vec![self];
        iter::from_fn(move || {
            let node = pending.pop()?;
            pending.extend(node.child_frames.into_iter().rev());
            Some(node.frame)
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithinDocument {
    frame_id: String,
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Lifecycle {
    frame_id: String,
    loader_id: String,
    name: String,
}

/// `Page.Frame` names its URL without the fragment, and the fragment apart.
fn with_fragment(mut url: String, fragment: Option<&str>) -> String {
    url.push_str(fragment.unwrap_or_default());
    url
}

/// An event's params as `T`. An event of the right name that does not decode is ignored, like
/// any other event the watch does not need.
fn parse<T: DeserializeOwned>(params: &Value) -> Option<T> {
    T::deserialize(params).ok()
}

/// A CDP reply as `T`, or core's error for a reply Neo cannot read.
pub(crate) fn decode<T: DeserializeOwned>(reply: Value) -> Result<T, CoreError> {
    serde_json::from_value(reply).map_err(|error| CoreError::Message(error.to_string()))
}

#[cfg(test)]
mod tests {
    use browseros_cdp::CdpEvent;
    use browseros_core::{FrameId, SessionId};
    use serde_json::{Value, json};

    use super::{Commit, FrameTreeReply, WatchedFrame};
    use crate::page::LoadState;

    fn event(method: &str, params: Value) -> CdpEvent {
        CdpEvent {
            method: method.to_owned(),
            params,
            session_id: Some(SessionId::from("S".to_owned())),
        }
    }

    fn navigated(id: &str, loader: &str, parent: Option<&str>) -> CdpEvent {
        event(
            "Page.frameNavigated",
            json!({ "frame": { "id": id, "parentId": parent, "loaderId": loader,
                               "url": "https://a.test/", "urlFragment": "#x" } }),
        )
    }

    fn lifecycle(frame: &str, loader: &str, name: &str) -> CdpEvent {
        event(
            "Page.lifecycleEvent",
            json!({ "frameId": frame, "loaderId": loader, "name": name, "timestamp": 1.0 }),
        )
    }

    fn commit(loader: &str) -> Commit {
        Commit {
            loader: loader.to_owned(),
            url: "https://a.test/#x".to_owned(),
            unreachable: None,
        }
    }

    #[test]
    fn tests_that_a_load_state_counts_only_for_the_document_that_reached_it() {
        let main = FrameId("M".to_owned());
        let mut frame = WatchedFrame::default();
        frame.apply(&lifecycle("M", "L1", "init"), &main);
        frame.apply(&navigated("M", "L1", None), &main);
        frame.apply(&lifecycle("M", "L1", "DOMContentLoaded"), &main);
        assert!(frame.has_reached(LoadState::DomContentLoaded));
        assert!(!frame.has_reached(LoadState::Load));
        frame.apply(&navigated("M", "L2", None), &main);
        assert!(
            !frame.has_reached(LoadState::DomContentLoaded),
            "a new commit resets the states, as upstream does"
        );
        frame.apply(&lifecycle("M", "L1", "load"), &main);
        assert!(
            !frame.has_reached(LoadState::Load),
            "a late event of the old document is not the new one's"
        );
        assert_eq!(frame.commits, [commit("L1"), commit("L2")]);
        assert_eq!(frame.url.as_deref(), Some("https://a.test/#x"));
    }

    #[test]
    fn tests_that_an_opened_document_reaches_its_states_again_under_the_same_loader() {
        let main = FrameId("M".to_owned());
        let mut frame = WatchedFrame::default();
        frame.apply(&navigated("M", "L1", None), &main);
        frame.apply(&lifecycle("M", "L1", "load"), &main);
        let opened = event(
            "Page.documentOpened",
            json!({ "frame": { "id": "M", "loaderId": "L1", "url": "https://a.test/" } }),
        );
        frame.apply(&opened, &main);
        assert!(
            frame.has_reached(LoadState::Commit),
            "the written document is there"
        );
        assert!(
            !frame.has_reached(LoadState::Load),
            "the old content's load is not the new content's"
        );
        frame.apply(&lifecycle("M", "L1", "load"), &main);
        assert!(frame.has_reached(LoadState::Load));
        assert_eq!(
            frame.commits,
            [commit("L1")],
            "an opened document is no navigation"
        );
    }

    #[test]
    fn tests_that_every_navigation_is_taken_once_in_order() {
        let main = FrameId("M".to_owned());
        let mut frame = WatchedFrame::default();
        frame.apply(&navigated("M", "L1", None), &main);
        frame.apply(
            &event(
                "Page.navigatedWithinDocument",
                json!({ "frameId": "M", "url": "https://a.test/#y" }),
            ),
            &main,
        );
        let urls: Vec<String> = frame.navigations.drain(..).map(|next| next.url).collect();
        assert_eq!(
            urls,
            ["https://a.test/#x", "https://a.test/#y"],
            "a new document and a move inside it, as the frame made them"
        );
    }

    #[test]
    fn tests_that_other_frames_never_move_the_watched_one() {
        let main = FrameId("M".to_owned());
        let mut frame = WatchedFrame::default();
        frame.apply(&navigated("C", "L9", Some("M")), &main);
        frame.apply(&lifecycle("C", "L9", "load"), &main);
        frame.apply(
            &event(
                "Page.navigatedWithinDocument",
                json!({ "frameId": "C", "url": "https://a.test/#y" }),
            ),
            &main,
        );
        assert!(frame.commits.is_empty());
        assert_eq!(frame.same_document, 0);
        assert!(!frame.has_reached(LoadState::Load));
    }

    #[test]
    fn tests_that_a_child_frame_can_be_the_watched_one() {
        let child = FrameId("C".to_owned());
        let mut frame = WatchedFrame::default();
        frame.apply(&navigated("C", "L9", Some("M")), &child);
        frame.apply(&lifecycle("C", "L9", "load"), &child);
        assert!(frame.has_reached(LoadState::Load));
    }

    #[test]
    fn tests_that_the_tree_lists_parents_before_children() -> Result<(), serde_json::Error> {
        let frame = |id: &str| json!({ "id": id, "loaderId": "L", "url": "about:blank" });
        let reply: FrameTreeReply = serde_json::from_value(json!({ "frameTree": {
            "frame": frame("M"),
            "childFrames": [
                { "frame": frame("A"), "childFrames": [{ "frame": frame("A1") }] },
                { "frame": frame("B") },
            ],
        } }))?;
        let ids: Vec<String> = reply
            .frame_tree
            .into_frames()
            .map(|frame| frame.id)
            .collect();
        assert_eq!(ids, ["M", "A", "A1", "B"]);
        Ok(())
    }
}
