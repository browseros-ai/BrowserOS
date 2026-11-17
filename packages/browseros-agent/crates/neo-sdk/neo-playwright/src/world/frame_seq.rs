//! The identities an isolated world is known by: which `Playwright` booted it, which document of
//! a page it serves, and which node in it.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// The `Playwright` instance a world answers to, random per instance. Worlds are shared by
/// every CDP connection to a tab (measured), so after a claw-sdk restart the new process meets
/// worlds the old one booted. Booting one with another epoch re-keys it: the probe forgets
/// every pin, ref, frame key and `_ariaRef` expando and takes the new seq, so a ref the agent
/// kept from before the restart fails instead of naming whatever a new snapshot minted under
/// the same string. The same epoch is a no-op `describe`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Epoch(String);

impl Epoch {
    /// A fresh epoch, unguessable to the page.
    pub(crate) fn random() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// The epoch as `boot` takes it.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// One frame document of a page. Refs print it (`f7e3` is element 3 of document 7; the main
/// frame's first document is 0 and prints bare, `e3`). A seq is never reused, and every new
/// document, of the main frame or a child frame, gets a new one (decisions.md), so a ref can
/// only ever reach the document that printed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub(crate) struct FrameSeq(pub(crate) u32);

/// CDP's `DOM.BackendNodeId`: a node's id within its document, stable while the node lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub(crate) struct BackendNodeId(pub(crate) i64);

/// CDP's `Runtime.ExecutionContextId`: a small per-renderer counter, so it may name another
/// context once its document is gone. Only `DOM.resolveNode` gets one (`IsolatedWorld`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ContextId(pub(crate) i64);

/// An element the probe pinned in one world, by number, so the calls of one op act on the
/// element its first call found (Playwright's inner retry loop does the same with a handle).
/// The probe keeps the last 512 and never reuses a number in a document, so an evicted or
/// removed pin reads as `detached` and sends the op back to re-resolve; it can never name
/// another element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub(crate) struct Pin(pub(crate) u32);
