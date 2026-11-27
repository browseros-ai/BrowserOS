//! A tab's frames, out-of-process ones included. `page.frames()`, for the prelude's synchronous
//! `frames()`, `mainFrame()`, `parentFrame()` and `childFrames()`, answers from the target
//! watcher's list (`TabFrames`) and sends the tab nothing ([`frames`]). The ops that act on a
//! tab's frames (evaluate, tags, navigations, the resolver's walk into a child frame) still read
//! the tree from the renderer here ([`read`], [`snapshot`]), on the tab they admitted.
//!
//! A session's `Page.getFrameTree` lists only the frames in its own renderer. Each
//! out-of-process iframe is a target of its own whose `parentFrameId` names the frame that
//! embeds it, so a read goes session by session:
//!
//! ```text
//!  page session ── getFrameTree ──▶ main frame + same-process children
//!  Target.getTargets (iframes) ── parentFrameId already in the tree? ──▶ that OOPIF's session
//!     (core's FrameRegistry) ── getFrameTree ──▶ its frames; repeat for OOPIFs inside OOPIFs
//! ```

use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Duration;

use browseros_core::{
    BrowserSession, CoreError, FrameId, PageId, ProtocolSession, TabId, TargetId,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time;
use ts_rs::TS;

use crate::events::EventScope;
use crate::events::tab_frames::{ListedFrame, TabFrameList};
use crate::op_scope::OpScope;
use crate::page::document_watch::{DocumentWatch, FrameTreeNode, FrameTreeReply, decode};
use crate::page::frame_routes::{FrameRoute, RootAttachment};
use crate::target::PageRef;
use crate::wire::options;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// Loading can mean an uncommitted navigation that blocks renderer reads, or just a child
/// resource in an otherwise responsive document. Keep those optional reads short without
/// hiding every newly attached child after `setContent`.
const LOADING_SNAPSHOT: Duration = Duration::from_millis(100);

/// Even a fully loaded page can contain a child that never answers. Allow ordinary discovery
/// more time than an active navigation, but retain cached identity and route hints when a
/// busy child prevents a complete refresh.
const SNAPSHOT_BUDGET: Duration = Duration::from_secs(1);

/// `page.frames()`: `{ page }`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct FramesInput {
    page: PageRef,
}
options!(FramesInput);

/// `page.frames()`'s answer: the tab's frames, the main frame first, each parent before its
/// children. Absent when the bounded read did not finish, so the prelude keeps the frames it
/// has.
// A plain optional field where a flattened `Option<FrameTree>` was (ts-rs cannot print a
// flattened `Option`): the same JSON, pinned by `tests::tests_that_the_frame_answer_keeps_its_bytes`.
#[derive(Debug, Serialize, TS)]
pub(crate) struct PageFrames {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    frames: Option<Vec<FrameInfo>>,
    /// `true` when an out-of-process frame under the tree could not be read: `frames` then lacks
    /// it (and what it holds). The operation's record is marked `framesIncomplete` for the host,
    /// and the server's warn line names each such frame and its error.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    incomplete: Option<bool>,
}

/// A page's frames, each parent before its children; the first is the main frame.
#[derive(Debug, Default, Serialize)]
pub(crate) struct FrameTree {
    frames: Vec<FrameInfo>,
    #[serde(skip)]
    hosts: HashMap<FrameId, ProtocolSession>,
    /// The out-of-process frames the browser lists under this tree whose own tree could not be
    /// read, and why: the tree lacks them, so it is incomplete rather than whole.
    #[serde(skip)]
    unread: Vec<(String, String)>,
}

/// Where one read of a page's frames places a frame ([`FrameTree::place`]).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FramePlace {
    /// The read lists it: the frames from the main frame down to it, the main frame left out.
    Listed(Vec<FrameId>),
    /// A whole read does not list it: it was detached.
    Absent,
    /// A read that is not whole does not list it: it may be inside an out-of-process frame the
    /// read could not read, so the read says nothing about it.
    Unread,
}

/// Stable identity for one document when validating subtree lifecycle readiness.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FrameDocument {
    pub(crate) frame: FrameId,
    pub(crate) loader: String,
}

struct HostedTree {
    tree: FrameTreeReply,
    session: ProtocolSession,
}

/// One frame, as Playwright's `Frame` reports it.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrameInfo {
    id: FrameId,
    /// `None` for the main frame.
    parent_id: Option<FrameId>,
    /// The frame element's `name`, `""` without one.
    name: String,
    url: String,
    /// Correlates lifecycle snapshots across reads; never exposed as a public Frame field.
    #[serde(skip)]
    loader: String,
}

