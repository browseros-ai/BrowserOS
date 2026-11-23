//! Every tab's frames, as the target watcher's CDP events and seed reads left them, in the shared
//! event hub where any op can read them.
//!
//! ```text
//!  target watcher (the one writer) ── seed / record / changed / forget ──▶ TabFrames ◀── get /
//!    its tab lines order each tab's     (inside EventHub's log, under      versions / owner /
//!    events behind its tree reads        the log's one short lock)          main: any reader
//! ```
//!
//! The list follows Playwright's frame rules: a new document drops the old document's children,
//! a same-document navigation changes only the URL, a renderer swap keeps the frame, and a seed
//! read that lands after some of the tab's events merges with what they gave. The tab's frame
//! version lives beside the list, but only its own writers move it, which are the same ones as
//! before the list moved here: a frame event on the tab's lifecycle source, and an out-of-process
//! frame's session attaching or ending ([`TabFrames::changed`]). A seed never moves it. A file
//! tab's version rests on the session writers: its cross-site child's attach gives it one while
//! its `Page` stays off.

use std::collections::{HashMap, HashSet};

use browseros_core::{FrameId, TabId, TargetId};
use serde::Deserialize;
use serde_json::Value;

use crate::events::PageEventKind;
use crate::events::frame_lifecycle::{EventFrame, FrameLifecycle};

/// One frame of a tab's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedFrame {
    /// The frame's id; the main frame's is its tab's target id.
    pub id: FrameId,
    /// `None` for the tab's main frame.
    pub parent_id: Option<FrameId>,
    /// The frame's `name`, as its embedding iframe or its document set it.
    pub name: String,
    /// With its fragment, as the frame shows it.
    pub url: String,
    /// The document's loader, when an event or the seed named one.
    pub loader_id: Option<String>,
}

/// A tab's frames as the event log knows them ([`TabFrames::get`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabFrameList {
    /// The main frame first, once known; the rest in the order the tab's events and seeds
    /// gave them.
    pub frames: Vec<ListedFrame>,
    /// The tab's frame-tree version: the one every reply carries for the tab, `0` while no
    /// writer moved it.
    pub version: u64,
    /// Whether the list is known complete: a full read of the tab's own tree (its seed) landed,
    /// and so did the own subtree of every out-of-process frame session the watcher holds for
    /// the tab. Until then the list holds what the tab's events and the seeds that landed named;
    /// a frame session's frames from before its `Page` went on may be missing.
    pub whole: bool,
    /// The newest event-log seq the list includes: every record up to it is reflected in
    /// `frames`, so a reader that applies records past a list can skip those it covers. It can
    /// sit at the log's head with no record of the tab's own there: a seed, and a frame event the
    /// log writes no record for (a tab no session of core's is on), move it to the head too.
    pub events: u64,
}

/// Every tab's live frames, frame-tree version and tab id. Renderer swaps keep their entries;
/// real removals delete a subtree exactly once, including when a new document implicitly removes
/// its children.
#[derive(Debug, Default)]
pub(crate) struct TabFrames {
    tabs: HashMap<TargetId, TabList>,
    /// The frames whose attach was announced, so a repeated attach (a seed that read ahead of a
    /// queued one, a frame's own session reporting it again) announces nothing.
    attached: HashSet<(TargetId, FrameId)>,
    /// Each open tab's version: kept apart from `tabs` because the two are forgotten at
    /// different moments (a tab's list when the watcher drops the tab, its version with the tab's
    /// recorded close), as before the list moved here.
    versions: HashMap<TargetId, u64>,
    /// The last version handed out: versions grow across tabs, so a tab's never repeats.
    last_version: u64,
    /// Each target's tab id, as the watcher learned it: the wire keys versions by tab id.
    tab_ids: HashMap<TargetId, TabId>,
}

/// One tab's list and what it covers.
#[derive(Debug, Default)]
struct TabList {
    frames: Vec<EventFrame>,
    /// Whether the tab's own tree landed as a seed.
    seeded: bool,
    /// The out-of-process frames whose session the watcher holds as their source and whose own
    /// subtree has not landed: the list may lack their same-process children from before that
    /// session's `Page` went on, so it is not whole while any is here.
    unseeded: HashSet<FrameId>,
    events: u64,
}

impl TabList {
    fn whole(&self) -> bool {
        self.seeded && self.unseeded.is_empty()
    }
}

