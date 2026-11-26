//! The snapshot ops' output, the contract with the prelude that renders it: the page's nodes as
//! the APC tree emits them (`tree_snapshot.rs`), every frame in place, and the frames the
//! capture could not read.
//!
//! The YAML is rendered in the agent runtime, never in a page: rendering the tree in the root
//! frame's world would copy every cross-site frame's text into the embedding site's renderer
//! process, which site isolation exists to prevent.

use neo_snapshot::AriaNode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A frame the snapshot could not read. The prelude prints
/// `# partial: <frame> not read (<reason>)` under its iframe's line.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
pub(crate) struct PartialFrame {
    /// The iframe node's ref in its parent frame.
    pub(crate) r#ref: String,
    /// The frame's URL as far as Neo knows it, the iframe's `src`.
    pub(crate) frame: String,
    /// `timeout after 2000 ms`, `frame detached`, `target closed`.
    pub(crate) reason: String,
}

/// The snapshot ops' output, and the contract with the prelude that renders it:
///
/// ```json
/// {
///   "json": [{ "role": "generic", "ref": "e1", "children": [
///     { "role": "iframe", "ref": "e10", "children": [{ "role": "button", "name": "Go", "ref": "f1e2" }] },
///     { "role": "iframe", "ref": "e11" }
///   ] }],
///   "partial": [{ "ref": "e11", "frame": "https://pay.example/frame", "reason": "timeout after 2000 ms" }]
/// }
/// ```
///
/// `json` is upstream's `AriaSnapshotJSON` with every frame in place and the name rule applied.
/// The prelude renders it, as upstream's YAML (the vendored `renderAriaSnapshotAsYaml`)
/// or as the condensed v1 text (`prelude/snapshot-v1.ts`), and adds a `# partial` line under each
/// partial iframe. `tests/data/snapshot-output.json` is one recorded from real frames.
///
/// A whole page's snapshot also carries `document`, the tab's document generation read before
/// the tree, which the prelude hands back with the rendered text so the delta baseline is only
/// ever stored for the document it was read from (`snapshot_baselines.rs`).
///
/// `unmarked` and `closedRoots` are never set by the tree, which reads clickability and closed
/// shadow roots itself; the prelude still prints their notices when a producer sets them.
#[derive(Debug, PartialEq, Serialize, TS)]
pub(crate) struct SnapshotResult {
    // The nodes' Rust type lives in neo-snapshot, which knows nothing of ts-rs. The probe and the
    // renderer already share upstream's recursive node format, pinned by snapshot-output.json, so
    // the generated TypeScript names that type.
    #[ts(type = "import('../../vendored').AriaSnapshotJSON")]
    pub(crate) json: Vec<AriaNode>,
    pub(crate) partial: Vec<PartialFrame>,
    /// Only the root frame: a requested clickable check did not finish. The renderer explains
    /// the possible missing refs without turning this read’s status into a page change.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) unmarked: Option<Unmarked>,
    /// Pinned words from the main page alone; empty keeps every old output byte unchanged.
    #[serde(rename = "onTop", skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<Vec<SnapshotTop>>")]
    pub(crate) on_top: Vec<SnapshotTop>,
    /// Set on `page.ariaSnapshot` only; a locator's snapshot never becomes a baseline.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) document: Option<u32>,
    /// Set on `page.ariaSnapshot` only: the id of this snapshot call, which the baseline ops after
    /// it name so Rust holds them to this call's own deadline (`snapshot_calls.rs`).
    #[serde(rename = "snapshotCall", skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) snapshot_call: Option<u32>,
    /// Seeds of closed shadow roots, at least one per root, when not every one found was sent.
    #[serde(rename = "closedRoots", skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) closed_roots: Option<ClosedRoots>,
}

/// Why the root frame’s requested clickable check was unavailable. Rust owns this fact;
/// child-frame failures remain in their call logs and never become a page-wide notice.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub(crate) struct Unmarked {
    pub(crate) reason: String,
}

/// Rust's shared wording and reach; the tree's marks identify the whole-view content.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SnapshotTop {
    pub(crate) r#ref: String,
    pub(crate) line: String,
    pub(crate) whole_view: bool,
}

/// How many of the seeds clickable marking found inside closed shadow roots reached a probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
pub(crate) struct ClosedRoots {
    pub(crate) sent: u32,
    pub(crate) found: u32,
}