/// The out-of-process iframes of the browser, as `Target.getTargets` lists them.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Targets {
    target_infos: Vec<IframeTarget>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IframeTarget {
    /// An OOPIF's frame id is its target id.
    target_id: String,
    #[serde(default)]
    parent_frame_id: Option<String>,
}

impl FrameTree {
    /// The frames as the prelude reads them, main frame first; the sessions stay behind.
    #[cfg(test)]
    pub(crate) fn into_frames(self) -> Vec<FrameInfo> {
        self.frames
    }

    /// Whether every out-of-process frame the browser lists under the tree was read.
    pub(crate) fn is_whole(&self) -> bool {
        self.unread.is_empty()
    }

    /// The frames for a reply that supplements the prelude's tree (an evaluate's, a tag's, a
    /// navigation's): `None` unless the tree is whole, so a partial tree never replaces a whole
    /// one the prelude holds and detaches the frames it lacks. The prelude reads the tree again
    /// with `page.frames`, whose answer marks a partial tree `incomplete`.
    pub(crate) fn into_whole_frames(self) -> Option<Vec<FrameInfo>> {
        self.is_whole().then_some(self.frames)
    }

    /// The documents under `root`, including itself, in stable identity order. A replacement
    /// document must reach its own lifecycle state even if its Frame object was retained.
    pub(crate) fn documents(&self, root: &FrameId) -> Vec<FrameDocument> {
        let main = self.frames.first().is_some_and(|frame| frame.id == *root);
        let mut documents: Vec<_> = self
            .frames
            .iter()
            .filter(|frame| {
                main || self
                    .path_to(&frame.id)
                    .is_some_and(|path| path.contains(root))
            })
            .map(|frame| FrameDocument {
                frame: frame.id.clone(),
                loader: frame.loader.clone(),
            })
            .collect();
        documents.sort_by(|a, b| a.frame.as_str().cmp(b.frame.as_str()));
        documents
    }

    /// Frame identities for page-wide operations; worlds are resolved on each frame's own
    /// CDP session, so the JavaScript never crosses renderer or origin boundaries.
    pub(crate) fn ids(&self) -> impl Iterator<Item = &FrameId> {
        self.frames.iter().map(|frame| &frame.id)
    }

    /// The endpoint found through this page's ancestry, including same-process descendants
    /// of OOPIFs. A global frame lookup without this host would fall back to the page session.
    pub(crate) fn endpoint(&self, frame: &FrameId) -> Option<&ProtocolSession> {
        self.hosts.get(frame)
    }

    /// Keep endpoint hints only after a complete admitted discovery, stamped at its start.
    /// An incomplete read leaves the previous routes and journal position intact. Advancing
    /// that position could hide an ancestor transition that invalidates a cached endpoint.
    pub(crate) fn remember(
        &self,
        pw: &Playwright,
        scope: &OpScope<'_>,
        page: TabId,
        root: &ProtocolSession,
        since: u64,
    ) {
        if !self.is_whole() {
            return;
        }
        let (Some(root_id), Some(main)) = (root.session_id(), self.frames.first()) else {
            return;
        };
        let routes = self
            .hosts
            .iter()
            .filter_map(|(frame, session)| {
                Some((
                    frame.clone(),
                    FrameRoute {
                        session: session.clone(),
                        ancestors: self.path_to(frame)?,
                        since,
                    },
                ))
            })
            .collect();
        scope.run_state().frames.replace(
            page,
            RootAttachment {
                session: root_id.clone(),
                frame: main.id.clone(),
                epoch: pw.browser().connection().connection_epoch(),
            },
            routes,
        );
    }

    fn add_hosted(
        &mut self,
        tree: FrameTreeNode,
        embedder: Option<&str>,
        session: &ProtocolSession,
    ) {
        let start = self.frames.len();
        self.add(tree, embedder);
        for frame in self.frames.iter().skip(start) {
            self.hosts.insert(frame.id.clone(), session.clone());
        }
    }

    fn add(&mut self, tree: FrameTreeNode, embedder: Option<&str>) {
        let root = tree.frame.id.clone();
        for frame in tree.into_frames() {
            // An OOPIF's root reports no parent in its own session; its embedder is known from
            // its target.
            let parent = if frame.id == root {
                embedder.map(str::to_owned).or(frame.parent_id)
            } else {
                frame.parent_id
            };
            let mut url = frame.url;
            url.push_str(frame.url_fragment.as_deref().unwrap_or_default());
            self.frames.push(FrameInfo {
                id: FrameId(frame.id),
                parent_id: parent.map(FrameId),
                name: frame.name.unwrap_or_default(),
                url,
                loader: frame.loader_id,
            });
        }
    }

    fn contains(&self, id: &str) -> bool {
        self.frames.iter().any(|frame| frame.id.as_str() == id)
    }