/// The reads a tab's list still lacks before it is whole ([`TabFrames::missing_seeds`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MissingSeeds {
    /// The tab's own tree never landed.
    pub(crate) own_tree: bool,
    /// The out-of-process frames whose own subtree never landed.
    pub(crate) frames: Vec<FrameId>,
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

impl TabFrames {
    /// Seeds `tab`'s existing documents from a `Page.getFrameTree` reply without inventing
    /// historical events, as of event-log seq `at_event`. Queued attach notifications still
    /// announce their frame once, even if this read ran ahead of them. Only the tab's own tree,
    /// its main frame at the root, makes the list whole: an out-of-process frame's session reads
    /// its own subtree only, which adds those frames and says nothing of the rest of the tab.
    pub(crate) fn seed(&mut self, tab: &TargetId, tree: Value, at_event: u64) {
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
        let root = reply.frame_tree.frame.id.clone();
        let full = root.0 == tab.0;
        let list = self.tabs.entry(tab.clone()).or_default();
        let frames = &mut list.frames;
        let mut pending = vec![(reply.frame_tree, None)];
        while let Some((tree, parent)) = pending.pop() {
            let mut frame = tree.frame;
            if frame.parent_id.is_none() && frame.id.0 != tab.0 {
                frame.parent_id = parent;
            }
            let id = frame.id.clone();
            frame.join_url_fragment();
            if let Some(known) = frames.iter_mut().find(|known| known.id == id) {
                // An event announced this frame before the read, often only its parent's
                // attach, with no document yet. The read is newer than every event applied for
                // the tab: every tree read, the tab's own session's and each frame session's,
                // goes out off the watcher's loop, and the tab's later events wait behind it in
                // the tab's line (`TabLine`) until it is applied, so the frame takes the read's
                // document. Its parent stays the one its events gave. Without this, a frame
                // that committed before its session's `Page` was on keeps an empty URL, and its
                // later removal reports that.
                known.url = frame.url;
                known.name = frame.name;
                known.loader_id = frame.loader_id;
                if known.parent_id.is_none() {
                    known.parent_id = frame.parent_id;
                }
            } else {
                frames.push(frame);
            }
            pending.extend(
                tree.child_frames
                    .into_iter()
                    .rev()
                    .map(|child| (child, Some(id.clone()))),
            );
        }
        if full {
            list.seeded = true;
        } else {
            list.unseeded.remove(&root);
        }
        list.events = at_event;
    }

    /// An out-of-process frame's session became `frame`'s source on `tab`: until its own subtree
    /// lands, the list may lack that frame's same-process children, so it is not whole.
    pub(crate) fn session_attached(&mut self, tab: &TargetId, frame: FrameId) {
        self.tabs
            .entry(tab.clone())
            .or_default()
            .unseeded
            .insert(frame);
    }

    /// `frame`'s session ended with no other to take over as its source: nothing reads its
    /// subtree any more, and its own removal, if it went, comes as a frame event.
    pub(crate) fn session_ended(&mut self, tab: &TargetId, frame: &FrameId) {
        if let Some(list) = self.tabs.get_mut(tab) {
            list.unseeded.remove(frame);
        }
    }

    /// What `tab`'s list still lacks before it is whole; `None` once it is whole. A tab with no
    /// list at all (its seed failed and it sent no frame event since) lacks its own tree.
    pub(crate) fn missing_seeds(&self, tab: &TargetId) -> Option<MissingSeeds> {
        let Some(list) = self.tabs.get(tab) else {
            return Some(MissingSeeds {
                own_tree: true,
                frames: Vec::new(),
            });
        };
        (!list.whole()).then(|| MissingSeeds {
            own_tree: !list.seeded,
            frames: list.unseeded.iter().cloned().collect(),
        })
    }

