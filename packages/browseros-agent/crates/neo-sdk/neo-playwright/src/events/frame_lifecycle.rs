//! Frame identities and event-time metadata owned by the single CDP watcher. A later tree
//! snapshot cannot reconstruct removed frames or the child-before-parent detach order.

use std::collections::{HashMap, HashSet};

use browseros_core::{FrameId, TargetId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::events::PageEventKind;

/// The Frame fields delivered to JavaScript before its callback runs.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventFrame {
    pub(crate) id: FrameId,
    #[serde(default)]
    pub(crate) parent_id: Option<FrameId>,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) url: String,
    #[serde(default, skip_serializing)]
    url_fragment: String,
    #[serde(default, skip_serializing)]
    pub(crate) loader_id: Option<String>,
}

/// One public frame transition, separately from the navigation waiter's loader facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameLifecycle {
    pub(crate) target: TargetId,
    pub(crate) kind: PageEventKind,
    pub(crate) frame: EventFrame,
}

/// The watcher's live frame trees. Renderer swaps keep their entries; real removals delete
/// a subtree exactly once, including when a new document implicitly removes its children.
#[derive(Default)]
pub(crate) struct FrameLifecycleTracker {
    tabs: HashMap<TargetId, Vec<EventFrame>>,
    attached: HashSet<(TargetId, FrameId)>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Attached {
    frame_id: FrameId,
    parent_frame_id: FrameId,
}

#[derive(Deserialize)]
struct Navigated {
    frame: EventFrame,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SameDocument {
    frame_id: FrameId,
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Detached {
    frame_id: FrameId,
    reason: String,
}

impl FrameLifecycleTracker {
    pub(crate) fn forget(&mut self, target: &TargetId) {
        self.tabs.remove(target);
        self.attached.retain(|(tab, _)| tab != target);
    }

    /// Main-document state already ordered before its subsequent load notification.
    pub(crate) fn main(&self, target: &TargetId) -> Option<EventFrame> {
        self.tabs
            .get(target)?
            .iter()
            .find(|frame| frame.id.0 == target.0)
            .cloned()
    }

    /// Seed existing documents without inventing historical events. Actual queued attach
    /// notifications still announce their frame once, even if this snapshot read ahead.
    pub(crate) fn seed(&mut self, target: &TargetId, tree: Value) {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Tree {
            frame: EventFrame,
            #[serde(default)]
            child_frames: Vec<Tree>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Reply {
            frame_tree: Tree,
        }
        let Ok(reply) = serde_json::from_value::<Reply>(tree) else {
            return;
        };
        let frames = self.tabs.entry(target.clone()).or_default();
        let mut pending = vec![(reply.frame_tree, None)];
        while let Some((tree, parent)) = pending.pop() {
            let mut frame = tree.frame;
            if frame.parent_id.is_none() && frame.id.0 != target.0 {
                frame.parent_id = parent;
            }
            let id = frame.id.clone();
            if !frames.iter().any(|known| known.id == id) {
                frame.url.push_str(&frame.url_fragment);
                frames.push(frame);
            }
            pending.extend(
                tree.child_frames
                    .into_iter()
                    .rev()
                    .map(|child| (child, Some(id.clone()))),
            );
        }
    }

    /// Decode CDP at its boundary. All events remain in CDP order, including an OOPIF's
    /// commit whose payload omits the parent already learned from its attachment.
    pub(crate) fn record(
        &mut self,
        target: &TargetId,
        method: &str,
        params: &Value,
    ) -> Vec<FrameLifecycle> {
        let frames = self.tabs.entry(target.clone()).or_default();
        let mut events = Vec::new();
        match method {
            "Page.frameAttached" => {
                if let Ok(attached) = serde_json::from_value::<Attached>(params.clone())
                    && self
                        .attached
                        .insert((target.clone(), attached.frame_id.clone()))
                {
                    let mut frame = EventFrame {
                        id: attached.frame_id,
                        parent_id: Some(attached.parent_frame_id),
                        ..EventFrame::default()
                    };
                    if let Some(known) = frames.iter_mut().find(|known| known.id == frame.id) {
                        frame.loader_id.clone_from(&known.loader_id);
                        *known = frame.clone();
                    } else {
                        frames.push(frame.clone());
                    }
                    events.push(transition(target, PageEventKind::FrameAttached, frame));
                }
            }
            "Page.frameNavigated" => {
                if let Ok(Navigated { frame }) = serde_json::from_value(params.clone()) {
                    commit(frames, target, frame, &mut events);
                }
            }
            "Page.navigatedWithinDocument" => {
                if let Ok(navigation) = serde_json::from_value::<SameDocument>(params.clone()) {
                    let known = frames
                        .iter_mut()
                        .find(|frame| frame.id == navigation.frame_id);
                    let frame = if let Some(known) = known {
                        known.url = navigation.url;
                        known.clone()
                    } else {
                        let frame = EventFrame {
                            parent_id: (navigation.frame_id.0 != target.0)
                                .then(|| FrameId(target.0.clone())),
                            id: navigation.frame_id,
                            url: navigation.url,
                            ..EventFrame::default()
                        };
                        frames.push(frame.clone());
                        frame
                    };
                    events.push(transition(target, PageEventKind::FrameNavigated, frame));
                }
            }
            "Page.frameDetached" => {
                if let Ok(detached) = serde_json::from_value::<Detached>(params.clone())
                    && detached.reason != "swap"
                {
                    remove_frame(frames, &detached.frame_id, target, &mut events);
                }
            }
            _ => {}
        }
        for event in &events {
            if event.kind == PageEventKind::FrameDetached {
                self.attached
                    .remove(&(target.clone(), event.frame.id.clone()));
            }
        }
        events
    }
}

/// A replay of the seeded document must not detach its own children. A genuinely new
/// loader removes the old subtree before its public navigation, matching Playwright.
fn commit(
    frames: &mut Vec<EventFrame>,
    target: &TargetId,
    mut frame: EventFrame,
    events: &mut Vec<FrameLifecycle>,
) {
    // Playwright drops the old document's children before announcing commit.
    let same_document = frame.loader_id.is_some()
        && frames
            .iter()
            .any(|known| known.id == frame.id && known.loader_id == frame.loader_id);
    if !same_document {
        remove_children(frames, &frame.id, target, events);
    }
    if let Some(known) = frames.iter_mut().find(|known| known.id == frame.id) {
        if frame.parent_id.is_none() && frame.id.0 != target.0 {
            frame.parent_id.clone_from(&known.parent_id);
        }
        frame.url.push_str(&frame.url_fragment);
        *known = frame.clone();
    } else {
        frame.url.push_str(&frame.url_fragment);
        frames.push(frame.clone());
    }
    events.push(transition(target, PageEventKind::FrameNavigated, frame));
}

fn transition(target: &TargetId, kind: PageEventKind, frame: EventFrame) -> FrameLifecycle {
    FrameLifecycle {
        target: target.clone(),
        kind,
        frame,
    }
}

fn remove_children(
    frames: &mut Vec<EventFrame>,
    parent: &FrameId,
    target: &TargetId,
    events: &mut Vec<FrameLifecycle>,
) {
    let children: Vec<FrameId> = frames
        .iter()
        .filter(|frame| frame.parent_id.as_ref() == Some(parent))
        .map(|frame| frame.id.clone())
        .collect();
    for child in children {
        remove_frame(frames, &child, target, events);
    }
}

fn remove_frame(
    frames: &mut Vec<EventFrame>,
    id: &FrameId,
    target: &TargetId,
    events: &mut Vec<FrameLifecycle>,
) {
    remove_children(frames, id, target, events);
    if let Some(index) = frames.iter().position(|frame| frame.id == *id)
        && frames
            .get(index)
            .is_some_and(|frame| frame.parent_id.is_some())
    {
        events.push(transition(
            target,
            PageEventKind::FrameDetached,
            frames.remove(index),
        ));
    }
}

#[cfg(test)]
mod tests {
    use browseros_core::{FrameId, TargetId};
    use serde_json::json;
    use std::error::Error;

    use super::{EventFrame, FrameLifecycleTracker};
    use crate::RunState;
    use crate::events::{EventHub, PageEvent, PageEventKind};
    use crate::page::LoadState;

    #[test]
    fn tests_that_seeded_frames_are_removed_without_replaying_old_attachments()
    -> Result<(), Box<dyn Error>> {
        let mut tracker = FrameLifecycleTracker::default();
        let target = TargetId("main".to_owned());
        tracker.seed(
            &target,
            json!({"frameTree": {
                "frame": {"id":"main", "loaderId":"document", "url":"https://a.test/"},
                "childFrames": [{"frame":{"id":"existing", "url":"about:blank", "name":"before"}}]
            }}),
        );
        let replay = tracker.record(
            &target,
            "Page.frameNavigated",
            &json!({"frame":{"id":"main", "loaderId":"document", "url":"https://a.test/"}}),
        );
        assert_eq!(
            replay.len(),
            1,
            "the seeded document's own commit cannot detach its children"
        );
        let removed = tracker.record(
            &target,
            "Page.frameDetached",
            &json!({"frameId":"existing", "reason":"remove"}),
        );
        assert_eq!(
            removed.len(),
            1,
            "a frame that predates the watcher still emits its removal"
        );
        assert_eq!(
            removed
                .first()
                .ok_or("the existing frame must have a removal")?
                .frame
                .name,
            "before"
        );
        assert_eq!(
            removed
                .first()
                .and_then(|event| event.frame.parent_id.as_ref()),
            Some(&FrameId("main".to_owned()))
        );
        Ok(())
    }

    #[test]
    fn tests_that_swaps_keep_identity_and_document_replacement_detaches_children_first()
    -> Result<(), Box<dyn Error>> {
        let mut tracker = FrameLifecycleTracker::default();
        let target = TargetId("main".to_owned());
        tracker.record(
            &target,
            "Page.frameAttached",
            &json!({"frameId":"child", "parentFrameId":"main"}),
        );
        tracker.record(
            &target,
            "Page.frameAttached",
            &json!({"frameId":"nested", "parentFrameId":"child"}),
        );
        assert!(
            tracker
                .record(
                    &target,
                    "Page.frameDetached",
                    &json!({"frameId":"child", "reason":"swap"})
                )
                .is_empty(),
            "a renderer swap never detaches a Frame"
        );
        let navigated = tracker.record(
            &target,
            "Page.frameNavigated",
            &json!({"frame":{"id":"child", "url":"https://cross.test/", "name":"remote"}}),
        );
        assert_eq!(
            navigated.len(),
            2,
            "a new child document removes its old descendants"
        );
        assert_eq!(
            navigated
                .first()
                .ok_or("the replaced descendant must be detached")?
                .frame
                .id,
            FrameId("nested".to_owned())
        );
        assert_eq!(
            navigated
                .last()
                .and_then(|event| event.frame.parent_id.as_ref()),
            Some(&FrameId("main".to_owned())),
            "an OOPIF commit retains the embedding parent"
        );
        let removed = tracker.record(
            &target,
            "Page.frameNavigated",
            &json!({"frame":{"id":"main", "url":"https://other.test/"}}),
        );
        assert_eq!(
            removed.first().map(|event| event.kind),
            Some(PageEventKind::FrameDetached),
            "main-frame commit removes children before announcing navigation"
        );
        assert!(
            tracker
                .record(
                    &target,
                    "Page.frameDetached",
                    &json!({"frameId":"child", "reason":"remove"})
                )
                .is_empty(),
            "the subsequent CDP removal cannot emit a second detach"
        );
        Ok(())
    }

    #[test]
    fn navigation_publication_waits_for_its_loader_and_rejects_a_replacement()
    -> Result<(), Box<dyn Error>> {
        use std::sync::Arc;

        let (hub, mut writer) = EventHub::new();
        let run = RunState::default();
        let target = TargetId("main".to_owned());
        run.navigation_publications.watch(
            7,
            Arc::clone(&hub),
            FrameId("main".to_owned()),
            "wanted".to_owned(),
            LoadState::Load,
            0,
        );
        let mut tracker = FrameLifecycleTracker::default();
        for transition in tracker.record(
            &target,
            "Page.frameNavigated",
            &json!({"frame":{"id":"main","loaderId":"wanted","url":"https://a.test/"}}),
        ) {
            writer.record(PageEvent::FrameLifecycle(transition));
        }
        assert!(
            run.completed_navigation_loads().is_empty(),
            "a commit is not a load"
        );
        let frame = |loader: &str| -> Result<EventFrame, serde_json::Error> {
            serde_json::from_value(json!({"id":"main","loaderId":loader,"url":"https://a.test/"}))
        };
        writer.record(PageEvent::Load {
            target: target.clone(),
            kind: PageEventKind::Load,
            frame: frame("older")?,
        });
        assert!(
            run.completed_navigation_loads().is_empty(),
            "another loader cannot lend its load state"
        );
        writer.record(PageEvent::Load {
            target: target.clone(),
            kind: PageEventKind::Load,
            frame: frame("wanted")?,
        });
        assert_eq!(run.completed_navigation_loads(), vec![7]);
        for transition in tracker.record(
            &target,
            "Page.frameNavigated",
            &json!({"frame":{"id":"main","loaderId":"replacement","url":"https://b.test/"}}),
        ) {
            writer.record(PageEvent::FrameLifecycle(transition));
        }
        assert!(
            run.completed_navigation_loads().is_empty(),
            "a replaced document does not prove current readiness"
        );
        run.navigation_publications.acknowledge(&[7]);
        assert!(run.completed_navigation_loads().is_empty());
        Ok(())
    }
}