    /// The frames from the main frame down to `frame`, the main frame left out: empty for the
    /// main frame itself, `None` when `frame` is not in the tree (it was detached).
    pub(crate) fn path_to(&self, frame: &FrameId) -> Option<Vec<FrameId>> {
        let mut path = Vec::new();
        let mut at = self.frames.iter().find(|info| info.id == *frame)?;
        // Bounded by the tree's size, so a parent loop in a bad reply cannot spin.
        for _ in 0..self.frames.len() {
            let Some(parent) = &at.parent_id else {
                path.reverse();
                return Some(path);
            };
            path.push(at.id.clone());
            at = self.frames.iter().find(|info| info.id == *parent)?;
        }
        None
    }

    /// Where this read places `frame`. Only a whole read proves a frame gone: a read under
    /// [`Subtrees::Skip`] leaves out an out-of-process frame that does not answer in time (a
    /// renderer busy in a script, say), and with it every frame inside it, which are as attached
    /// as before. The frames it did read keep their paths.
    pub(crate) fn place(&self, frame: &FrameId) -> FramePlace {
        match self.path_to(frame) {
            Some(path) => FramePlace::Listed(path),
            None if self.is_whole() => FramePlace::Absent,
            None => FramePlace::Unread,
        }
    }
}

/// `page.frames()`: `page`'s frames from the target watcher's list (`TabFrames`), which the tab's
/// CDP events and seed reads keep. It sends the tab nothing: no attach, no tree read, so a tab
/// whose renderer is busy answers as fast as a healthy one, with the last list its events gave.
/// Frames a busy renderer adds appear once it runs again and its events arrive.
///
/// ```text
///  admit the tab (no attach) ─▶ synchronize on [tab] ─▶ TabFrames.get(tab) ─▶ frames, main first
///                                (its own held events,      └─ not whole: incomplete, and the
///                                 never another tab's)         watcher reads its missing seeds
/// ```
///
/// The answer fills no frame routes: a route names a session of core's and is retired by ring
/// records written only while core's session is on the tab, so routes come only from a child-frame
/// op's own whole read of a tab it admitted (`navigation::frame_host`).
pub(crate) async fn frames(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: FramesInput,
) -> Result<PageFrames, PlaywrightError> {
    let read = async {
        let target = scope.admit_target(pw.browser(), input.page.tab()).await?;
        let fence = pw.event_fence();
        // Bounded inside: a watcher that cannot catch up answers with what its list holds.
        let _caught_up = pw
            .synchronize_events(fence, &EventScope::Tabs(vec![target.clone()]))
            .await;
        let list = pw.event_hub().tab_frames(&target);
        let (answer, own_tree) = answer(scope, &target, list);
        if answer.incomplete.is_some() || answer.frames.is_none() {
            pw.reseed_frames(target, own_tree);
        }
        Ok::<_, PlaywrightError>(answer)
    };
    match scope.within(read).await {
        Ok(result) => result,
        Err(_expired) => Err(scope.timed_out(ErrorCode::WaitTimeout)),
    }
}

/// `page.frames()`'s answer from `tab`'s list, and whether the tab's own tree should be read
/// again even if the list's seeds all landed: it has no main frame, or a frame whose parent it
/// does not know. Neither a list that is not whole nor a missing one is silent: the op's record is
/// marked (`OpScope::frames_incomplete`), its note says why for the server's warn line, and the
/// prelude reads the frames again before the script's next operation resolves, by which time the
/// watcher may have read what the list lacked. The main frame is the one whose id is the tab's
/// target id: a frame whose commit carried no parent and whose attach the list never saw also
/// has none, and must not reach the prelude as the main frame.
fn answer(scope: &OpScope<'_>, tab: &TargetId, list: Option<TabFrameList>) -> (PageFrames, bool) {
    let Some(list) = list.filter(|list| list.frames.first().is_some_and(|main| main.id.0 == tab.0))
    else {
        scope.frames_incomplete();
        // Debug notes, never stored in the run's record (the mark above is); neo-server logs those
        // starting `frame tree` at warn (operation_guard).
        scope.log(
            "frame tree not known yet: no event or seed of the tab named its main frame; its \
             frames stay as they were"
                .to_owned(),
        );
        let answer = PageFrames {
            frames: None,
            incomplete: None,
        };
        return (answer, true);
    };
    let (frames, orphans) = parents_first(list.frames);
    let incomplete = !list.whole || orphans > 0;
    if incomplete {
        scope.frames_incomplete();
        scope.log(format!(
            "frame tree incomplete: the watcher has not read the tab's own tree or a frame \
             session's subtree yet ({orphans} frame(s) with no known parent left out); it reads \
             them again"
        ));
    }
    let answer = PageFrames {
        incomplete: incomplete.then_some(true),
        frames: Some(frames),
    };
    (answer, orphans > 0)
}

