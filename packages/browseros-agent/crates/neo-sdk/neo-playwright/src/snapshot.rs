//! Playwright's aria snapshots, read from the tab's APC tree and printed by neo-snapshot, in Rust
//! or, on the prelude's path until its renderer is removed, by the prelude.
//!
//! ```text
//!  snapshot_op         the ops; and the baseline read and pending record behind
//!                      `ariaSnapshot({ delta: true })` on the prelude's path
//!  tree_snapshot       the reads: one APC capture of the tab, the tree's aria emission, and
//!                      what the page still finds (the cover survey, closed-root seeds); the
//!                      tree's names for what a page answer pinned, which a covered action's
//!                      answer shares (`PinNames`)
//!  snapshot_text       the text printed in Rust (neo_snapshot::render) when the op is asked
//!                      for one; a whole page's baseline under the call's clock, after the
//!                      event wait it takes inside the handler
//!  aria_snapshot       the ops' output: the tree, its partial frames and on-top lines
//!  snapshot_baselines  each tab's last whole-page v1 text its caller was given, across runs
//!  pending_baselines   one run's texts, committed or dropped when the run ends
//!  snapshot_calls      each whole-page call by id and when its timeout passes, which the
//!                      baseline ops after its tree are held to
//! ```

mod aria_snapshot;
mod pending_baselines;
mod snapshot_baselines;
mod snapshot_calls;
pub(crate) mod snapshot_op;
mod snapshot_text;
mod tree_snapshot;

pub(crate) use tree_snapshot::{
    Named, PinNames, element_in_tree, element_ref, expect_body_snapshot,
};

pub(crate) use pending_baselines::PendingBaselines;
#[cfg(test)]
pub(crate) use snapshot_baselines::TextForm;
pub(crate) use snapshot_baselines::{Baseline, SnapshotBaselines, SnapshotForm};
pub(crate) use snapshot_calls::SnapshotCalls;
pub(crate) use snapshot_op::SnapshotOp;
