//! One download a run was handed, as the run's record keeps it.
//!
//! claw-sdk builds these when the run ends, from the downloads its `download` events handed the
//! script, after it copied each finished file into the run's outputs (`run_outputs.rs`). They
//! are what `GET /v1/runs/{id}` lists; the kept bytes are what
//! `GET /v1/runs/{id}/downloads/{index}` serves.

use serde::{Deserialize, Serialize};

/// One download of a run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDownload {
    /// Its place among the run's downloads: the order the run first saw each, from 0.
    pub index: u32,
    /// The URL it came from.
    pub url: String,
    /// The name the page suggested.
    pub suggested_filename: String,
    /// Where the browser saved it, which `download.path()` returned. Set when `completed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// How many bytes Neo kept for the host. Set when `completed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// How it stood when the run ended.
    pub state: DownloadStatus,
    /// Why it failed, when it did: the browser's `canceled`, or why Neo could not keep it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

/// How a download stood when its run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    /// Still being written when the run ended. Neo kept no bytes: a script that needs the file
    /// awaits `download.path()` before it returns.
    InProgress,
    /// Finished, and Neo kept a copy.
    Completed,
    /// Canceled or interrupted, or finished but Neo could not keep it.
    Failed,
}

impl DownloadStatus {
    /// The stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}