/// The list's frames as the prelude reads them: the main frame first, each parent before its
/// children, as the renderer read gave them. A frame other than the main one with no parent the
/// list knows is left out, as the prelude would take it for a second main frame; how many were
/// left out comes back too.
fn parents_first(frames: Vec<ListedFrame>) -> (Vec<FrameInfo>, usize) {
    let mut ordered: Vec<FrameInfo> = Vec::with_capacity(frames.len());
    let mut placed: HashSet<FrameId> = HashSet::new();
    let mut rest = frames.into_iter();
    if let Some(main) = rest.next() {
        placed.insert(main.id.clone());
        ordered.push(info(main));
    }
    let mut rest: Vec<ListedFrame> = rest.collect();
    loop {
        let before = rest.len();
        rest.retain(|frame| {
            let ready = frame
                .parent_id
                .as_ref()
                .is_some_and(|parent| placed.contains(parent));
            if ready {
                placed.insert(frame.id.clone());
                ordered.push(info(frame.clone()));
            }
            !ready
        });
        if rest.is_empty() || rest.len() == before {
            break;
        }
    }
    (ordered, rest.len())
}

fn info(frame: ListedFrame) -> FrameInfo {
    FrameInfo {
        id: frame.id,
        parent_id: frame.parent_id,
        name: frame.name,
        url: frame.url,
        loader: frame.loader_id.unwrap_or_default(),
    }
}

/// Cache metadata must not become a renderer barrier. Bound discovery even after loading has
/// finished, and discard it when a navigation starts. Only a complete refresh replaces the
/// caller's previous tree and native route hints.
pub(crate) async fn snapshot(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: TabId,
) -> Result<Option<FrameTree>, PlaywrightError> {
    let admitted = scope.admit(pw.browser(), page).await?;
    let session_id = admitted.session.session_id.clone();
    let root = pw.browser().protocol_session(session_id.clone());
    let frame = FrameId(admitted.session.target_id.as_str().to_owned());
    let mut watch = DocumentWatch::new(pw.browser(), root.clone(), session_id, frame);
    let read = async {
        let shown = scope
            .view()
            .page(admitted.core_page.clone())
            .await
            .map_err(|error| scope.core_error(&error))?;
        let tree = async {
            let since = pw.events().head();
            let tree = read(pw.browser(), &admitted.core_page, &root)
                .await
                .map_err(|error| scope.core_error(&error))?;
            tree.remember(pw, scope, page, &root, since);
            Ok(Some(tree))
        };
        if shown.is_loading {
            time::timeout(LOADING_SNAPSHOT, tree)
                .await
                .unwrap_or(Ok(None))
        } else {
            tree.await
        }
    };
    tokio::select! {
        result = time::timeout(SNAPSHOT_BUDGET, read) => result.unwrap_or(Ok(None)),
        _ = watch.navigation_started() => Ok(None),
    }
}

/// Every frame of `page`, whose main frame's session is `root`: that session's own tree, then
/// each out-of-process iframe's, as the module doc draws it. The resolver reads it too, to walk
/// down to the child frame a `frame.locator()` starts in.
pub(crate) async fn read(
    browser: &BrowserSession,
    page: &PageId,
    root: &ProtocolSession,
) -> Result<FrameTree, CoreError> {
    let mut tree = FrameTree::default();
    tree.add_hosted(frame_tree(root).await?.frame_tree, None, root);
    let iframes = list_iframes(browser).await?;
    read_oopifs(browser, page, &mut tree, &iframes, Subtrees::Skip).await?;
    Ok(tree)
}

/// How long a read that may leave frames out waits for one out-of-process frame's own tree: the
/// browser's deadline for a subframe's content in an APC capture (contract P6).
const OOPIF_TREE_BUDGET: Duration = Duration::from_secs(2);

/// What a read does with a cross-site frame whose own tree cannot be read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Subtrees {
    /// Leave it out, and mark the tree not whole (`FrameTree::unread`): the frame may have gone
    /// away between the two reads, or its session may still be attaching.
    Skip,
    /// Fail the read.
    Require,
}