    /// Applies one CDP frame event of `tab`'s, in CDP order, and returns the public transitions
    /// it makes (an OOPIF's commit whose payload omits the parent keeps the one its attach gave).
    /// It moves no version and no `events`: the writer does both, as it writes the records.
    pub(crate) fn record(
        &mut self,
        tab: &TargetId,
        method: &str,
        params: &Value,
    ) -> Vec<FrameLifecycle> {
        let frames = &mut self.tabs.entry(tab.clone()).or_default().frames;
        let mut events = Vec::new();
        match method {
            "Page.frameAttached" => {
                if let Ok(attached) = serde_json::from_value::<Attached>(params.clone())
                    && self
                        .attached
                        .insert((tab.clone(), attached.frame_id.clone()))
                {
                    let mut frame = EventFrame {
                        id: attached.frame_id,
                        parent_id: Some(attached.parent_frame_id),
                        ..EventFrame::default()
                    };
                    if let Some(known) = frames.iter_mut().find(|known| known.id == frame.id) {
                        // A seed listed this frame before its queued attach arrived (the read
                        // ran ahead of the events). The attach still announces it once, under
                        // the parent it names, but keeps the document the seed gave it: an
                        // attach carries no document, and taking its empty URL lost the seed's
                        // until the frame's next commit.
                        known.parent_id = frame.parent_id;
                        frame = known.clone();
                    } else {
                        frames.push(frame.clone());
                    }
                    events.push(transition(tab, PageEventKind::FrameAttached, frame));
                }
            }
            "Page.frameNavigated" => {
                if let Ok(Navigated { frame }) = serde_json::from_value(params.clone()) {
                    commit(frames, tab, frame, &mut events);
                }
            }
            "Page.navigatedWithinDocument" => {
                if let Ok(navigation) = serde_json::from_value::<SameDocument>(params.clone()) {
                    let known = frames
                        .iter_mut()
                        .find(|frame| frame.id == navigation.frame_id);
                    let frame = if let Some(known) = known {
                        // The same document: only its URL changes.
                        known.url = navigation.url;
                        known.clone()
                    } else {
                        let frame = EventFrame {
                            parent_id: (navigation.frame_id.0 != tab.0)
                                .then(|| FrameId(tab.0.clone())),
                            id: navigation.frame_id,
                            url: navigation.url,
                            ..EventFrame::default()
                        };
                        frames.push(frame.clone());
                        frame
                    };
                    events.push(transition(tab, PageEventKind::FrameNavigated, frame));
                }
            }
            "Page.frameDetached" => {
                // A renderer swap (`reason: "swap"`) moves the frame to another process; the
                // frame stays.
                if let Ok(detached) = serde_json::from_value::<Detached>(params.clone())
                    && detached.reason != "swap"
                {
                    remove_frame(frames, &detached.frame_id, tab, &mut events);
                }
            }
            _ => {}
        }
        for event in &events {
            if event.kind == PageEventKind::FrameDetached {
                self.attached.remove(&(tab.clone(), event.frame.id.clone()));
                if let Some(list) = self.tabs.get_mut(tab) {
                    list.unseeded.remove(&event.frame.id);
                }
            }
        }
        events
    }

    /// Records that `tab`'s list includes every event-log record up to seq `at_event`.
    pub(crate) fn included(&mut self, tab: &TargetId, at_event: u64) {
        if let Some(list) = self.tabs.get_mut(tab) {
            list.events = at_event;
        }
    }

    /// Moves `tab`'s frame-tree version. Its writers are the watcher's: a frame event on the
    /// tab's lifecycle source, and an out-of-process frame's session attaching or ending.
    pub(crate) fn changed(&mut self, tab: TargetId) {
        self.last_version += 1;
        self.versions.insert(tab, self.last_version);
    }

    /// Records `tab`'s tab id, which the per-tab versions are keyed by on the wire.
    pub(crate) fn tab_id(&mut self, tab: TargetId, id: TabId) {
        self.tab_ids.insert(tab, id);
    }

    /// Forgets `tab`'s list, as the watcher drops the tab: its version and tab id stay until its
    /// recorded close ([`Self::closed`]).
    pub(crate) fn forget(&mut self, tab: &TargetId) {
        self.tabs.remove(tab);
        self.attached.retain(|(known, _)| known != tab);
    }

    /// Forgets `tab`'s version and tab id, with its recorded close.
    /// Whether `tab`'s tab id is known: from the watcher's listing until its close. The event log
    /// reads it to tell a live tab with no ring yet from one whose ring was removed.
    pub(crate) fn knows(&self, tab: &TargetId) -> bool {
        self.tab_ids.contains_key(tab)
    }

    pub(crate) fn closed(&mut self, tab: &TargetId) {
        self.versions.remove(tab);
        self.tab_ids.remove(tab);
    }

    /// Forgets every list, after a reconnect lost the events between: versions stay, as they
    /// must never repeat.
    pub(crate) fn reset_lists(&mut self) {
        self.tabs.clear();
        self.attached.clear();
    }

