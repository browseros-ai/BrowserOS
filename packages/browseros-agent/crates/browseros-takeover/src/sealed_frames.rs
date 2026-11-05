//! Neo's sealed world in every frame of one tab: the per-frame plumbing the sweep and the
//! control journal share (design D13, synthesis item 6). sdk-privacy owns the seal itself
//! (`seal.js`) and its classifier; this module only puts it in place.
//!
//! A tab's frames live in one or more targets: the page's own, and one per out-of-process frame
//! (OOPIF). A frame can be reached only on the session of the target that hosts it, so the walk
//! goes target by target, starting from the page:
//!
//! ```text
//!  page session (PageManager::get_session)
//!    ├─ Page.getFrameTree ────────────────────▶ the frames this target hosts
//!    └─ DOM.getDocument {depth: -1, pierce} ──▶ the closed shadow roots, each with its frame
//!                                            └─▶ frame owners with no contentDocument: OOPIFs
//!                                                  └─ BrowserView::frame_session ─▶ the same walk
//! ```
//!
//! [`TargetFrames::seal`] then works on one target's session:
//! 1. `Page.createIsolatedWorld {frameId, worldName: SEALED_WORLD}` for each frame;
//! 2. `Runtime.addBinding {name: JOURNAL_BINDING, executionContextName: SEALED_WORLD}`;
//! 3. `Runtime.evaluate(seal_source())` in each world. It seals a document once, and a second
//!    run changes nothing;
//! 4. `__clawSeal.watch(root)` on each closed shadow root, resolved into its own frame's world,
//!    then `Runtime.releaseObject` on the root's handle.
//!
//! **The `browseros-sdk` world, not core's `browseros`.** Core's page scripts run in `browseros`
//! and read raw values to place the caret and check fills, so sealing that world would break
//! them. `SEALED_WORLD` is claw-sdk's own world: what Neo's code reads there is masked, and the
//! page's own world is untouched.
//!
//! **The binding goes in after the worlds and before the seal.** `seal.js` takes the binding into
//! its closure when it installs, and deletes the global. A seal that ran before the binding
//! existed never journals, for the life of that document (measured on Chromium 155).
//! `Runtime.addBinding` reaches every world of that name that already exists, but a world made
//! later only on a session with Runtime enabled, and core does not enable it on OOPIF sessions.
//! So the sweep adds the binding too: a frame it seals can still journal once the journal arms.
//!
//! **A frame that fails is skipped.** Every step gets [`FRAME_BUDGET`]. A frame or a frame target
//! that errs or stalls is logged and left out, and the rest go on; only the page itself failing
//! fails the tab. Nothing here reads a field's value, so nothing here can log one.

use crate::limits::FRAME_BUDGET;
use crate::{BrowserTabs, PartError};
use browseros_cdp::CdpError;
use browseros_core::sensitive_fields::{JOURNAL_BINDING, SEALED_WORLD, seal_source};
use browseros_core::{CoreError, FrameId, PageId, ProtocolSession, TabId};
use serde_json::{Value, json};
use std::future::Future;

/// Watches a closed shadow root in the sealed world: a document-level listener cannot see input
/// that starts inside one (sdk-privacy D4).
const WATCH: &str = "function () { __clawSeal.watch(this); }";

/// One frame with the sealed world installed: where Neo evaluates in that frame.
#[derive(Clone, Debug)]
pub(crate) struct SealedFrame {
    /// The session of the target that hosts the frame.
    pub(crate) session: ProtocolSession,
    /// The frame.
    pub(crate) frame: FrameId,
    /// The sealed world's execution context in the frame's current document.
    pub(crate) context: i64,
}

/// Seals every frame of the tab, and gives back the frames that took the seal.
///
/// # Errors
/// `TabNotFound` when no open tab has this id; `Browser` when the page itself cannot be reached.
pub(crate) async fn seal_tab(
    tabs: &BrowserTabs,
    tab: TabId,
) -> Result<Vec<SealedFrame>, PartError> {
    let (page, session) = tabs.page_session(tab).await?;
    let mut sealed = Vec::new();
    for target in TabFrames::walk(tabs, &page, session).await?.targets {
        sealed.extend(target.seal().await);
    }
    Ok(sealed)
}

/// The targets one tab's frames live in, the page's own first.
pub(crate) struct TabFrames {
    pub(crate) targets: Vec<TargetFrames>,
}