/// The private look's complete frame read (ruling R63), with one list of the
/// browser's out-of-process frames taken right before the root's tree and compared with the list
/// the read takes right after it. `None` when the two differ: a frame out of process at the root's
/// read and in its parent's process at the list is in neither, and the read would come back whole
/// without it. A frame that moves after the list needs no other check: a listed frame whose own
/// tree cannot be read fails the read ([`Subtrees::Require`], after its tries). So the window is
/// two adjacent calls. Only target ids are compared, as a target's address and title change with
/// every navigation; the lists are the browser's, so frames moving in another tab answer `None`
/// too, the safe side.
pub(crate) async fn read_steady(
    browser: &BrowserSession,
    page: &PageId,
    root: &ProtocolSession,
) -> Result<Option<FrameTree>, CoreError> {
    let before = list_iframes(browser).await?;
    let mut tree = FrameTree::default();
    tree.add_hosted(frame_tree(root).await?.frame_tree, None, root);
    let iframes = list_iframes(browser).await?;
    if target_ids(&iframes) != target_ids(&before) {
        return Ok(None);
    }
    read_oopifs(browser, page, &mut tree, &iframes, Subtrees::Require).await?;
    Ok(Some(tree))
}

/// The browser's out-of-process frames, of every tab (an OOPIF's target id is its frame id).
async fn list_iframes(browser: &BrowserSession) -> Result<Targets, CoreError> {
    browser
        .cdp(
            "Target.getTargets",
            json!({ "filter": [{ "type": "iframe" }] }),
            None,
        )
        .await
        .and_then(decode)
}

/// The target ids of the browser's out-of-process frames, of every tab: the browser answers it
/// from its own frame tree, so no renderer, busy or not, holds it.
pub(crate) async fn iframe_targets(browser: &BrowserSession) -> Result<HashSet<String>, CoreError> {
    Ok(list_iframes(browser)
        .await?
        .target_infos
        .into_iter()
        .map(|target| target.target_id)
        .collect())
}

fn target_ids(targets: &Targets) -> BTreeSet<&str> {
    targets
        .target_infos
        .iter()
        .map(|target| target.target_id.as_str())
        .collect()
}

/// Adds to `tree` the subtree of every out-of-process frame in `iframes` whose parent it holds,
/// level by level. One that cannot be read fails the read under [`Subtrees::Require`], and is
/// left out and noted (`FrameTree::unread`) under [`Subtrees::Skip`].
async fn read_oopifs(
    browser: &BrowserSession,
    page: &PageId,
    tree: &mut FrameTree,
    iframes: &Targets,
    subtrees: Subtrees,
) -> Result<(), CoreError> {
    let mut read = HashSet::new();
    loop {
        let next: Vec<&IframeTarget> = iframes
            .target_infos
            .iter()
            .filter(|oopif| !read.contains(oopif.target_id.as_str()))
            .filter(|oopif| {
                oopif
                    .parent_frame_id
                    .as_deref()
                    .is_some_and(|parent| tree.contains(parent))
            })
            .collect();
        if next.is_empty() {
            break;
        }
        for oopif in next {
            read.insert(oopif.target_id.as_str());
            // A frame that went away between the two reads is simply not listed.
            let Some(parent) = oopif
                .parent_frame_id
                .as_ref()
                .and_then(|id| tree.hosts.get(&FrameId(id.clone())))
            else {
                continue;
            };
            let read_one = oopif_tree_retried(browser, page, &oopif.target_id, parent);
            let subtree = match subtrees {
                Subtrees::Require => read_one.await,
                // A frame whose renderer hangs never answers its tree: a read that may leave a
                // frame out leaves that one out once its budget passes, so the frames beside it
                // are still found (the APC capture gives such a frame the same budget).
                Subtrees::Skip => time::timeout(OOPIF_TREE_BUDGET, read_one)
                    .await
                    .unwrap_or_else(|_elapsed| {
                        Err(CoreError::Message(format!(
                            "its tree did not answer within {} ms",
                            OOPIF_TREE_BUDGET.as_millis()
                        )))
                    }),
            };
            match subtree {
                Ok(subtree) => tree.add_hosted(
                    subtree.tree.frame_tree,
                    oopif.parent_frame_id.as_deref(),
                    &subtree.session,
                ),
                Err(error) if subtrees == Subtrees::Require => return Err(error),
                Err(error) => tree
                    .unread
                    .push((oopif.target_id.clone(), error.to_string())),
            }
        }
    }
    Ok(())
}

/// `session`'s own frames: its root and the same-process frames under it.
pub(crate) async fn frame_tree(session: &ProtocolSession) -> Result<FrameTreeReply, CoreError> {
    decode(session.send_value("Page.getFrameTree", json!({})).await?)
}

/// Pauses before each try of an OOPIF's tree. Its session may still be attaching (core attaches
/// an OOPIF it has not seen on demand, and the browser's attach event can land between two
/// tries), so a failed read is tried again before the tree is answered as incomplete. The whole
/// read stays inside [`SNAPSHOT_BUDGET`].
const OOPIF_TRIES: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_millis(50),
    Duration::from_millis(150),
];

