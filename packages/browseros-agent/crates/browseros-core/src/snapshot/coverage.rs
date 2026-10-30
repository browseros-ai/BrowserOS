//! What a snapshot could read of its page.
//!
//! Every frame is read on its own, so one that fails leaves its siblings and their refs usable.
//! The capture then says what it missed. A frame it did not read has unknown contents, not
//! absent ones: nothing here means the page removed them, and a diff leaves such a frame out of
//! its comparison instead of reporting its lines as removed or, once it reads again, as added.

use crate::{CoreError, FrameId};
use browseros_cdp::CdpError;
use serde::Serialize;
use std::ops::Range;

/// The iframe element holding a child frame, as its parent document's capture saw it. Stable
/// while that element lives, so it names a frame whose own id could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameOwner {
    /// The frame whose document holds the element; `None` for the main frame.
    #[serde(rename = "frameId", skip_serializing_if = "Option::is_none")]
    pub parent: Option<FrameId>,
    pub backend_node_id: i64,
}

/// Where a frame's read stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameReadStage {
    /// Finding the frame an iframe element holds.
    Identify,
    /// Reaching the frame's protocol session and admitting it as part of this page.
    Session,
    /// Reading the frame's accessibility tree, which holds its contents.
    Ax,
    /// Scanning the frame's DOM for clickable elements its accessibility tree does not name.
    /// The contents were read; only those elements may be missing.
    CursorScan,
}

/// Why a read stopped, in a fixed vocabulary that is safe to show. A browser's own message can
/// carry page or profile details, so it goes only to the debug log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameReadReason {
    /// The browser did not answer in time.
    Timeout,
    /// The frame's session or the browser connection went away.
    Detached,
    /// BrowserOS refused the read: it could not admit the frame as part of this page.
    Refused,
    /// The browser reported no frame or document to read.
    NotFound,
    /// The browser answered the read with an error.
    BrowserError,
}

impl FrameReadReason {
    pub(crate) fn of(error: &CoreError) -> Self {
        match error {
            CoreError::Cdp(CdpError::Timeout { .. }) => Self::Timeout,
            CoreError::Cdp(
                CdpError::SessionGone
                | CdpError::ConnectionLost
                | CdpError::NotConnected
                | CdpError::ConnectionEpochChanged { .. },
            ) => Self::Detached,
            CoreError::Target(_) | CoreError::NativeFeatureUnavailable { .. } => Self::Refused,
            _ => Self::BrowserError,
        }
    }
}

/// A frame one capture did not fully read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameError {
    /// `None` for the main frame, and for a child whose iframe element exposed no frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<FrameId>,
    /// `None` only for the main frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<FrameOwner>,
    pub stage: FrameReadStage,
    pub reason: FrameReadReason,
}

impl FrameError {
    /// Whether the frame's contents are missing, not merely read without its clickable scan.
    #[must_use]
    pub fn contents_missing(&self) -> bool {
        self.stage != FrameReadStage::CursorScan
    }

    #[must_use]
    pub fn is_main_frame(&self) -> bool {
        self.owner.is_none()
    }

    fn names(&self, frame_id: Option<&FrameId>, owner: Option<&FrameOwner>) -> bool {
        match (&self.frame_id, frame_id) {
            (Some(ours), Some(theirs)) => ours == theirs,
            _ => self.owner.as_ref() == owner,
        }
    }
}

/// What one capture read. Complete when no frame failed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotCoverage {
    /// Every frame the capture did not fully read, in document order.
    pub frame_errors: Vec<FrameError>,
}

impl SnapshotCoverage {
    #[must_use]
    pub fn is_partial(&self) -> bool {
        !self.frame_errors.is_empty()
    }

    fn error_for(
        &self,
        frame_id: Option<&FrameId>,
        owner: Option<&FrameOwner>,
    ) -> Option<&FrameError> {
        self.frame_errors
            .iter()
            .find(|error| error.names(frame_id, owner))
    }
}

/// The lines of a capture's full text that hold one child frame's contents: everything after
/// its `- iframe` line that the frame and the frames inside it rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameSpan {
    pub frame_id: Option<FrameId>,
    pub owner: FrameOwner,
    pub lines: Range<usize>,
}