impl TabFrames {
    /// Walks from the page's session down to every OOPIF under it.
    ///
    /// # Errors
    /// `Browser` when the page's own frame tree cannot be read. An OOPIF that cannot be reached
    /// or read is logged and left out.
    pub(crate) async fn walk(
        tabs: &BrowserTabs,
        page: &PageId,
        session: ProtocolSession,
    ) -> Result<Self, PartError> {
        let (first, mut pending) = TargetFrames::read(tabs, page, session)
            .await
            .map_err(PartError::Browser)?;
        let mut targets = vec![first];
        while let Some(session) = pending.pop() {
            if targets
                .iter()
                .any(|target| target.session.same_session(&session))
            {
                continue;
            }
            match TargetFrames::read(tabs, page, session).await {
                Ok((target, below)) => {
                    pending.extend(below);
                    targets.push(target);
                }
                Err(error) => {
                    tracing::warn!(%error, "the sealed world skipped a frame target it could not read");
                }
            }
        }
        Ok(Self { targets })
    }
}

/// One target: the session it is reached on, the frames it hosts, and the closed shadow roots
/// in their documents.
pub(crate) struct TargetFrames {
    pub(crate) session: ProtocolSession,
    frames: Vec<FrameId>,
    closed_roots: Vec<ClosedRoot>,
}

/// A closed shadow root, and the frame whose document holds it.
struct ClosedRoot {
    frame: FrameId,
    backend_node: i64,
}

impl TargetFrames {
    /// Reads the frames `session`'s target hosts, and the sessions of the OOPIFs its documents
    /// embed. A DOM that cannot be read loses only the closed roots and the OOPIFs below it.
    async fn read(
        tabs: &BrowserTabs,
        page: &PageId,
        session: ProtocolSession,
    ) -> Result<(Self, Vec<ProtocolSession>), CoreError> {
        let tree = step(&session, "Page.getFrameTree", json!({})).await?;
        let mut frames = Vec::new();
        list_frames(&tree["frameTree"], &mut frames);
        let root = frames
            .first()
            .cloned()
            .ok_or_else(|| CoreError::Message("the frame tree has no frame".into()))?;
        let mut scan = DocumentScan::default();
        match step(
            &session,
            "DOM.getDocument",
            json!({ "depth": -1, "pierce": true }),
        )
        .await
        {
            Ok(document) => scan.node(&document["root"], &root),
            Err(error) => tracing::warn!(
                %error,
                "the sealed world could not read a frame target's DOM; its closed shadow roots and \
                 out-of-process frames are left out"
            ),
        }
        let mut below = Vec::new();
        for frame in scan.remote_frames {
            let found = within_budget(
                "frame_session",
                tabs.frame_session(page.clone(), frame.clone()),
            )
            .await;
            match found {
                Ok(child) if !child.same_session(&session) => below.push(child),
                Ok(_) => {}
                Err(error) => tracing::warn!(
                    frame = frame.as_str(),
                    %error,
                    "the sealed world skipped an out-of-process frame it could not reach"
                ),
            }
        }
        Ok((
            Self {
                session,
                frames,
                closed_roots: scan.closed_roots,
            },
            below,
        ))
    }

    /// Seals every frame this target hosts, and gives back the frames that took the seal.
    pub(crate) async fn seal(&self) -> Vec<SealedFrame> {
        let mut worlds = Vec::new();
        for frame in &self.frames {
            match create_world(&self.session, frame).await {
                Ok(context) => worlds.push((frame, context)),
                Err(error) => skipped(frame, &error),
            }
        }
        // After the worlds exist and before the seal runs in them; the module doc says why.
        if let Err(error) = step(
            &self.session,
            "Runtime.addBinding",
            json!({ "name": JOURNAL_BINDING, "executionContextName": SEALED_WORLD }),
        )
        .await
        {
            tracing::warn!(%error, "the journal's binding was not added; frames sealed now cannot journal");
        }
        let mut sealed = Vec::new();
        for (frame, context) in worlds {
            match install_seal(&self.session, context).await {
                Ok(()) => sealed.push(SealedFrame {
                    session: self.session.clone(),
                    frame: frame.clone(),
                    context,
                }),
                Err(error) => skipped(frame, &error),
            }
        }
        for root in &self.closed_roots {
            let Some(world) = sealed.iter().find(|sealed| sealed.frame == root.frame) else {
                continue;
            };
            if let Err(error) = watch(&self.session, root.backend_node, world.context).await {
                tracing::warn!(
                    frame = root.frame.as_str(),
                    %error,
                    "the sealed world could not watch a closed shadow root"
                );
            }
        }
        sealed
    }
}

/// What a pierced `DOM.getDocument` shows about one target's frames.
#[derive(Default)]
struct DocumentScan {
    closed_roots: Vec<ClosedRoot>,
    /// Frames this target's documents embed but another process hosts.
    remote_frames: Vec<FrameId>,
}

