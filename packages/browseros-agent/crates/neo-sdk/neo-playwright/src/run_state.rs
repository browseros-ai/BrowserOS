//! One run's Playwright state, owned by the host so it lives exactly as long as the run.

mod navigation_publications;

use navigation_publications::NavigationPublications;

use crate::DownloadFacts;
use crate::events::{RunDownloads, RunEvents};
use crate::object_group::ObjectGroup;
use crate::page::{FrameRoutes, InitScripts};

/// One run's Playwright state: event cursors, discovered frame endpoints, retained element
/// objects, the downloads it was handed and its init scripts. The host creates it with
/// `Default`, keeps it in its run slot and drops it with the run; the crate reaches it through
/// `OpEnv::run_state`. Opaque, so its contents can change without the host noticing.
///
/// The event hub holds runs weakly, so dropping this is how a run's listeners and the dialogs
/// held for them go away.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct RunState {
    pub(crate) events: RunEvents,
    pub(crate) navigation_publications: NavigationPublications,
    pub(crate) frames: FrameRoutes,
    /// Returned tags and chooser inputs retained until the script and all its handles end.
    pub(crate) objects: ObjectGroup,
    /// The run's init scripts. Dropping them removes each from the tabs it was added to, which
    /// is how a script ends with the run that registered it.
    pub(crate) init_scripts: InitScripts,
    pub(crate) downloads: RunDownloads,
}

impl RunState {
    /// How many `events.next` replies the run's pump has been given. The host reads it as each op
    /// ends and that op's reply carries it (`pumped`); the prelude holds the op's result until it
    /// has dispatched as many pump replies, so the listeners for events an op waited on
    /// (`Playwright::events_before_reply`) run before the script resumes past the op, however the
    /// host's promises settle. Every reply counted here exists, so that hold always ends.
    #[must_use]
    pub fn pump_replies(&self) -> u64 {
        self.events.pump_replies()
    }

    /// Navigation publication identities whose requested document reached its load state.
    /// The bridge combines these with fully settled native replies; a genuinely pending
    /// document never makes a later evaluate wait for it.
    #[must_use]
    pub fn completed_navigation_loads(&self) -> Vec<u64> {
        self.navigation_publications.ready()
    }

    /// Every download a `download` event handed this run, in the order it first saw each (the
    /// run's download index), as the browser last reported it. The host reads it after the run's
    /// ops drained and before it drops the state, to keep the finished files with the run.
    #[must_use]
    pub fn downloads(&self) -> Vec<DownloadFacts> {
        self.downloads.facts()
    }

    /// Removes every init script the run registered, from every tab, and awaits each removal.
    /// The host calls it after the run's ops drained and before it answers, so once the run's
    /// reply is out no document loads with the run's scripts, not even one the next run or the
    /// owner navigates to at once. Whatever it does not reach (a removal cut off, a straggling
    /// op's add) is removed when the state drops.
    pub async fn remove_init_scripts(&self) {
        self.init_scripts.remove_all().await;
    }
}