impl FrameSpan {
    #[must_use]
    pub fn shifted(mut self, offset: usize) -> Self {
        self.lines = self.lines.start + offset..self.lines.end + offset;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameRead {
    Absent,
    Missing,
    Degraded,
    Read,
}

fn frame_read(
    coverage: &SnapshotCoverage,
    frames: &[FrameSpan],
    frame_id: Option<&FrameId>,
    owner: Option<&FrameOwner>,
) -> FrameRead {
    if let Some(error) = coverage.error_for(frame_id, owner) {
        return if error.contents_missing() {
            FrameRead::Missing
        } else {
            FrameRead::Degraded
        };
    }
    let Some(owner) = owner else {
        return FrameRead::Read;
    };
    let read = frames.iter().any(|span| match (frame_id, &span.frame_id) {
        (Some(ours), Some(theirs)) => ours == theirs,
        _ => span.owner == *owner,
    });
    if read {
        FrameRead::Read
    } else {
        FrameRead::Absent
    }
}

/// The frames two captures read differently: one side holds contents the other did not read,
/// so none of their lines can be compared. A frame both sides missed alike, or one that is gone
/// from either side, compares as usual. Each carries the current capture's error when it has
/// one, else the baseline's.
pub(super) fn unevenly_read(
    before: (&SnapshotCoverage, &[FrameSpan]),
    after: (&SnapshotCoverage, &[FrameSpan]),
) -> Vec<FrameError> {
    let mut uneven: Vec<FrameError> = Vec::new();
    for error in after.0.frame_errors.iter().chain(&before.0.frame_errors) {
        let (frame_id, owner) = (error.frame_id.as_ref(), error.owner.as_ref());
        if uneven.iter().any(|seen| seen.names(frame_id, owner)) {
            continue;
        }
        let was = frame_read(before.0, before.1, frame_id, owner);
        let now = frame_read(after.0, after.1, frame_id, owner);
        if was != now && was != FrameRead::Absent && now != FrameRead::Absent {
            uneven.push(error.clone());
        }
    }
    uneven
}

/// `text` without the lines of `frames` that `uncompared` names; empty once the main frame is.
pub(super) fn compared_text(text: &str, frames: &[FrameSpan], uncompared: &[FrameError]) -> String {
    if uncompared.iter().any(FrameError::is_main_frame) {
        return String::new();
    }
    let lines = if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').collect::<Vec<_>>()
    };
    let mut keep = vec![true; lines.len()];
    for span in frames {
        if uncompared
            .iter()
            .any(|error| error.names(span.frame_id.as_ref(), Some(&span.owner)))
        {
            for kept in keep.iter_mut().take(span.lines.end).skip(span.lines.start) {
                *kept = false;
            }
        }
    }
    lines
        .into_iter()
        .zip(keep)
        .filter_map(|(line, kept)| kept.then_some(line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::{FrameError, FrameOwner, FrameReadReason, FrameReadStage};
    use crate::{CoreError, FrameId, PageId, TargetError};
    use browseros_cdp::CdpError;
    use serde_json::json;

    #[test]
    fn reasons_name_the_class_of_an_error_never_its_message() {
        let cases = [
            (
                CoreError::Cdp(CdpError::Timeout {
                    method: "Accessibility.getFullAXTree".to_string(),
                }),
                FrameReadReason::Timeout,
            ),
            (
                CoreError::Cdp(CdpError::SessionGone),
                FrameReadReason::Detached,
            ),
            (
                CoreError::Cdp(CdpError::ConnectionEpochChanged {
                    expected: 1,
                    actual: 2,
                }),
                FrameReadReason::Detached,
            ),
            (
                TargetError::PageTargetChanged(PageId(3)).into(),
                FrameReadReason::Refused,
            ),
            (
                CoreError::Cdp(CdpError::Protocol {
                    code: -32000,
                    message: "Frame with the given id was not found.".to_string(),
                }),
                FrameReadReason::BrowserError,
            ),
            (
                CoreError::Message("https://private.example/?token=1".to_string()),
                FrameReadReason::BrowserError,
            ),
        ];
        for (error, reason) in cases {
            assert_eq!(FrameReadReason::of(&error), reason, "{error}");
        }
    }

    #[test]
    fn frame_errors_serialize_with_only_the_identity_they_have() -> Result<(), serde_json::Error> {
        let nested = FrameError {
            frame_id: Some(FrameId("child".to_string())),
            owner: Some(FrameOwner {
                parent: Some(FrameId("parent".to_string())),
                backend_node_id: 7,
            }),
            stage: FrameReadStage::Session,
            reason: FrameReadReason::Detached,
        };
        let unidentified = FrameError {
            frame_id: None,
            owner: Some(FrameOwner {
                parent: None,
                backend_node_id: 9,
            }),
            stage: FrameReadStage::Identify,
            reason: FrameReadReason::NotFound,
        };
        let main = FrameError {
            frame_id: None,
            owner: None,
            stage: FrameReadStage::CursorScan,
            reason: FrameReadReason::Refused,
        };

        assert_eq!(
            serde_json::to_value([nested, unidentified, main])?,
            json!([
                {
                    "frameId": "child",
                    "owner": { "frameId": "parent", "backendNodeId": 7 },
                    "stage": "session",
                    "reason": "detached"
                },
                { "owner": { "backendNodeId": 9 }, "stage": "identify", "reason": "not_found" },
                { "stage": "cursor_scan", "reason": "refused" }
            ])
        );
        Ok(())
    }
}