impl DocumentScan {
    /// Scans `node`, which belongs to `frame`'s document.
    fn node(&mut self, node: &Value, frame: &FrameId) {
        for root in node["shadowRoots"].as_array().into_iter().flatten() {
            if root["shadowRootType"] == "closed"
                && let Some(backend_node) = root["backendNodeId"].as_i64()
            {
                self.closed_roots.push(ClosedRoot {
                    frame: frame.clone(),
                    backend_node,
                });
            }
            self.node(root, frame);
        }
        for child in node["children"].as_array().into_iter().flatten() {
            self.node(child, frame);
        }
        if let Some(owned) = owned_frame(node) {
            match node.get("contentDocument") {
                // In this process: the child's document, and its closed roots, are in this one.
                Some(document) => self.node(document, &owned),
                None => self.remote_frames.push(owned),
            }
        }
    }
}

/// The frame a frame owner element holds, as core's own frame walk recognizes owners.
fn owned_frame(node: &Value) -> Option<FrameId> {
    let owner = matches!(
        node["nodeName"].as_str(),
        Some("IFRAME" | "FRAME" | "OBJECT" | "EMBED" | "FENCEDFRAME")
    );
    owner
        .then(|| node["frameId"].as_str())
        .flatten()
        .map(|id| FrameId(id.to_owned()))
}

/// Every frame in a `Page.getFrameTree` tree, its root first.
fn list_frames(tree: &Value, frames: &mut Vec<FrameId>) {
    if let Some(id) = tree["frame"]["id"].as_str() {
        frames.push(FrameId(id.to_owned()));
    }
    for child in tree["childFrames"].as_array().into_iter().flatten() {
        list_frames(child, frames);
    }
}

async fn create_world(session: &ProtocolSession, frame: &FrameId) -> Result<i64, CoreError> {
    let world = step(
        session,
        "Page.createIsolatedWorld",
        json!({ "frameId": frame.as_str(), "worldName": SEALED_WORLD }),
    )
    .await?;
    world["executionContextId"]
        .as_i64()
        .ok_or_else(|| CoreError::Message("Page.createIsolatedWorld gave no context".into()))
}

async fn install_seal(session: &ProtocolSession, context: i64) -> Result<(), CoreError> {
    let reply = step(
        session,
        "Runtime.evaluate",
        json!({ "expression": seal_source(), "contextId": context, "returnByValue": true }),
    )
    .await?;
    if script_value(&reply)?.is_number() {
        Ok(())
    } else {
        Err(CoreError::Message(
            "the seal did not report its version".into(),
        ))
    }
}

/// Watches one closed shadow root, through the remote object its node resolves into in the
/// sealed world. The object is released whether or not the watch ran: it would otherwise live
/// as long as the root's document, and every sweep and every arming resolves the roots again.
async fn watch(
    session: &ProtocolSession,
    backend_node: i64,
    context: i64,
) -> Result<(), CoreError> {
    let resolved = step(
        session,
        "DOM.resolveNode",
        json!({ "backendNodeId": backend_node, "executionContextId": context }),
    )
    .await?;
    let object = resolved["object"]["objectId"]
        .as_str()
        .ok_or_else(|| CoreError::Message("the closed shadow root did not resolve".into()))?;
    let watched = step(
        session,
        "Runtime.callFunctionOn",
        json!({ "objectId": object, "functionDeclaration": WATCH, "returnByValue": true }),
    )
    .await;
    let released = step(
        session,
        "Runtime.releaseObject",
        json!({ "objectId": object }),
    )
    .await;
    if let Err(error) = released {
        tracing::debug!(%error, "a closed shadow root's handle was not released; it goes with its document");
    }
    script_value(&watched?).map(drop)
}

fn skipped(frame: &FrameId, error: &CoreError) {
    tracing::warn!(frame = frame.as_str(), %error, "the sealed world skipped a frame");
}

/// What a script in the sealed world returned by value, or the exception it threw. The
/// exception's description comes from Neo's own code there, never from a field.
pub(crate) fn script_value(reply: &Value) -> Result<&Value, CoreError> {
    match reply.get("exceptionDetails") {
        Some(exception) => Err(CoreError::PageScript(
            exception["exception"]["description"]
                .as_str()
                .or_else(|| exception["text"].as_str())
                .unwrap_or("the script threw")
                .to_owned(),
        )),
        None => Ok(&reply["result"]["value"]),
    }
}

/// One CDP call on a frame's target, within [`FRAME_BUDGET`].
pub(crate) async fn step(
    session: &ProtocolSession,
    method: &str,
    params: Value,
) -> Result<Value, CoreError> {
    within_budget(method, session.send_value(method, params)).await
}

/// `work`, or a timeout error naming `what` once [`FRAME_BUDGET`] passes.
async fn within_budget<T>(
    what: &str,
    work: impl Future<Output = Result<T, CoreError>>,
) -> Result<T, CoreError> {
    tokio::time::timeout(FRAME_BUDGET, work)
        .await
        .unwrap_or_else(|_| {
            Err(CdpError::Timeout {
                method: what.to_owned(),
            }
            .into())
        })
}
