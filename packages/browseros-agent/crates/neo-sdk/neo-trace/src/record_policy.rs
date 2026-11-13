//! What the trace keeps, as the host set it: Neo's own screenshots and the size of the values it
//! keeps whole.

use crate::RedactionLimits;

/// The host's say over what the writer records (`trace.screenshots`, `trace.valueBytes` on the
/// host API). The app sends it with [`TraceSink::record_policy`](crate::TraceSink::record_policy)
/// at start and after every change; it travels in the writer's own queue, so it applies from the
/// next event the writer handles, in order with every record. The default is what the trace
/// recorded before the host could set it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecordPolicy {
    /// Whether Neo takes screenshots of its own for operations and run ends.
    pub screenshots: Screenshots,
    /// Keeps each operation's arguments and result, a run's result, and any one string in them
    /// whole up to this many bytes; `0` keeps the default limits (`RedactionLimits::default`).
    pub value_bytes: usize,
}

/// Neo's own screenshots (D7, `screenshot_rule`). A script's `page.screenshot()` is its result,
/// not one of these, and is kept either way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screenshots {
    /// After each write, after a failure the website or the script caused, and of each tab a run
    /// touched when it ends.
    #[default]
    PerOperation,
    /// None. A job made before the change is still taken.
    Off,
}

impl RecordPolicy {
    /// The size limits the writer bounds values with under this policy.
    #[must_use]
    pub fn limits(&self) -> RedactionLimits {
        if self.value_bytes == 0 {
            RedactionLimits::default()
        } else {
            RedactionLimits::new_with_value_bytes(self.value_bytes)
        }
    }
}
