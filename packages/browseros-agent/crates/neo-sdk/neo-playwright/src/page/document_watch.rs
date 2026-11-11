//! What one frame of a page did since a point in time: which documents committed and which load
//! states each reached, read from native CDP events. Child navigation follows the logical
//! frame across hosting sessions without replacing the original event subscription.
//!
//! ```text
//!  DocumentWatch::new ── subscribe ── enable lifecycle events ── send the command (goto, ...)
//!        │                                                            │
//!        └── Page.frameNavigated, Page.navigatedWithinDocument, ──────┴─▶ WatchedFrame
//!            Page.lifecycleEvent (the watched frame)                       (states per loader)
//! ```
//!
//! Chromium answers `Page.navigate` before the new document commits (measured: the reply at
//! 27 ms, the commit at 28 ms), so a navigation waits here for its own loader's commit. The
//! subscription is taken before the command is sent, so an event that races the reply is never
//! missed. `Page.setLifecycleEventsEnabled` replays the load states the current document already
//! reached each time it is called (measured), which is how `waitForLoadState` learns them.
//! A back-forward-cache restore does not fire those lifecycle events: its navigation event
//! supplies all reached states for the restored loader instead.

use std::collections::{HashMap, HashSet, VecDeque};
use std::iter;
use std::sync::Arc;

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{
    BrowserSession, CdpConnection, CoreError, FrameId, ProtocolSession, SessionId,
};
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::page::LoadState;