    /// `tab`'s frames, main frame first, with its version, whether its seed landed and the
    /// newest record the list includes; `None` for a tab neither a list nor a version names.
    pub(crate) fn get(&self, tab: &TargetId) -> Option<TabFrameList> {
        let list = self.tabs.get(tab);
        let version = self.versions.get(tab).copied();
        if list.is_none() && version.is_none() {
            return None;
        }
        let mut frames: Vec<ListedFrame> = list
            .map(|list| list.frames.iter().map(listed).collect())
            .unwrap_or_default();
        if let Some(main) = frames.iter().position(|frame| frame.id.0 == tab.0) {
            let main = frames.remove(main);
            frames.insert(0, main);
        }
        Some(TabFrameList {
            frames,
            version: version.unwrap_or(0),
            whole: list.is_some_and(TabList::whole),
            events: list.map_or(0, |list| list.events),
        })
    }

    /// Every tab's version whose tab id is known: every reply carries them, so a run learns that
    /// a tab's frames changed even from a reply about another tab.
    pub(crate) fn versions(&self) -> Vec<(TabId, u64)> {
        self.versions
            .iter()
            .filter_map(|(tab, version)| self.tab_ids.get(tab).map(|id| (*id, *version)))
            .collect()
    }

    /// The tab a live frame belongs to, same-process children without a target of their own
    /// included. Seeds and lifecycle events own this membership and its removal.
    pub(crate) fn owner(&self, frame: &FrameId) -> Option<&TargetId> {
        self.tabs.iter().find_map(|(tab, list)| {
            list.frames
                .iter()
                .any(|known| known.id == *frame)
                .then_some(tab)
        })
    }

    /// `tab`'s main frame, once its commit or seed was applied: main-document state already
    /// ordered before the tab's next load notification.
    pub(crate) fn main(&self, tab: &TargetId) -> Option<EventFrame> {
        self.tabs
            .get(tab)?
            .frames
            .iter()
            .find(|frame| frame.id.0 == tab.0)
            .cloned()
    }
}

fn listed(frame: &EventFrame) -> ListedFrame {
    ListedFrame {
        id: frame.id.clone(),
        parent_id: frame.parent_id.clone(),
        name: frame.name.clone(),
        url: frame.url.clone(),
        loader_id: frame.loader_id.clone(),
    }
}

/// A replay of the seeded document must not detach its own children. A genuinely new loader
/// removes the old subtree before its public navigation, matching Playwright.
fn commit(
    frames: &mut Vec<EventFrame>,
    tab: &TargetId,
    mut frame: EventFrame,
    events: &mut Vec<FrameLifecycle>,
) {
    // Playwright drops the old document's children before announcing commit.
    let same_document = frame.loader_id.is_some()
        && frames
            .iter()
            .any(|known| known.id == frame.id && known.loader_id == frame.loader_id);
    if !same_document {
        remove_children(frames, &frame.id, tab, events);
    }
    frame.join_url_fragment();
    if let Some(known) = frames.iter_mut().find(|known| known.id == frame.id) {
        if frame.parent_id.is_none() && frame.id.0 != tab.0 {
            frame.parent_id.clone_from(&known.parent_id);
        }
        *known = frame.clone();
    } else {
        frames.push(frame.clone());
    }
    events.push(transition(tab, PageEventKind::FrameNavigated, frame));
}

fn transition(tab: &TargetId, kind: PageEventKind, frame: EventFrame) -> FrameLifecycle {
    FrameLifecycle {
        target: tab.clone(),
        kind,
        frame,
    }
}

fn remove_children(
    frames: &mut Vec<EventFrame>,
    parent: &FrameId,
    tab: &TargetId,
    events: &mut Vec<FrameLifecycle>,
) {
    let children: Vec<FrameId> = frames
        .iter()
        .filter(|frame| frame.parent_id.as_ref() == Some(parent))
        .map(|frame| frame.id.clone())
        .collect();
    for child in children {
        remove_frame(frames, &child, tab, events);
    }
}

