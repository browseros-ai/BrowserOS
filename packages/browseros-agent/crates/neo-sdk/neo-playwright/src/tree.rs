//! Per-tab APC capture ownership, request-time coalescing, safe hydration, and host-private
//! node mirroring for `neo-playwright`.
//!
//! ```text
//!  attempt ─▶ TabTree ─▶ PageCapture (Page.getAnnotatedPageContent + contract gate + coalescing)
//!                ├─▶ FrameWorlds / DocTable (DocToken ⇄ FrameSeq + section 6.2 world proof)
//!                ├─▶ TabRefTable (neo_snapshot_apc::refs: NodeKey ⇄ Ref across captures)
//!                ├─▶ PrivateMirror (host-typed secret marks by DomNodeId and site, copied into
//!                │   every build as neo_snapshot_apc's PrivateNodes)
//!                ├─▶ Hydrate (DOM.resolveNode + probe pin under a proven pre-capture world)
//!                └─▶ ChunkWorld (a chunk run's page side: its tree nodes handed to the frame's
//!                    world under the same proof, and the tab's refs as its RefResolver)
//! ```

#![cfg_attr(not(test), expect(dead_code, reason = "wired by the switch-over"))]

mod chunk_world;
mod hydrate;
mod page_capture;
mod private_mirror;
mod tab_tree;
#[cfg(test)]
mod tree_script;

#[cfg_attr(not(test), expect(unused_imports, reason = "wired by the switch-over"))]
pub(crate) use chunk_world::{ChunkWorld, RunAttempt, RunRoot, Unproven};
pub(crate) use hydrate::{Hydrate, HydrateOutcome, HydratedNode};
pub(crate) use page_capture::{AttemptStart, CaptureError, CapturedTree, PageCapture};
pub(crate) use private_mirror::{PrivateMirror, TabOriginState};
pub(crate) use tab_tree::TabTree;
