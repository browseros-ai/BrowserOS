//! One run's Playwright state, owned by the host so it lives exactly as long as the run.

use crate::DownloadFacts;
use crate::events::{RunDownloads, RunEvents};
use crate::object_group::ObjectGroup;

/// One run's Playwright state: what the run listens for, how far its event pump has read, and
/// the downloads it was handed. The host creates it with `Default`, keeps it in its run slot and
/// drops it with the run; the crate reaches it through `OpEnv::run_state`. Opaque, so its
/// contents can change without the host noticing.
///
/// The event hub holds runs weakly, so dropping this is how a run's listeners and the dialogs
/// held for them go away.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct RunState {
    pub(crate) events: RunEvents,
    /// Inputs retained for chooser objects until the script and all its handles end.
    pub(crate) objects: ObjectGroup,
    pub(crate) downloads: RunDownloads,
}

impl RunState {
    /// Every download a `download` event handed this run, in the order it first saw each (the
    /// run's download index), as the browser last reported it. The host reads it after the run's
    /// ops drained and before it drops the state, to keep the finished files with the run.
    #[must_use]
    pub fn downloads(&self) -> Vec<DownloadFacts> {
        self.downloads.facts()
    }
}
