//! Playwright's aria snapshots: each frame snapshots itself in its own isolated world, Rust
//! stitches the frames, and the prelude renders the text.
//!
//! ```text
//!  snapshot_op         the ops: reads each frame through its probe, child frames on their own
//!                      budget; and the baseline read and pending record behind
//!                      `ariaSnapshot({ delta: true })`
//!  clickable_targets   CDP: which elements of a frame Chromium reports clickable, for its snapshot
//!  aria_snapshot       pure: stitches the frames' trees as upstream's server does
//!  snapshot_rules      pure: Neo's rule that a name-prohibited role prints no quoted name
//!  snapshot_baselines  each tab's last whole-page v1 text its caller was given, across runs
//!  pending_baselines   one run's texts, committed or dropped when the run ends
//! ```

mod aria_snapshot;
mod clickable_targets;
mod pending_baselines;
mod snapshot_baselines;
pub(crate) mod snapshot_op;
mod snapshot_rules;

pub(crate) use pending_baselines::PendingBaselines;
pub(crate) use snapshot_baselines::SnapshotBaselines;
pub(crate) use snapshot_op::SnapshotOp;
