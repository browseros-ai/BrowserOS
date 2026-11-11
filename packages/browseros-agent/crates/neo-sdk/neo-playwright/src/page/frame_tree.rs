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

use browseros_core::{BrowserSession, CoreError, FrameId, PageId, ProtocolSession, TabId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::op_output::to_output;
use crate::op_scope::OpScope;
use crate::page::document_watch::{DocumentWatch, FrameTreeNode, FrameTreeReply, decode};
use crate::page::frame_routes::{FrameRoute, RootAttachment};
use crate::target::PageRef;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// `page.frames()`: `{ page }`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FramesInput {
    page: PageRef,
}

/// A page's frames, each parent before its children; the first is the main frame.
#[derive(Debug, Default, Serialize)]
pub(crate) struct FrameTree {
    frames: Vec<FrameInfo>,
    #[serde(skip)]
    hosts: HashMap<FrameId, ProtocolSession>,
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
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FrameInfo {
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
) -> Result<Value, PlaywrightError> {
    #[derive(Serialize)]
    struct Snapshot {
        #[serde(flatten)]
        frames: Option<FrameTree>,
    }
    let frames = match scope.within(snapshot(pw, scope, input.page.tab())).await {
        Ok(result) => result?,
        Err(_expired) => return Err(scope.timed_out(ErrorCode::WaitTimeout)),
    };
    to_output(scope, &Snapshot { frames })
}

/// Cache metadata must not become a navigation barrier. Browser-owned loading state catches
/// an existing pending navigation; a watcher subscribed before that read catches one that
/// starts in between. The caller retains its previous tree when the renderer is busy.
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
        if shown.is_loading {
            return Ok(None);
        }
        let since = pw.events().head();
        let tree = read(pw.browser(), &admitted.core_page, &root)
            .await
            .map_err(|error| scope.core_error(&error))?;
        tree.remember(pw, scope, page, &root, since);
        Ok(Some(tree))
    };
    tokio::select! {
        result = read => result,
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
            if let Ok(subtree) = oopif_tree(browser, page, &oopif.target_id, parent).await {
                tree.add_hosted(
                    subtree.tree.frame_tree,
                    oopif.parent_frame_id.as_deref(),
                    &subtree.session,
                );
            }
        }
    }
    Ok(tree)
}

/// `session`'s own frames: its root and the same-process frames under it.
pub(crate) async fn frame_tree(session: &ProtocolSession) -> Result<FrameTreeReply, CoreError> {
    decode(session.send_value("Page.getFrameTree", json!({})).await?)
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
    Ok(HostedTree {
        tree,
        session: target.session,
    })
}

#[cfg(test)]
mod tests {
    use browseros_core::FrameId;
    use serde_json::json;

    use super::FrameTree;
    use crate::page::document_watch::FrameTreeReply;

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
