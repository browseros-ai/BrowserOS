//! One run's Playwright state, owned by the host so it lives exactly as long as the run.

use crate::events::RunEvents;

/// One run's Playwright state: what the run listens for and how far its event pump has read.
/// The host creates it with `Default`, keeps it in its run slot and drops it with the run;
/// the crate reaches it through `OpEnv::run_state`. Opaque, so its contents can change without
/// the host noticing.
///
/// The event hub holds runs weakly, so dropping this is how a run's listeners and the dialogs
/// held for them go away.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct RunState {
    pub(crate) events: RunEvents,
}
