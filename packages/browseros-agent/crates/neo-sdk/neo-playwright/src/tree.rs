//! Per-tab APC capture ownership, request-time coalescing, safe hydration, and host-private
//! node mirroring for `neo-playwright`.
//!
//! ```text
//!  attempt ─▶ TabTree ─▶ PageCapture (Page.getAnnotatedPageContent + contract gate + coalescing)
//!                ├─▶ FrameWorlds / DocTable (DocToken ⇄ FrameSeq + section 6.2 world proof)
//!                ├─▶ TabRefTable (neo_snapshot_apc::refs: NodeKey ⇄ Ref across captures)
//!                ├─▶ PrivateNodes (host-typed secret marks by site and DomNodeId)
//!                └─▶ Hydrate (DOM.resolveNode + probe pin under a proven pre-capture world)
//! ```

#![cfg_attr(not(test), expect(dead_code, reason = "wired by the switch-over"))]

mod hydrate;
mod page_capture;
mod private_nodes;
mod tab_tree;

pub(crate) use hydrate::{Hydrate, HydrateOutcome, HydratedNode};
pub(crate) use page_capture::{AttemptStart, CaptureError, CapturedTree, PageCapture};
pub(crate) use private_nodes::{PrivateNodes, SiteKey, TabOriginState};
pub(crate) use tab_tree::TabTree;
