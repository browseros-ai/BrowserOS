//! `page.frames()`: every frame of a tab, out-of-process ones included, for the prelude's
//! synchronous `frames()`, `mainFrame()`, `parentFrame()` and `childFrames()`.
//!
//! A session's `Page.getFrameTree` lists only the frames in its own renderer. Each
//! out-of-process iframe is a target of its own whose `parentFrameId` names the frame that
//! embeds it, so the tree is read session by session:
//!
//! ```text
//!  page session ── getFrameTree ──▶ main frame + same-process children
//!  Target.getTargets (iframes) ── parentFrameId already in the tree? ──▶ that OOPIF's session
//!     (core's FrameRegistry) ── getFrameTree ──▶ its frames; repeat for OOPIFs inside OOPIFs
//! ```

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use browseros_core::{BrowserSession, CoreError, FrameId, PageId, ProtocolSession, TabId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time;
use ts_rs::TS;

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

    /// Keep endpoint hints only after a complete admitted discovery. Stamp the start, so a
    /// renderer swap during discovery invalidates the hint rather than being hidden by it.
    pub(crate) fn remember(
        &self,
        pw: &Playwright,
        scope: &OpScope<'_>,
        page: TabId,
        root: &ProtocolSession,
        since: u64,
    ) {
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
            since,
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
}

/// `page.frames()`: reads `page`'s frame tree.
pub(crate) async fn frames(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: FramesInput,
) -> Result<PageFrames, PlaywrightError> {
    let frames = match scope.within(snapshot(pw, scope, input.page.tab())).await {
        Ok(result) => result?,
        Err(_expired) => return Err(scope.timed_out(ErrorCode::WaitTimeout)),
    };
    Ok(answer(scope, frames))
}

/// `page.frames()`'s answer from what its read got. Neither a short tree nor a missing one is
/// silent: the op's record is marked (`OpScope::frames_incomplete`), its notes name each frame
/// and error for the server's warn line, and the prelude reads the tree again before the
/// script's next operation resolves.
fn answer(scope: &OpScope<'_>, frames: Option<FrameTree>) -> PageFrames {
    let incomplete = frames.as_ref().is_some_and(|tree| !tree.unread.is_empty());
    if incomplete || frames.is_none() {
        scope.frames_incomplete();
    }
    match &frames {
        // Debug notes, never stored in the run's record (the mark above is); neo-server logs those
        // starting `frame tree` at warn (operation_guard), naming the frame and the error.
        None => scope.log(
            "frame tree not read: the tab was loading or navigating past the read's time; its \
             frames stay as they were"
                .to_owned(),
        ),
        Some(tree) => {
            for (frame, error) in &tree.unread {
                scope.log(format!(
                    "frame tree incomplete: the out-of-process frame {frame} could not be read, \
                     so the tree lacks it: {error}"
                ));
            }
        }
    }
    PageFrames {
        incomplete: incomplete.then_some(true),
        frames: frames.map(FrameTree::into_frames),
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
    read_frames(browser, page, root, Subtrees::Skip).await
}

/// [`read`], failing where it skips: a cross-site frame that exists and whose own tree cannot be
/// read fails the whole read. Neo's camera reads every frame before it may store a picture
/// (protected-fill ruling R45), and a frame left out there could be the one showing a card
/// number.
pub(crate) async fn read_every(
    browser: &BrowserSession,
    page: &PageId,
    root: &ProtocolSession,
) -> Result<FrameTree, CoreError> {
    read_frames(browser, page, root, Subtrees::Require).await
}

/// What a read does with a cross-site frame whose own tree cannot be read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Subtrees {
    /// Leave it out, and mark the tree not whole (`FrameTree::unread`): the frame may have gone
    /// away between the two reads, or its session may still be attaching.
    Skip,
    /// Fail the read.
    Require,
}

async fn read_frames(
    browser: &BrowserSession,
    page: &PageId,
    root: &ProtocolSession,
    subtrees: Subtrees,
) -> Result<FrameTree, CoreError> {
    let mut tree = FrameTree::default();
    tree.add_hosted(frame_tree(root).await?.frame_tree, None, root);
    let iframes: Targets = browser
        .cdp(
            "Target.getTargets",
            json!({ "filter": [{ "type": "iframe" }] }),
            None,
        )
        .await
        .and_then(decode)?;
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
            match oopif_tree_retried(browser, page, &oopif.target_id, parent).await {
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
    Ok(tree)
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
    // and leave this frame and its children out, with no error: a short tree, read as whole,
    // that the camera too would take for every frame. It is a failed try, read again after a
    // pause (`OOPIF_TRIES`). An OOPIF's target id is its frame id.
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
    use browseros_core::FrameId;
    use serde::Serialize;
    use serde_json::json;

    use super::{FrameTree, PageFrames, answer, sample_tree};
    use crate::Op;
    use crate::op_scope::OpScope;
    use crate::page::document_watch::FrameTreeReply;
    use crate::target::CallHeader;
    use crate::test_env::{EnvCall, TestEnv};

    /// A read that came back whole leaves the op's record unmarked. One that lacks an
    /// out-of-process frame, or got no tree at all, marks it (`OpEnv::frames_incomplete`), so a
    /// host finds the run whose script was handed a short tree.
    #[tokio::test]
    async fn tests_that_only_a_short_or_missing_tree_marks_the_operation()
    -> Result<(), Box<dyn std::error::Error>> {
        let marked = |frames: Option<FrameTree>| -> Result<bool, Box<dyn std::error::Error>> {
            let env = TestEnv::new();
            let op = Op::from_name("page.frames").ok_or("page.frames is an op")?;
            let scope = OpScope::new(&env, op, CallHeader::default());
            answer(&scope, frames);
            Ok(env.calls().contains(&EnvCall::FramesIncomplete))
        };
        let mut short = sample_tree()?;
        short.unread.push((
            "cross".to_owned(),
            "frame cross's own session is not attached yet".to_owned(),
        ));
        assert!(!marked(Some(sample_tree()?))?, "a whole tree");
        assert!(marked(Some(short))?, "a tree that lacks a frame");
        assert!(marked(None)?, "no tree");
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
