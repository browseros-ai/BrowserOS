//! What the host learns about a run's downloads when the run ends.
//!
//! A run is handed a download through its `download` event; [`RunState::downloads`] lists each
//! such download as the browser last reported it. The host keeps the finished files with the
//! run's record, so it reads these once, after the run's ops have drained.
//!
//! [`RunState::downloads`]: crate::RunState::downloads

use std::path::PathBuf;

/// One download a run was handed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DownloadFacts {
    /// The URL it came from, as `download.url()` reads it.
    pub url: String,
    /// The name the page suggested, as `download.suggestedFilename()` reads it.
    pub suggested_filename: String,
    /// How far it got.
    pub state: DownloadState,
}

/// How a download stands, as the browser's `Browser.downloadProgress` last reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadState {
    /// Still being written.
    InProgress,
    /// Finished. The file is where the browser saved it, under its download directory with the
    /// name the browser chose (` (1)` added on a clash): what `download.path()` returns.
    Completed(PathBuf),
    /// It will never finish: Playwright's `download.failure()`, such as `canceled`.
    Failed(String),
}

impl DownloadState {
    /// Whether the browser is still writing it.
    #[must_use]
    pub const fn is_in_progress(&self) -> bool {
        matches!(self, Self::InProgress)
    }
}
