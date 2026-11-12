//! Playwright's aria snapshots: each frame snapshots itself in its own isolated world, Rust
//! stitches the frames, and the prelude renders the text.
//!
//! ```text
//!  snapshot_op         the ops: reads each frame through its probe, child frames on their own
//!                      budget; and the baseline swap behind `ariaSnapshot({ delta: true })`
//!  aria_snapshot       pure: stitches the frames' trees as upstream's server does
//!  snapshot_rules      pure: Neo's rule that a name-prohibited role prints no quoted name
//!  snapshot_baselines  each tab's last whole-page v1 text, kept across runs
//! ```

mod aria_snapshot;
mod snapshot_baselines;
pub(crate) mod snapshot_op;
mod snapshot_rules;

pub(crate) use snapshot_baselines::SnapshotBaselines;
pub(crate) use snapshot_op::SnapshotOp;
