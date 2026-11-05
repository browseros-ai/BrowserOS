//! Whether snapshots mask sensitive fields structurally (claw-sdk) or keep today's heuristic
//! (claw-server). One setting per [`crate::BrowserSession`], read by every observer at capture
//! time, so a page observed before the host chose still follows the choice.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// How a snapshot shows the values of password, card and one-time-code fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SensitiveValues {
    /// claw-server's behavior, unchanged: Chromium's password bullets, card numbers cut to their
    /// last four and security codes in card forms hidden; everything else as the page holds it.
    #[default]
    Heuristic,
    /// sdk-privacy's L1: every field the shared rule table calls sensitive shows a fixed mask
    /// (a card number keeps its last four), and those fields' values are masked wherever else
    /// the snapshot repeats them. Falls back to accessibility-only rules for a frame whose DOM
    /// snapshot failed, never to raw values.
    Mask,
}

/// The shared, changeable form of [`SensitiveValues`].
#[derive(Clone, Debug, Default)]
pub(crate) struct SensitiveValuesSetting(Arc<AtomicBool>);

impl SensitiveValuesSetting {
    pub(crate) fn get(&self) -> SensitiveValues {
        // Acquire pairs with the Release in `set`: a capture that starts after the host chose
        // sees the choice.
        if self.0.load(Ordering::Acquire) {
            SensitiveValues::Mask
        } else {
            SensitiveValues::Heuristic
        }
    }

    pub(crate) fn set(&self, mode: SensitiveValues) {
        self.0
            .store(mode == SensitiveValues::Mask, Ordering::Release);
    }
}
