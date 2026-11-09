//! Neo: one tab's frame tree as the run's frame events see it, for `frameattached`,
//! `framedetached` and `framenavigated` on the page and the context. Playwright's server keeps
//! a frame tree from the protocol (`server/frames.ts` `FrameManager`) and its client emits the
//! page's frame events from that tree's changes; this model follows the same changes from the
//! tab watcher's CDP events, so the events name the same frames in the same order.
//!
//! ```text
//!  Page.frameAttached                         ─▶ add the frame             ─▶ frameAttached
//!  Page.frameDetached (reason remove)         ─▶ drop it and its subtree   ─▶ frameDetached, leaves first
//!  Page.frameDetached (reason swap)           ─▶ nothing: the frame moves to another process
//!  Page.frameNavigated (a new document)       ─▶ drop its subtree, update  ─▶ frameDetached…, frameNavigated
//!  Page.navigatedWithinDocument               ─▶ update its URL            ─▶ frameNavigated
//! ```
//!
//! Chromium reports no `frameDetached` for a same-process child frame whose parent document
//! goes away (measured on BrowserOS neo, `page.rs` `FrameFeeds::detach_all`), so the model drops
//! that subtree itself on the parent's commit, as Playwright's `removeChildFramesRecursively`
//! does. CDP does not replay existing frames, so the model starts from the tab's frame tree read
//! when its watcher starts ([`FrameModel::seed`]).
//!
//! Deviation: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client.send('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: true, flatten: true }),`
//! Playwright attaches every out-of-process frame as it starts, paused until attached. Neo
//! attaches one when the run first reaches into it (a frame read, a locator in it), so that
//! frame's own commits before then are not reported; its attach and removal, which the tab's
//! session reports, always are.

use std::collections::HashMap;

use browseros_cdp::CdpEvent;
use neo_runtime_spec::serve::{FrameChannel, FrameInitializer};
use serde_json::{Value, json};

use crate::chromium_bridge::cdp::{self, page};
use crate::chromium_bridge::frames::FrameInfo;

/// One frame change for the run's listeners: the Neo event method on the page's guid, the
/// listener event a script subscribes to (page and context share the name), and its params.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrameChange {
    pub(crate) method: &'static str,
    pub(crate) listener: &'static str,
    pub(crate) params: Value,
}

/// What the model knows of one frame.
#[derive(Debug, Clone, Default)]
struct Known {
    parent: Option<String>,
    name: String,
    url: String,
}

/// The frames of one tab, by frame id, the main frame included (its id is the tab's target id).
#[derive(Debug)]
pub(crate) struct FrameModel {
    target: String,
    frames: HashMap<String, Known>,
}

impl FrameModel {
    pub(crate) fn new(target: &str) -> Self {
        let mut frames = HashMap::new();
        frames.insert(target.to_string(), Known::default());
        Self {
            target: target.to_string(),
            frames,
        }
    }

    /// Adds the frames of a frame-tree read that no event has reported yet. An event applied
    /// before the read's reply already left the model as the read describes it.
    pub(crate) fn seed(&mut self, tree: &[FrameInfo]) {
        for frame in tree {
            let known = self.frames.entry(frame.id.clone()).or_default();
            if known.url.is_empty() {
                known.url.clone_from(&frame.url);
                known.name.clone_from(&frame.name);
            }
            if known.parent.is_none() && frame.id != self.target {
                known.parent.clone_from(&frame.parent);
            }
        }
    }

    /// Applies one `Page` event of the tab's session or of one of its frame sessions, and
    /// returns the frame changes it makes, in Playwright's order.
    /// Whether `frame` is one of the tab's frames the model knows.
    pub(crate) fn contains(&self, frame: &str) -> bool {
        self.frames.contains_key(frame)
    }

    pub(crate) fn apply(&mut self, event: &CdpEvent) -> Vec<FrameChange> {
        let mut changes = Vec::new();
        if let Some(page::FrameAttached {
            frame_id,
            parent_frame_id: Some(parent_frame_id),
        }) = cdp::decode(event)
        {
            if !self.frames.contains_key(&frame_id) {
                self.frames.insert(
                    frame_id.clone(),
                    Known {
                        parent: Some(parent_frame_id),
                        ..Known::default()
                    },
                );
                changes.push(self.change("frameAttached", "frameattached", &frame_id));
            }
        } else if let Some(page::FrameDetached { frame_id, reason }) = cdp::decode(event) {
            if reason.as_deref() != Some("swap") && frame_id != self.target {
                self.detach(&frame_id, true, &mut changes);
            }
        } else if let Some(page::FrameNavigated { frame, .. }) = cdp::decode(event) {
            let url = format!("{}{}", frame.url, frame.url_fragment.unwrap_or_default());
            if self.frames.contains_key(&frame.id) {
                self.detach(&frame.id, false, &mut changes);
            }
            let parent = (frame.id != self.target)
                .then_some(frame.parent_id)
                .flatten();
            let known = self.frames.entry(frame.id.clone()).or_default();
            known.url = url;
            known.name = frame.name.unwrap_or_default();
            if known.parent.is_none() {
                known.parent = parent;
            }
            changes.push(self.navigated(&frame.id));
        } else if let Some(page::NavigatedWithinDocument {
            frame_id,
            url: Some(url),
        }) = cdp::decode(event)
            && let Some(known) = self.frames.get_mut(&frame_id)
        {
            known.url = url;
            changes.push(self.navigated(&frame_id));
        }
        changes
    }

