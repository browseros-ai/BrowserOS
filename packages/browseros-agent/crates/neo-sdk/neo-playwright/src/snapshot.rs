//! Playwright's aria snapshots: each frame snapshots itself in its own isolated world, Rust
//! stitches the frames, and the text is printed by neo-snapshot, in Rust or, on the prelude's
//! path until its renderer is removed, by the prelude.
//!
//! ```text
//!  snapshot_op         the ops: reads each frame through its probe, child frames on their own
//!                      budget; and the baseline read and pending record behind
//!                      `ariaSnapshot({ delta: true })` on the prelude's path
//!  snapshot_text       the text printed in Rust (neo_snapshot::render) when the op is asked
//!                      for one; a whole page's baseline under the call's clock, after the
//!                      event wait it takes inside the handler
//!  clickable_targets   CDP: which elements of a frame Chromium reports clickable, for its snapshot
//!  aria_snapshot       pure: stitches the frames' trees as upstream's server does
//!  snapshot_rules      pure: Neo's rule that a name-prohibited role prints no quoted name
//!  snapshot_baselines  each tab's last whole-page v1 text its caller was given, across runs
//!  pending_baselines   one run's texts, committed or dropped when the run ends
//!  snapshot_calls      each whole-page call by id and when its timeout passes, which the
//!                      baseline ops after its tree are held to
//! ```

mod aria_snapshot;
mod clickable_targets;
mod pending_baselines;
mod snapshot_baselines;
mod snapshot_calls;
pub(crate) mod snapshot_op;
mod snapshot_rules;
mod snapshot_text;

pub(crate) use pending_baselines::PendingBaselines;
#[cfg(test)]
pub(crate) use snapshot_baselines::TextForm;
pub(crate) use snapshot_baselines::{Baseline, SnapshotBaselines, SnapshotForm};
pub(crate) use snapshot_calls::SnapshotCalls;
pub(crate) use snapshot_op::SnapshotOp;
