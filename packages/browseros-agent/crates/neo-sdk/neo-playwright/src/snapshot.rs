//! Playwright's aria snapshots: each frame snapshots itself in its own isolated world, Rust
//! stitches the frames, and the prelude renders the YAML.
//!
//! ```text
//!  snapshot_op     the ops: reads each frame through its probe, child frames on their own budget
//!  aria_snapshot   pure: stitches the frames' trees as upstream's server does
//!  snapshot_rules  pure: Neo's rule that a name-prohibited role prints no quoted name
//! ```

mod aria_snapshot;
pub(crate) mod snapshot_op;
mod snapshot_rules;

pub(crate) use snapshot_op::SnapshotOp;
