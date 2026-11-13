//! What the trace keeps, as the host set it: Neo's own screenshots, whether values are stored,
//! and the size of values it keeps whole.

use crate::RedactionLimits;

/// The host's say over what the writer records (`trace.screenshots`, `trace.valueBytes`, `trace.values` on the
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
    /// Whether script and host values reach storage when their record ends.
    pub values: Values,
}

/// Neo's own screenshots (D7, `screenshot_rule`). A script's `page.screenshot()` is its result,
/// not one of these; its storage follows [`Values`].
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

/// Whether the trace stores script and host values. This governs persistence, never the reply
/// delivered to the caller, and is independent of Neo's own screenshots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Values {
    /// Keep today's bounded values.
    #[default]
    On,
    /// Keep metadata only. Error owner and code stay, with an empty message.
    Off,
}

impl Values {
    /// The stored omission marker; absent preserves historical records and default responses.
    #[must_use]
    pub const fn marker(self) -> Option<&'static str> {
        match self {
            Self::On => None,
            Self::Off => Some("off"),
        }
    }
}