/// One operation's document history. Most waits observe one session; child goto retains
/// its original receiver and follows the logical frame across successor sessions.
pub(crate) struct DocumentWatch {
    /// Taken in [`DocumentWatch::new`], before the op sends anything.
    events: Receiver<CdpEvent>,
    connection: Arc<dyn CdpConnection>,
    /// Child goto keeps this same receiver across renderer changes. The containing target
    /// distinguishes closing the page from retiring an OOPIF's old target during a swap.
    containing_page: Option<FrameId>,
    /// A new hosting session must replay states that may have fired before its attachment.
    lifecycle_pending: bool,
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
    /// The frame left the watched session while a navigation was awaited: its document there
    /// was detached, as when it moves to another renderer or the page removes it, or the
    /// session itself went away. The frame may live on elsewhere (`waitForURL` follows it).
    Left,
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
    /// Failure URL from the current snapshot, including one read after a renderer swap.
    unreachable: Option<String>,
    /// The frame left the watched session: `Page.frameDetached` named it there, either reason
    /// (`swap`, `remove`), or the session detached. Navigation callers re-resolve this frame
    /// before continuing a commit or lifecycle wait.
    left: bool,
    /// A later navigation has started but not committed. Renderer metadata reads can block
    /// behind it; a completed navigation must not wait for that replacement's response.
    pending: bool,
    /// The current document was restored already loaded. Missing subsequent observer work
    /// belongs to Neo; the website has no new load events to produce for this document.
    cache_restore: bool,
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
            connection: browser.connection(),
            containing_page: None,
            lifecycle_pending: false,
            session,
            session_id,
            frame_id,
            frame: WatchedFrame::default(),
        }
    }

    /// Follow this child by its stable `FrameId` on every native session. Keeping the original
    /// receiver preserves commits queued during handoff, including immediate redirects that
    /// a newly subscribed watch could never recover from the current frame tree.
    pub(crate) fn follow_frame_events(&mut self, containing_page: FrameId) {
        self.containing_page = Some(containing_page);
    }

    /// Subscribe before dispatch: setup on an old child session can lose its reply when that
    /// target retires. This separate cursor cancels only setup; the original commit cursor stays.
    pub(crate) fn session_retired(&self) -> BoxFuture<'static, ()> {
        let mut events = self.connection.events();
        let session = self.session_id.clone();
        let frame = self.frame_id.clone();
        Box::pin(async move {
            loop {
                let Ok(event) = events.recv().await else {
                    return;
                };
                let detached = event.method == "Target.detachedFromTarget"
                    && event.params.get("sessionId").and_then(Value::as_str)
                        == Some(session.as_str());
                let moved = event.method == "Page.frameDetached"
                    && event.session_id.as_ref() == Some(&session)
                    && event.params.get("frameId").and_then(Value::as_str) == Some(frame.as_str());
                if detached || moved {
                    return;
                }
            }
        })
    }

    /// A cancelled old-host setup must replay on the host selected by the commit instead.
    pub(crate) fn replay_lifecycle(&mut self) {
        self.lifecycle_pending = true;
    }

    /// The frame's URL as last reported, if the watch saw one.
    pub(crate) fn url(&self) -> Option<&str> {
        self.frame.url.as_deref()
    }

    /// The document last read or committed on this frame.
    pub(crate) fn loader(&self) -> Option<&str> {
        self.frame.current.as_deref()
    }

    /// Chromium restored the current document with every load state already reached.
    pub(crate) fn restored_from_cache(&self) -> bool {
        self.frame.cache_restore
    }

    /// The current endpoint, which a child navigation can replace when its renderer swaps.
    pub(crate) fn session(&self) -> &ProtocolSession {
        &self.session
    }

    /// Waits for a subsequent navigation intent while preserving all lifecycle events read
    /// along the way. Used to cancel an optional timing read, never the navigation itself.
    pub(crate) async fn navigation_started(&mut self) -> Result<(), WatchError> {
        while !self.frame.pending {
            self.next().await?;
        }
        Ok(())
    }

    /// Turns on the `Page` domain for the session, for a watch whose frame may be an
    /// out-of-process iframe's. Chromium sends an OOPIF's `documentOpened`, `frameNavigated` and
    /// lifecycle events only on a session with `Page` enabled (measured, spike
    /// S-setcontent-oopif: none arrive without it, all of them with it), and core enables it
    /// there only on an agent's own tab, for its file chooser interception; on a page's own
    /// session core enabled it already, and a second enable changes nothing. Sent before the
    /// watch reads the frame, so a navigation between that read and the wait is still seen.
    ///
    /// It lives here, under the op's timeout, rather than in a sealed world's session setup,
    /// whose calls spend a frame's install budget.
    pub(crate) async fn enable_page(&self) -> Result<(), WatchError> {
        self.session.send_value("Page.enable", json!({})).await?;
        Ok(())
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
            self.frame.unreachable = frame.unreachable_url;
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
            // A replacement session may have committed before Page was enabled there.
            // Its current snapshot is sufficient only for this exact expected loader.
            if self.frame.current.as_deref() == Some(loader)
                && let Some(url) = &self.frame.url
            {
                return Ok(CommitOutcome::Committed(Commit {
                    loader: loader.to_owned(),
                    url: url.clone(),
                    unreachable: self.frame.unreachable.clone(),
                }));
            }
            if self.frame.left {
                return Err(WatchError::Left);
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
    /// inside the current one, each once and in order. [`WatchError::Left`] once the frame left
    /// the session with no navigation still to hand out: a navigation that commits in another
    /// renderer is never seen here.
    pub(crate) async fn navigated(&mut self) -> Result<Navigated, WatchError> {
        loop {
            if let Some(navigated) = self.frame.navigations.pop_front() {
                return Ok(navigated);
            }
            if self.frame.left {
                return Err(WatchError::Left);
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
            if self.containing_page.is_some() && self.frame.left {
                return Err(WatchError::Left);
            }
            if self.frame.has_reached(state) {
                return Ok(());
            }
            if self.frame.left {
                return Err(WatchError::Left);
            }
            if self.lifecycle_pending {
                self.lifecycle_pending = false;
                self.enable_lifecycle().await?;
            }
            self.next().await?;
        }
    }

    /// Applies the next event of the watched frame.
    async fn next(&mut self) -> Result<(), WatchError> {
        match self.events.recv().await {
            Ok(event) => {
                if self.containing_page.is_some() && self.followed_event(&event) {
                    return Ok(());
                }
                if event.session_id.as_ref() == Some(&self.session_id) {
                    self.frame.apply(&event, &self.frame_id);
                } else if event.method == "Target.detachedFromTarget"
                    && event.params.get("sessionId").and_then(Value::as_str)
                        == Some(self.session_id.as_str())
                {
                    // Sent on the session that attached ours, or the browser's, never on ours:
                    // an out-of-process frame whose renderer it left.
                    self.frame.left = true;
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

    /// A child can commit on its parent's session after an OOPIF disappears. Its `FrameId`,
    /// not the retired session, correlates the native event; loader ids still select the
    /// dispatched navigation. Only actual removal or closing its containing page ends it.
    fn followed_event(&mut self, event: &CdpEvent) -> bool {
        let names_frame = event
            .params
            .get("frameId")
            .and_then(Value::as_str)
            .or_else(|| event.params.get("frame")?.get("id")?.as_str())
            == Some(self.frame_id.as_str());
        if names_frame && event.method.starts_with("Page.") {
            if event.method == "Page.frameDetached" {
                if event.params.get("reason").and_then(Value::as_str) != Some("swap")
                    || event.session_id.as_ref() == Some(&self.session_id)
                {
                    self.frame.left = true;
                }
            } else {
                if event.method == "Page.frameNavigated"
                    && let Some(session) = &event.session_id
                {
                    self.rehost(session.clone());
                }
                self.frame.apply(event, &self.frame_id);
            }
            return true;
        }
        if event.method == "Target.targetDestroyed"
            && event.params.get("targetId").and_then(Value::as_str)
                == self.containing_page.as_ref().map(FrameId::as_str)
        {
            self.frame.left = true;
            return true;
        }
        if event.method == "Target.detachedFromTarget" {
            if event.params.get("sessionId").and_then(Value::as_str)
                == Some(self.session_id.as_str())
            {
                self.frame.left = true;
            }
            return true;
        }
        false
    }

    fn rehost(&mut self, session: SessionId) {
        if session != self.session_id {
            self.session =
                ProtocolSession::for_session(Arc::clone(&self.connection), session.clone());
            self.session_id = session;
            self.lifecycle_pending = true;
        }
    }

    /// Adopt the resolved host while retaining the original cursor and loader history. Drain
    /// handoff events before consulting the snapshot: an immediate redirect must not replace
    /// an already queued commit. A genuinely newer snapshot settles an interrupted loader;
    /// the old pre-dispatch document is not a new commit.
    pub(crate) async fn resume_on(&mut self, session: ProtocolSession) -> Result<(), WatchError> {
        self.session_id = session
            .session_id()
            .cloned()
            .ok_or(WatchError::FrameDetached)?;
        self.session = session;
        self.frame.left = false;
        self.lifecycle_pending = false;
        self.enable_page().await?;
        self.enable_lifecycle().await?;
        for _ in 0..self.events.len() {
            match self.events.try_recv() {
                Ok(event) => {
                    self.followed_event(&event);
                }
                Err(TryRecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
        if self.frame.left {
            return Ok(());
        }
        let current = self.read_frame().await?;
        if !current.loader_id.is_empty()
            && self.frame.current.as_deref() != Some(current.loader_id.as_str())
        {
            self.frame.commit(Commit {
                loader: current.loader_id,
                url: with_fragment(current.url, current.url_fragment.as_deref()),
                unreachable: current.unreachable_url,
            });
        }
        Ok(())
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
            "Page.frameRequestedNavigation" | "Page.frameStartedNavigating" => {
                if let Some(Detached { frame_id: id }) = parse(&event.params)
                    && id == frame_id.as_str()
                {
                    self.pending = true;
                }
            }
            "Page.frameNavigated" => {
                if let Some(FrameNavigated { frame, kind }) = parse(&event.params)
                    && frame.id == frame_id.as_str()
                {
                    self.commit_event(frame, kind.as_deref() == Some("BackForwardCacheRestore"));
                }
            }
            // `document.open()` (setContent) keeps the loader but starts the document over:
            // Chromium fires `init`, `DOMContentLoaded` and `load` again under that loader
            // (measured), never `networkIdle`.
            "Page.documentOpened" => {
                if let Some(FrameNavigated { frame, .. }) = parse(&event.params)
                    && frame.id == frame_id.as_str()
                {
                    self.reopen(frame.loader_id);
                }
            }
            "Page.frameDetached" => {
                if let Some(Detached { frame_id: id }) = parse(&event.params)
                    && id == frame_id.as_str()
                {
                    self.left = true;
                }
            }
            "Page.navigatedWithinDocument" => {
                if let Some(WithinDocument { frame_id: id, url }) = parse(&event.params)
                    && id == frame_id.as_str()
                {
                    self.same_document += 1;
                    self.pending = false;
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

    /// A cached document resumes without `DOMContentLoaded`, load or `networkIdle` events.
    /// Keep these facts under its loader so another document cannot borrow its readiness.
    fn commit_event(&mut self, frame: FrameFields, restored: bool) {
        let loader = frame.loader_id;
        self.commit(Commit {
            loader: loader.clone(),
            url: with_fragment(frame.url, frame.url_fragment.as_deref()),
            unreachable: frame.unreachable_url,
        });
        if restored {
            self.cache_restore = true;
            self.reached.entry(loader).or_default().extend([
                LoadState::Commit,
                LoadState::DomContentLoaded,
                LoadState::Load,
                LoadState::NetworkIdle,
            ]);
        }
    }

    fn commit(&mut self, commit: Commit) {
        self.cache_restore = false;
        self.pending = false;
        self.unreachable.clone_from(&commit.unreachable);
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
        self.cache_restore = false;
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

/// `Page.frameDetached`, reduced to what the watch reads.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Detached {
    frame_id: String,
}

/// `Page.frameNavigated`, reduced to what the watch reads.
#[derive(Deserialize)]
struct FrameNavigated {
    frame: FrameFields,
    #[serde(default, rename = "type")]
    kind: Option<String>,
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
    fn cached_restore_reaches_every_state_without_lifecycle_events() {
        let main = FrameId("M".to_owned());
        let mut frame = WatchedFrame::default();
        let mut restored = navigated("M", "cached", None);
        restored.params["type"] = json!("BackForwardCacheRestore");
        frame.apply(&restored, &main);
        assert!(frame.cache_restore);
        for state in [
            LoadState::Commit,
            LoadState::DomContentLoaded,
            LoadState::Load,
            LoadState::NetworkIdle,
        ] {
            assert!(frame.has_reached(state), "{state}");
        }
        frame.apply(&navigated("M", "next", None), &main);
        assert!(!frame.cache_restore);
        assert!(!frame.has_reached(LoadState::Load));
        frame.apply(&restored, &main);
        frame.reopen("cached".to_owned());
        assert!(!frame.cache_restore);
        assert!(!frame.has_reached(LoadState::Load));
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