/// [`oopif_tree`], tried [`OOPIF_TRIES`] times; the last failure when none answered.
async fn oopif_tree_retried(
    browser: &BrowserSession,
    page: &PageId,
    frame: &str,
    parent: &ProtocolSession,
) -> Result<HostedTree, CoreError> {
    let mut failed = None;
    for pause in OOPIF_TRIES {
        // The first try waits for nothing. A zero sleep still yields to the timer (about a
        // millisecond), and a page's cross-site frames are read one after another, so a page
        // with a hundred of them would pay it on every read, and so after every operation. A
        // first try that comes before the frame's session attached is caught by `oopif_tree`'s
        // root check, never taken as the frame's tree.
        if !pause.is_zero() {
            time::sleep(pause).await;
        }
        match oopif_tree(browser, page, frame, parent).await {
            Ok(tree) => return Ok(tree),
            Err(error) => failed = Some(error),
        }
    }
    Err(failed.unwrap_or_else(|| CoreError::Message("no read was tried".to_owned())))
}

/// One OOPIF's own frame tree, on the session core's frame registry holds for it.
async fn oopif_tree(
    browser: &BrowserSession,
    page: &PageId,
    frame: &str,
    parent: &ProtocolSession,
) -> Result<HostedTree, CoreError> {
    let target = browser
        .frames()
        .resolve_frame_target(
            page.clone(),
            Some(FrameId(frame.to_owned())),
            Some(parent),
            true,
        )
        .await?;
    // Publish a child Frame only after its session reports navigation. On adopted tabs,
    // core has attached the OOPIF without enabling Page; enabling it when a waiter starts
    // is too late for a simultaneous same-renderer navigation (which has no swap to replay).
    target.session.send_value("Page.enable", json!({})).await?;
    let tree = frame_tree(&target.session).await?;
    // While the frame's own session is still attaching (its attach event has not landed, so
    // core's on-demand attach finds it in flight), core answers the parent's session instead,
    // whose tree is the parent's. Taken as the frame's, it would list the parent's frames again
    // and leave this frame and its children out, with no error: a short tree read as whole.
    // It is a failed try, read again after a pause (`OOPIF_TRIES`). An OOPIF's target id is its
    // frame id.
    if tree.frame_tree.frame.id != frame {
        return Err(CoreError::Message(format!(
            "frame {frame}'s own session is not attached yet"
        )));
    }
    Ok(HostedTree {
        tree,
        session: target.session,
    })
}