fn remove_frame(
    frames: &mut Vec<EventFrame>,
    id: &FrameId,
    tab: &TargetId,
    events: &mut Vec<FrameLifecycle>,
) {
    remove_children(frames, id, tab, events);
    if let Some(index) = frames.iter().position(|frame| frame.id == *id)
        && frames
            .get(index)
            .is_some_and(|frame| frame.parent_id.is_some())
    {
        events.push(transition(
            tab,
            PageEventKind::FrameDetached,
            frames.remove(index),
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_core::{FrameId, TabId, TargetId};
    use serde_json::{Value, json};

    use super::{MissingSeeds, TabFrameList, TabFrames};
    use crate::RunState;
    use crate::events::frame_lifecycle::EventFrame;
    use crate::events::{EventHub, PageEvent, PageEventKind};
    use crate::page::LoadState;

    /// The ids of `list`'s frames, in its order.
    fn ids(list: &TabFrameList) -> Vec<&str> {
        list.frames
            .iter()
            .map(|frame| frame.id.0.as_str())
            .collect()
    }

    /// `tab`'s list, which must exist.
    fn list(frames: &TabFrames, tab: &TargetId) -> Result<TabFrameList, Box<dyn Error>> {
        Ok(frames.get(tab).ok_or("the tab has a list")?)
    }

    /// A tree whose main frame `main` (document `main-doc`) holds `child` (document
    /// `child-doc`, named `slot`), which holds `grandchild`.
    fn three_levels() -> Value {
        json!({"frameTree": {
            "frame": {"id":"main", "loaderId":"main-doc", "url":"https://a.test/"},
            "childFrames": [{
                "frame": {"id":"child", "loaderId":"child-doc", "url":"https://a.test/child",
                    "name":"slot"},
                "childFrames": [{"frame": {"id":"grandchild", "loaderId":"grand-doc",
                    "url":"https://a.test/grand"}}]
            }]
        }})
    }

    #[test]
    fn tests_that_seeded_frames_are_removed_without_replaying_old_attachments()
    -> Result<(), Box<dyn Error>> {
        let mut frames = TabFrames::default();
        let target = TargetId("main".to_owned());
        frames.seed(
            &target,
            json!({"frameTree": {
                "frame": {"id":"main", "loaderId":"document", "url":"https://a.test/"},
                "childFrames": [{"frame":{"id":"existing", "url":"about:blank", "name":"before"}}]
            }}),
            0,
        );
        let replay = frames.record(
            &target,
            "Page.frameNavigated",
            &json!({"frame":{"id":"main", "loaderId":"document", "url":"https://a.test/"}}),
        );
        assert_eq!(
            replay.len(),
            1,
            "the seeded document's own commit cannot detach its children"
        );
        assert_eq!(
            frames.owner(&FrameId("existing".to_owned())),
            Some(&target),
            "the seed and its replay preserve the child's containing page"
        );
        let removed = frames.record(
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
        assert_eq!(
            frames.owner(&FrameId("existing".to_owned())),
            None,
            "a real removal clears the child's ownership"
        );
        frames.forget(&target);
        assert_eq!(
            frames.owner(&FrameId("main".to_owned())),
            None,
            "forgetting the page clears its remaining frame ownership"
        );
        Ok(())
    }

    /// A frame its parent's attach announced, which committed a document before its own session
    /// reported events, takes that document from its session's seed read, so its removal
    /// reports the document it had rather than an empty one.
    #[test]
    fn tests_that_a_seed_gives_an_announced_frame_its_document() -> Result<(), Box<dyn Error>> {
        let mut frames = TabFrames::default();
        let target = TargetId("main".to_owned());
        frames.seed(
            &target,
            json!({"frameTree": {"frame": {"id":"main", "loaderId":"document", "url":"https://a.test/"}}}),
            0,
        );
        let attached = frames.record(
            &target,
            "Page.frameAttached",
            &json!({"frameId":"ad", "parentFrameId":"main"}),
        );
        assert_eq!(attached.len(), 1, "the parent's attach announces the frame");
        frames.seed(
            &target,
            json!({"frameTree": {"frame": {"id":"ad", "parentId":"main", "loaderId":"ad-document",
                "url":"https://ad.test/", "name":"slot"}}}),
            0,
        );
        let removed = frames.record(
            &target,
            "Page.frameDetached",
            &json!({"frameId":"ad", "reason":"remove"}),
        );
        let frame = &removed.first().ok_or("the removal is reported")?.frame;
        assert_eq!(
            (
                frame.url.as_str(),
                frame.name.as_str(),
                frame.parent_id.as_ref().map(|parent| parent.0.as_str()),
            ),
            ("https://ad.test/", "slot", Some("main")),
            "the seed's document, under the parent the attach gave"
        );
        Ok(())
    }

    /// A seed that read a frame before its queued attach arrived keeps that frame's document:
    /// the attach announces the frame once, under its parent, and takes nothing from it.
    #[test]
    fn tests_that_a_late_attach_keeps_the_document_a_seed_gave() -> Result<(), Box<dyn Error>> {
        let mut frames = TabFrames::default();
        let target = TargetId("main".to_owned());
        frames.seed(
            &target,
            json!({"frameTree": {
                "frame": {"id":"main", "loaderId":"document", "url":"https://a.test/"},
                "childFrames": [{"frame":{"id":"child", "loaderId":"child-document",
                    "url":"https://a.test/child", "name":"slot"}}]
            }}),
            0,
        );
        let attached = frames.record(
            &target,
            "Page.frameAttached",
            &json!({"frameId":"child", "parentFrameId":"main"}),
        );
        assert_eq!(
            attached.len(),
            1,
            "the queued attach still announces the frame once"
        );
        let removed = frames.record(
            &target,
            "Page.frameDetached",
            &json!({"frameId":"child", "reason":"remove"}),
        );
        let frame = &removed.first().ok_or("the removal is reported")?.frame;
        assert_eq!(
            (frame.url.as_str(), frame.name.as_str()),
            ("https://a.test/child", "slot"),
            "the attach left the seeded document in place"
        );
        Ok(())
    }

    #[test]
    fn tests_that_swaps_keep_identity_and_document_replacement_detaches_children_first()
    -> Result<(), Box<dyn Error>> {
        let mut frames = TabFrames::default();
        let target = TargetId("main".to_owned());
        frames.record(
            &target,
            "Page.frameAttached",
            &json!({"frameId":"child", "parentFrameId":"main"}),
        );
        frames.record(
            &target,
            "Page.frameAttached",
            &json!({"frameId":"nested", "parentFrameId":"child"}),
        );
        assert!(
            frames
                .record(
                    &target,
                    "Page.frameDetached",
                    &json!({"frameId":"child", "reason":"swap"})
                )
                .is_empty(),
            "a renderer swap never detaches a Frame"
        );
        assert_eq!(
            frames.owner(&FrameId("child".to_owned())),
            Some(&target),
            "a renderer swap keeps the attached frame on the same page"
        );
        let navigated = frames.record(
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
        assert_eq!(
            frames.owner(&FrameId("nested".to_owned())),
            None,
            "a new document clears ownership of the replaced descendants"
        );
        let removed = frames.record(
            &target,
            "Page.frameNavigated",
            &json!({"frame":{"id":"main", "url":"https://other.test/"}}),
        );
        assert_eq!(
            removed.first().map(|event| event.kind),
            Some(PageEventKind::FrameDetached),
            "main-frame commit removes children before announcing navigation"
        );
        assert_eq!(
            frames.owner(&FrameId("child".to_owned())),
            None,
            "main-document replacement clears ownership of its old child frames"
        );
        assert!(
            frames
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
        let mut frames = TabFrames::default();
        for transition in frames.record(
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
        for transition in frames.record(
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

    /// A list holds the tab's main frame first, whatever order the tab's events named its
    /// frames in.
    #[test]
    fn tests_that_a_list_holds_the_main_frame_first() -> Result<(), Box<dyn Error>> {
        let mut frames = TabFrames::default();
        let tab = TargetId("main".to_owned());
        frames.record(
            &tab,
            "Page.frameAttached",
            &json!({"frameId":"child", "parentFrameId":"main"}),
        );
        frames.record(
            &tab,
            "Page.navigatedWithinDocument",
            &json!({"frameId":"main", "url":"https://a.test/#top"}),
        );
        assert_eq!(ids(&list(&frames, &tab)?), ["main", "child"]);
        Ok(())
    }

    /// A new document drops the old document's children: a child's commit drops its own, the
    /// main frame's drops every frame but itself.
    #[test]
    fn tests_that_a_new_document_drops_the_old_documents_children() -> Result<(), Box<dyn Error>> {
        let mut frames = TabFrames::default();
        let tab = TargetId("main".to_owned());
        frames.seed(&tab, three_levels(), 0);
        frames.record(
            &tab,
            "Page.frameNavigated",
            &json!({"frame":{"id":"child", "parentId":"main", "loaderId":"child-doc-2",
                "url":"https://a.test/child-2"}}),
        );
        assert_eq!(ids(&list(&frames, &tab)?), ["main", "child"]);
        frames.record(
            &tab,
            "Page.frameNavigated",
            &json!({"frame":{"id":"main", "loaderId":"main-doc-2", "url":"https://b.test/"}}),
        );
        let after = list(&frames, &tab)?;
        assert_eq!(ids(&after), ["main"]);
        assert_eq!(
            after.frames.first().map(|frame| frame.url.as_str()),
            Some("https://b.test/")
        );
        Ok(())
    }

    /// A same-document navigation changes only its frame's URL: the frame keeps its parent,
    /// name and document, and its children stay.
    #[test]
    fn tests_that_a_same_document_navigation_changes_only_the_url() -> Result<(), Box<dyn Error>> {
        let mut frames = TabFrames::default();
        let tab = TargetId("main".to_owned());
        frames.seed(&tab, three_levels(), 0);
        frames.record(
            &tab,
            "Page.navigatedWithinDocument",
            &json!({"frameId":"child", "url":"https://a.test/child#moved"}),
        );
        let after = list(&frames, &tab)?;
        assert_eq!(ids(&after), ["main", "child", "grandchild"]);
        let child = after
            .frames
            .iter()
            .find(|frame| frame.id.0 == "child")
            .ok_or("the child is listed")?;
        assert_eq!(
            (
                child.url.as_str(),
                child.name.as_str(),
                child.loader_id.as_deref(),
                child.parent_id.as_ref().map(|parent| parent.0.as_str()),
            ),
            (
                "https://a.test/child#moved",
                "slot",
                Some("child-doc"),
                Some("main")
            )
        );
        Ok(())
    }

    /// A renderer swap keeps its frame, children included; a removal drops it.
    #[test]
    fn tests_that_a_renderer_swap_keeps_the_frame() -> Result<(), Box<dyn Error>> {
        let mut frames = TabFrames::default();
        let tab = TargetId("main".to_owned());
        frames.seed(&tab, three_levels(), 0);
        frames.record(
            &tab,
            "Page.frameDetached",
            &json!({"frameId":"child", "reason":"swap"}),
        );
        assert_eq!(ids(&list(&frames, &tab)?), ["main", "child", "grandchild"]);
        frames.record(
            &tab,
            "Page.frameDetached",
            &json!({"frameId":"child", "reason":"remove"}),
        );
        assert_eq!(ids(&list(&frames, &tab)?), ["main"]);
        Ok(())
    }

    /// A seed that lands after the events of a frame it reads merges with them: the frame takes
    /// the seed's document and keeps the parent its events gave, as an out-of-process frame's
    /// own tree names none for its root.
    #[test]
    fn tests_that_a_late_seed_merges_with_the_events_before_it() -> Result<(), Box<dyn Error>> {
        let mut frames = TabFrames::default();
        let tab = TargetId("main".to_owned());
        frames.seed(&tab, three_levels(), 0);
        frames.record(
            &tab,
            "Page.frameAttached",
            &json!({"frameId":"oopif", "parentFrameId":"child"}),
        );
        frames.seed(
            &tab,
            json!({"frameTree": {"frame": {"id":"oopif", "loaderId":"oop-doc",
                "url":"https://b.test/", "name":"remote"}}}),
            0,
        );
        let after = list(&frames, &tab)?;
        let oopif = after
            .frames
            .iter()
            .find(|frame| frame.id.0 == "oopif")
            .ok_or("the frame is listed")?;
        assert_eq!(
            (
                oopif.url.as_str(),
                oopif.name.as_str(),
                oopif.parent_id.as_ref().map(|parent| parent.0.as_str()),
            ),
            ("https://b.test/", "remote", Some("child"))
        );
        Ok(())
    }

    /// A list is whole once its tab's seed landed, as of the seq the seed names, and not
    /// before; forgetting the tab drops it.
    #[test]
    fn tests_that_a_list_is_whole_once_its_seed_landed() -> Result<(), Box<dyn Error>> {
        let mut frames = TabFrames::default();
        let tab = TargetId("main".to_owned());
        frames.record(
            &tab,
            "Page.frameAttached",
            &json!({"frameId":"child", "parentFrameId":"main"}),
        );
        assert!(
            !list(&frames, &tab)?.whole,
            "events alone make no whole list"
        );
        frames.seed(
            &tab,
            json!({"frameTree": {"frame": {"id":"child", "parentId":"main",
                "loaderId":"child-doc", "url":"https://b.test/child"}}}),
            3,
        );
        let subtree = list(&frames, &tab)?;
        assert!(
            !subtree.whole,
            "a frame session's own subtree is not the tab's tree"
        );
        assert_eq!(subtree.events, 3);
        frames.seed(&tab, three_levels(), 5);
        let seeded = list(&frames, &tab)?;
        assert!(seeded.whole);
        assert_eq!(seeded.events, 5);
        frames.forget(&tab);
        assert_eq!(frames.get(&tab), None);
        Ok(())
    }

    /// A list is whole once the tab's own tree landed and so did every frame session's own
    /// subtree: a frame session that becomes a frame's source holds it from whole until its
    /// subtree lands, its session ends with no other to take over, or its frame goes; the tab's
    /// forget drops the hold with the list.
    #[test]
    fn tests_that_a_frame_session_holds_its_list_from_whole_until_its_subtree_lands()
    -> Result<(), Box<dyn Error>> {
        let mut frames = TabFrames::default();
        let tab = TargetId("main".to_owned());
        frames.seed(&tab, three_levels(), 1);
        assert!(list(&frames, &tab)?.whole);
        assert_eq!(frames.missing_seeds(&tab), None);

        let oopif = FrameId("oopif".to_owned());
        frames.session_attached(&tab, oopif.clone());
        assert!(!list(&frames, &tab)?.whole, "its subtree has not landed");
        assert_eq!(
            frames.missing_seeds(&tab),
            Some(MissingSeeds {
                own_tree: false,
                frames: vec![oopif.clone()],
            })
        );
        frames.seed(
            &tab,
            json!({"frameTree": {"frame": {"id":"oopif", "parentId":"child",
                "loaderId":"oop-doc", "url":"https://b.test/"}}}),
            2,
        );
        assert!(list(&frames, &tab)?.whole, "its subtree landed");

        let ended = FrameId("ended".to_owned());
        frames.session_attached(&tab, ended.clone());
        assert!(!list(&frames, &tab)?.whole);
        frames.session_ended(&tab, &ended);
        assert!(list(&frames, &tab)?.whole, "no session reads it any more");

        frames.record(
            &tab,
            "Page.frameAttached",
            &json!({"frameId":"gone", "parentFrameId":"main"}),
        );
        frames.session_attached(&tab, FrameId("gone".to_owned()));
        assert!(!list(&frames, &tab)?.whole);
        frames.record(
            &tab,
            "Page.frameDetached",
            &json!({"frameId":"gone", "reason":"remove"}),
        );
        assert!(list(&frames, &tab)?.whole, "its frame went");

        frames.session_attached(&tab, FrameId("late".to_owned()));
        frames.forget(&tab);
        assert_eq!(
            frames.missing_seeds(&tab),
            Some(MissingSeeds {
                own_tree: true,
                frames: Vec::new(),
            }),
            "forgotten with the list: only its own tree is missing now"
        );
        frames.seed(&tab, three_levels(), 3);
        assert!(list(&frames, &tab)?.whole);
        Ok(())
    }

    /// The version moves with its writers and no others: a frame event that changes the list or
    /// commits, written or not, and a frame session's attach or end (`frames_changed`); a seed
    /// and a renderer swap do not. The list names the newest record it includes, which is the
    /// log's head after a change.
    #[test]
    fn tests_that_the_version_moves_with_its_writers_and_the_list_names_its_newest_record()
    -> Result<(), Box<dyn Error>> {
        let (hub, mut writer) = EventHub::new();
        let tab = TargetId("main".to_owned());
        writer.tab_id(tab.clone(), TabId(1));
        let now = |hub: &EventHub| -> Result<TabFrameList, Box<dyn Error>> {
            Ok(hub.tab_frames(&tab).ok_or("the tab has a list")?)
        };
        writer.seed_frames(&tab, three_levels());
        assert_eq!(now(&hub)?.version, 0, "a seed moves no version");

        writer.frame_event(
            &tab,
            "Page.frameAttached",
            &json!({"frameId":"late", "parentFrameId":"main"}),
            true,
        );
        let attached = now(&hub)?;
        assert!(attached.version > 0, "a written attach moves it");
        assert_eq!(
            attached.events,
            hub.head(),
            "the attach's record is included"
        );
        assert_eq!(hub.read_after(0).records.len(), 1);

        writer.frame_event(
            &tab,
            "Page.frameDetached",
            &json!({"frameId":"late", "reason":"swap"}),
            true,
        );
        assert_eq!(now(&hub)?.version, attached.version, "a swap moves nothing");

        writer.frame_event(
            &tab,
            "Page.frameNavigated",
            &json!({"frame":{"id":"child", "parentId":"main", "loaderId":"child-doc-2",
                "url":"https://a.test/child-2"}}),
            false,
        );
        let committed = now(&hub)?;
        assert!(
            committed.version > attached.version,
            "an unwritten commit moves it"
        );
        assert_eq!(hub.read_after(0).records.len(), 1, "and writes nothing");
        assert_eq!(committed.events, hub.head());

        writer.frames_changed(tab.clone());
        let session = now(&hub)?;
        assert!(
            session.version > committed.version,
            "a frame session's attach or end moves it"
        );
        assert_eq!(hub.frame_versions(), [(TabId(1), session.version)]);
        Ok(())
    }
}
