//! A frame as an event recorded it, and one public frame transition. The tab lists that produce
//! them are `TabFrames`' (`tab_frames.rs`): a later tree snapshot cannot reconstruct removed
//! frames or the child-before-parent detach order, so the lists follow the events.

use browseros_core::{FrameId, TargetId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::events::PageEventKind;

/// The Frame fields delivered to JavaScript before its callback runs: the shape of a page's
/// `FrameInfo`, as the frame stood when the watcher recorded the event.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventFrame {
    pub(crate) id: FrameId,
    #[serde(default)]
    pub(crate) parent_id: Option<FrameId>,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) url: String,
    /// The URL's fragment, which CDP reports apart from it (`join_url_fragment`).
    #[serde(default, skip_serializing)]
    #[ts(skip)]
    pub(crate) url_fragment: String,
    #[serde(default, skip_serializing)]
    #[ts(skip)]
    pub(crate) loader_id: Option<String>,
}

/// One public frame transition, separately from the navigation waiter's loader facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameLifecycle {
    pub(crate) target: TargetId,
    pub(crate) kind: PageEventKind,
    pub(crate) frame: EventFrame,
}

impl EventFrame {
    /// Puts the frame's URL fragment back on its URL: CDP reports the two apart.
    pub(crate) fn join_url_fragment(&mut self) {
        self.url.push_str(&self.url_fragment);
    }
}