/// Two frames, the main one and a named child, for the answers' byte tests.
#[cfg(test)]
pub(crate) fn sample_tree() -> Result<FrameTree, serde_json::Error> {
    let page: FrameTreeReply = serde_json::from_value(json!({ "frameTree": {
        "frame": { "id": "main", "loaderId": "L", "url": "https://main.test/" },
        "childFrames": [{ "frame": {
            "id": "child", "parentId": "main", "name": "ad", "loaderId": "L",
            "url": "https://child.test/",
        } }],
    } }))?;
    let mut tree = FrameTree::default();
    tree.add(page.frame_tree, None);
    Ok(tree)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_core::{FrameId, SessionId, TabId, TargetId};
    use serde::Serialize;
    use serde_json::json;

    use super::{FramePlace, FrameTree, PageFrames, answer, sample_tree};
    use crate::events::EventHub;
    use crate::events::tab_frames::{ListedFrame, TabFrameList};
    use crate::op_scope::OpScope;
    use crate::page::document_watch::FrameTreeReply;
    use crate::target::CallHeader;
    use crate::test_env::{self, EnvCall, TestEnv};
    use crate::{Op, Playwright};

    /// Ordinary frame dispatch keeps a previously discovered endpoint while a refresh lacks
    /// an OOPIF. Replacing the routes with that short tree would lose a usable child session.
    #[tokio::test]
    async fn an_incomplete_tree_preserves_previously_discovered_endpoints()
    -> Result<(), Box<dyn Error>> {
        let pw = Playwright::new(test_env::offline_session());
        let env = TestEnv::new();
        let op = Op::from_name("page.frames").ok_or("page.frames is an op")?;
        let scope = OpScope::new(&env, op, CallHeader::default());
        let session = SessionId::from("root");
        let root = pw.browser().protocol_session(session.clone());
        let main = FrameId("main".to_owned());
        let attachment = super::RootAttachment {
            session: session.clone(),
            frame: main.clone(),
            epoch: pw.browser().connection().connection_epoch(),
        };
        let mut short = sample_tree()?;
        short.frames.retain(|frame| frame.id == main);
        short.hosts.insert(main.clone(), root.clone());
        short
            .unread
            .push(("unread-child".to_owned(), "session attaching".to_owned()));
        let (events, _writer) = EventHub::new();
        let mut previous = sample_tree()?;
        for frame in &previous.frames {
            previous.hosts.insert(frame.id.clone(), root.clone());
        }
        previous.remember(&pw, &scope, TabId(7), &root, events.head());
        short.remember(&pw, &scope, TabId(7), &root, events.head());
        let child = FrameId("child".to_owned());
        let cached = scope
            .run_state()
            .frames
            .cached(TabId(7), &attachment, &child, &events);
        assert_eq!(
            cached.and_then(|route| route.session_id().cloned()),
            Some(session),
            "a valid endpoint from the previous whole tree survives the incomplete refresh"
        );
        Ok(())
    }

    /// A whole list leaves the op's record unmarked. One that is not whole, one with a frame
    /// whose parent it does not know, or no list at all marks it (`OpEnv::frames_incomplete`), so
    /// a host finds the run whose script was handed a short tree; the answer says `incomplete`,
    /// lists the main frame first and each parent before its children, and leaves the orphan out.
    #[tokio::test]
    async fn tests_that_only_a_short_or_missing_list_marks_the_operation()
    -> Result<(), Box<dyn Error>> {
        // Whether the op was marked, its answer, and whether it asks for the tab's own tree again.
        let asked =
            |list: Option<TabFrameList>| -> Result<(bool, PageFrames, bool), Box<dyn Error>> {
                let env = TestEnv::new();
                let op = Op::from_name("page.frames").ok_or("page.frames is an op")?;
                let scope = OpScope::new(&env, op, CallHeader::default());
                let (answer, own_tree) = answer(&scope, &TargetId("main".to_owned()), list);
                Ok((
                    env.calls().contains(&EnvCall::FramesIncomplete),
                    answer,
                    own_tree,
                ))
            };
        let marked = |list: Option<TabFrameList>| -> Result<(bool, PageFrames), Box<dyn Error>> {
            let (flag, answer, _own_tree) = asked(list)?;
            Ok((flag, answer))
        };
        let frame = |id: &str, parent: Option<&str>| ListedFrame {
            id: FrameId(id.to_owned()),
            parent_id: parent.map(|parent| FrameId(parent.to_owned())),
            name: String::new(),
            url: format!("https://{id}.test/"),
            loader_id: Some(format!("{id}-doc")),
        };
        let list = |whole: bool, frames: Vec<ListedFrame>| TabFrameList {
            frames,
            version: 3,
            whole,
            events: 0,
        };
        let ids = |answer: &PageFrames| -> Vec<String> {
            answer
                .frames
                .iter()
                .flatten()
                .map(|frame| frame.id.0.clone())
                .collect()
        };
        let nested = vec![
            frame("main", None),
            frame("grandchild", Some("child")),
            frame("child", Some("main")),
        ];
        let (flag, whole) = marked(Some(list(true, nested.clone())))?;
        assert!(!flag, "a whole list");
        assert_eq!(whole.incomplete, None);
        assert_eq!(
            ids(&whole),
            ["main", "child", "grandchild"],
            "parents first"
        );
        let (flag, short) = marked(Some(list(false, nested.clone())))?;
        assert!(flag, "a list that is not whole");
        assert_eq!(short.incomplete, Some(true));
        assert_eq!(ids(&short).len(), 3, "its frames are real and go with it");
        let (_, _, own_tree) = asked(Some(list(false, nested.clone())))?;
        assert!(!own_tree, "a list short of a seed asks for that seed alone");
        let mut orphaned = nested;
        orphaned.push(frame("orphan", None));
        let (flag, orphan, own_tree) = asked(Some(list(true, orphaned)))?;
        assert!(flag, "a frame whose parent the list does not know");
        assert!(!ids(&orphan).contains(&"orphan".to_owned()));
        assert!(own_tree, "an orphan has the tab's own tree read again");
        let (flag, none) = marked(None)?;
        assert!(flag, "no list");
        assert!(none.frames.is_none());
        let (flag, mainless) = marked(Some(list(true, vec![frame("child", Some("main"))])))?;
        assert!(flag, "a list with no main frame");
        assert!(mainless.frames.is_none());
        // A frame whose commit carried no parent and whose attach the list never saw has none
        // either: it is not the main frame, which is the one with the tab's own id.
        let (flag, stray) = marked(Some(list(true, vec![frame("stray", None)])))?;
        assert!(flag, "a list whose only parentless frame is not the tab's");
        assert!(
            stray.frames.is_none(),
            "no stray frame goes as the main frame"
        );
        Ok(())
    }

    /// Only a whole read proves a frame gone. A frame a whole read does not list is absent; one
    /// missing from a read that left an out-of-process frame out is unread, since it may be inside
    /// that frame, and the frames the partial read did list keep their paths. Fails with a partial
    /// read's missing frame taken as absent, which retires a busy frame's refs for good.
    #[test]
    fn tests_that_only_a_whole_read_places_a_frame_absent() -> Result<(), serde_json::Error> {
        let mut tree = sample_tree()?;
        let child = FrameId("child".to_owned());
        let busy = FrameId("busy".to_owned());
        assert_eq!(tree.place(&child), FramePlace::Listed(vec![child.clone()]));
        assert_eq!(tree.place(&busy), FramePlace::Absent, "a whole read");
        tree.unread.push((
            "busy".to_owned(),
            "its tree did not answer within 2000 ms".to_owned(),
        ));
        assert_eq!(
            tree.place(&child),
            FramePlace::Listed(vec![child.clone()]),
            "a partial read still leads to the frames it read"
        );
        assert_eq!(tree.place(&busy), FramePlace::Unread, "a partial read");
        Ok(())
    }

    /// `page.frames()` answers the bytes the flattened `Option<FrameTree>` it replaced wrote,
    /// key order included, with frames and without: a later edit cannot drift unseen.
    #[test]
    fn tests_that_the_frame_answer_keeps_its_bytes() -> Result<(), serde_json::Error> {
        #[derive(Serialize)]
        struct Before {
            #[serde(flatten)]
            frames: Option<FrameTree>,
        }
        let with = serde_json::to_string(&PageFrames {
            frames: Some(sample_tree()?.into_frames()),
            incomplete: None,
        })?;
        assert_eq!(
            with,
            serde_json::to_string(&Before {
                frames: Some(sample_tree()?)
            })?
        );
        assert_eq!(
            with,
            r#"{"frames":[{"id":"main","parentId":null,"name":"","url":"https://main.test/"},{"id":"child","parentId":"main","name":"ad","url":"https://child.test/"}]}"#
        );
        let without = serde_json::to_string(&PageFrames {
            frames: None,
            incomplete: None,
        })?;
        assert_eq!(without, serde_json::to_string(&Before { frames: None })?);
        assert_eq!(without, "{}", "no frames read is no key at all");
        Ok(())
    }

    #[test]
    fn tests_that_a_partial_tree_never_supplements_a_reply() -> Result<(), serde_json::Error> {
        let whole = sample_tree()?;
        assert!(whole.is_whole());
        assert_eq!(
            whole.into_whole_frames().map(|frames| frames.len()),
            Some(2)
        );
        let mut partial = sample_tree()?;
        partial
            .unread
            .push(("oopif-1".to_owned(), "session detached".to_owned()));
        assert!(!partial.is_whole());
        assert!(
            partial.into_whole_frames().is_none(),
            "an evaluate, a tag or a navigation answers no tree rather than a short one"
        );
        Ok(())
    }

    #[test]
    fn tests_that_an_oopif_hangs_under_the_frame_that_embeds_it() -> Result<(), serde_json::Error> {
        let frame = |id: &str, parent: Option<&str>, name: Option<&str>| {
            json!({ "id": id, "parentId": parent, "name": name, "loaderId": "L",
                    "url": format!("https://{id}.test/") })
        };
        let page: FrameTreeReply = serde_json::from_value(json!({ "frameTree": {
            "frame": frame("main", None, None),
            "childFrames": [{ "frame": frame("same", Some("main"), Some("ad")) }],
        } }))?;
        let oopif: FrameTreeReply = serde_json::from_value(json!({ "frameTree": {
            "frame": frame("oopif", None, Some("pay")),
            "childFrames": [{ "frame": frame("inner", Some("oopif"), None) }],
        } }))?;
        let mut tree = FrameTree::default();
        tree.add(page.frame_tree, None);
        tree.add(oopif.frame_tree, Some("same"));
        let listed = serde_json::to_value(&tree)?;
        assert_eq!(
            listed,
            json!({ "frames": [
                { "id": "main", "parentId": null, "name": "", "url": "https://main.test/" },
                { "id": "same", "parentId": "main", "name": "ad", "url": "https://same.test/" },
                { "id": "oopif", "parentId": "same", "name": "pay", "url": "https://oopif.test/" },
                { "id": "inner", "parentId": "oopif", "name": "", "url": "https://inner.test/" },
            ] })
        );
        let ids = |ids: &[&str]| ids.iter().map(|id| FrameId((*id).to_owned())).collect();
        assert_eq!(
            tree.path_to(&FrameId("inner".to_owned())),
            Some(ids(&["same", "oopif", "inner"])),
            "an OOPIF's frame sits under the frame that embeds it"
        );
        assert_eq!(tree.path_to(&FrameId("main".to_owned())), Some(Vec::new()));
        assert_eq!(
            tree.path_to(&FrameId("gone".to_owned())),
            None,
            "a frame not in the tree was detached"
        );
        Ok(())
    }
}
