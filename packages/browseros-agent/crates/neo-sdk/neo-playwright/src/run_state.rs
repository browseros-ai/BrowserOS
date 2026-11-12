//! One run's Playwright state, owned by the host so it lives exactly as long as the run.

mod navigation_publications;
mod opened_tabs;

use std::sync::{Mutex, PoisonError};

use browseros_core::TabId;
use navigation_publications::NavigationPublications;
pub(crate) use opened_tabs::OpenedTabs;

use crate::DownloadFacts;
use crate::events::{RunDownloads, RunEvents};
use crate::locator_handler::LocatorHandlers;
use crate::object_group::ObjectGroup;
use crate::page::{FrameRoutes, InitScripts};
use crate::snapshot::{PendingBaselines, SnapshotCalls};

/// One run's Playwright state: event cursors, navigation publication receipts, discovered
/// frame endpoints, retained element objects, downloads, init scripts, what its snapshots
/// tell the run's reply, and the tabs it opened. The host creates it
/// with `Default`, keeps it in its run slot and drops it with the run; the crate reaches it through
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
    /// `page.addLocatorHandler` registrations; dropping them wakes any action still paused.
    pub(crate) locator_handlers: LocatorHandlers,
    /// The tabs whose whole-page v1 snapshot, taken without `delta`, found a stored baseline of
    /// the same tab and document (`page.snapshotPending`): each could have been a delta. In the
    /// order first seen, each once.
    full_snapshot_repeats: Mutex<Vec<TabId>>,
    /// The run's whole-page v1 texts, which become baselines only when the run ends with them in
    /// the caller's hands ([`RunState::settle_snapshot_baselines`]).
    pub(crate) pending_baselines: PendingBaselines,
    /// The run's whole-page snapshot calls and when each one's timeout passes: the baseline ops
    /// after a tree are held to their own call's, by Rust's clock alone.
    pub(crate) snapshot_calls: SnapshotCalls,
    /// The tabs the run opened and snapshotted: on a tab it opened and never snapshotted, a
    /// failed ref came from another tab, and its message says so.
    pub(crate) opened_tabs: OpenedTabs,
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

    /// The tabs this run snapshotted in full while a stored baseline of the same tab and document
    /// was there, so `{ delta: true }` would have printed only what changed. The host reads it
    /// once the run's ops drained and decides the reply's tip; the crate keeps no count of tips.
    #[must_use]
    pub fn full_snapshot_repeats(&self) -> Vec<TabId> {
        self.full_snapshot_repeats
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// `tab` goes on [`RunState::full_snapshot_repeats`], once.
    pub(crate) fn note_full_snapshot_repeat(&self, tab: TabId) {
        let mut tabs = self
            .full_snapshot_repeats
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !tabs.contains(&tab) {
            tabs.push(tab);
        }
    }

    /// Ends the run's whole-page snapshot texts, once its ops drained and its reply is built. When
    /// `ended_ok` (the run returned a value), each tab's last text whose returned string `given`
    /// finds in what the caller got (a string of the returned value, or a console line) becomes
    /// that tab's baseline for later deltas; every other text is dropped. A host that never
    /// calls it never stores a baseline, so every delta comes back whole.
    pub fn settle_snapshot_baselines(&self, ended_ok: bool, given: &dyn Fn(&str) -> bool) {
        self.pending_baselines.settle(ended_ok, given);
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