    /// Playwright: `server/frames.ts` `FrameManager.removeChildFramesRecursively`: `for (const child of frame.childFrames())`
    /// Drops `frame`'s subtree, children first, and `frame` itself when `itself`.
    fn detach(&mut self, frame: &str, itself: bool, changes: &mut Vec<FrameChange>) {
        let mut children: Vec<String> = self
            .frames
            .iter()
            .filter(|(_, known)| known.parent.as_deref() == Some(frame))
            .map(|(id, _)| id.clone())
            .collect();
        children.sort();
        for child in children {
            self.detach(&child, true, changes);
        }
        if itself && self.frames.contains_key(frame) {
            changes.push(self.change("frameDetached", "framedetached", frame));
            self.frames.remove(frame);
        }
    }

    /// Playwright: `server/frames.ts` `FrameManager.frameCommittedSameDocumentNavigation`: `const navigationEvent: NavigationEvent = { url, name: frame._name, isPublic: true };`
    /// A navigation names the frame's URL and name, within the document too.
    fn navigated(&self, frame: &str) -> FrameChange {
        let mut change = self.change("frameNavigated", "framenavigated", frame);
        let known = self.frames.get(frame).cloned().unwrap_or_default();
        change.params["url"] = json!(known.url);
        change.params["name"] = json!(known.name);
        change
    }

    fn change(&self, method: &'static str, listener: &'static str, frame: &str) -> FrameChange {
        FrameChange {
            method,
            listener,
            params: json!({ "frame": self.record(frame) }),
        }
    }

    /// The frame's record, as the frame getters mint it (`getters.rs` `frame_record`).
    fn record(&self, frame: &str) -> Value {
        let known = self.frames.get(frame).cloned().unwrap_or_default();
        let initializer = FrameInitializer {
            url: known.url,
            name: known.name,
            parent_frame: known.parent.map(|parent| FrameChannel {
                guid: self.guid(&parent),
            }),
            load_states: Vec::new(),
        };
        json!({ "guid": self.guid(frame), "initializer": initializer })
    }

    fn guid(&self, frame: &str) -> String {
        if frame == self.target {
            format!("frame@{}", self.target)
        } else {
            format!("frame@{}/{frame}", self.target)
        }
    }
}

#[cfg(test)]
mod tests {
    use browseros_cdp::SessionId;

    use super::*;

    const TAB: &str = "TAB";

    fn event(method: &str, params: Value) -> CdpEvent {
        CdpEvent {
            method: method.to_string(),
            params,
            session_id: Some(SessionId::from("s")),
        }
    }

    fn names(changes: &[FrameChange]) -> Vec<(String, String)> {
        changes
            .iter()
            .map(|change| {
                (
                    change.method.to_string(),
                    change.params["frame"]["guid"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn tests_that_a_child_frame_attaches_navigates_and_detaches_with_its_record() {
        let mut model = FrameModel::new(TAB);
        let attached = model.apply(&event(
            "Page.frameAttached",
            json!({"frameId": "A", "parentFrameId": TAB}),
        ));
        assert_eq!(
            names(&attached),
            [("frameAttached".into(), "frame@TAB/A".into())]
        );
        assert_eq!(
            attached[0].params["frame"]["initializer"]["parentFrame"]["guid"],
            "frame@TAB"
        );
        let navigated = model.apply(&event(
            "Page.frameNavigated",
            json!({"frame": {"id": "A", "parentId": TAB, "name": "x", "url": "https://a.test/", "urlFragment": "#f"}}),
        ));
        assert_eq!(navigated[0].params["url"], "https://a.test/#f");
        assert_eq!(navigated[0].params["name"], "x");
        let within = model.apply(&event(
            "Page.navigatedWithinDocument",
            json!({"frameId": "A", "url": "https://a.test/#g"}),
        ));
        assert_eq!(within[0].params["url"], "https://a.test/#g");
        let swapped = model.apply(&event(
            "Page.frameDetached",
            json!({"frameId": "A", "reason": "swap"}),
        ));
        assert!(swapped.is_empty(), "a process swap is no detach");
        let detached = model.apply(&event(
            "Page.frameDetached",
            json!({"frameId": "A", "reason": "remove"}),
        ));
        assert_eq!(
            names(&detached),
            [("frameDetached".into(), "frame@TAB/A".into())]
        );
        let again = model.apply(&event("Page.frameDetached", json!({"frameId": "A"})));
        assert!(again.is_empty(), "a frame detaches once");
    }

    #[test]
    fn tests_that_a_new_document_detaches_its_subtree_leaves_first_before_it_navigates() {
        let mut model = FrameModel::new(TAB);
        model.seed(&[
            FrameInfo {
                id: TAB.into(),
                parent: None,
                name: String::new(),
                url: "https://a.test/".into(),
            },
            FrameInfo {
                id: "A".into(),
                parent: Some(TAB.into()),
                name: String::new(),
                url: "https://a.test/a".into(),
            },
            FrameInfo {
                id: "B".into(),
                parent: Some("A".into()),
                name: String::new(),
                url: "https://a.test/b".into(),
            },
        ]);
        let changes = model.apply(&event(
            "Page.frameNavigated",
            json!({"frame": {"id": TAB, "url": "https://a.test/next"}}),
        ));
        assert_eq!(
            names(&changes),
            [
                ("frameDetached".into(), "frame@TAB/B".into()),
                ("frameDetached".into(), "frame@TAB/A".into()),
                ("frameNavigated".into(), "frame@TAB".into()),
            ]
        );
        assert!(
            changes[2].params["frame"]["initializer"]["parentFrame"].is_null(),
            "the main frame has no parent"
        );
    }
}
