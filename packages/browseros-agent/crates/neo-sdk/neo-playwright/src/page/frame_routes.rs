//! Run-scoped endpoint hints for a tab's child frames: from a whole frame-tree read of an admitted
//! tab, or from a child's host resolved with no renderer read (`known_host`). Navigation can
//! dispatch on an already discovered frame without querying a renderer stalled by its pending
//! navigation, or by a hung sibling. Root attachment/connection identity and journaled ancestor
//! swaps retire stale hints. Each route keeps the log seq it was made at, so a route added later
//! is checked only against what happened after it was made.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use browseros_core::{FrameId, ProtocolSession, SessionId, TabId, TargetId};

use crate::events::{EventHub, PageEvent};

/// A discovered endpoint and the ancestor identities whose removal or swap retires it.
#[derive(Clone, Debug)]
pub(crate) struct FrameRoute {
    pub(crate) session: ProtocolSession,
    /// The frames from the main frame (left out) down to the frame itself.
    pub(crate) ancestors: Vec<FrameId>,
    /// The event-log seq the route was made at, taken at or before what it was made from: every
    /// record after it can retire it.
    pub(crate) since: u64,
}

/// A route belongs to this native page attachment on exactly one CDP connection epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RootAttachment {
    pub(crate) session: SessionId,
    pub(crate) frame: FrameId,
    pub(crate) epoch: u64,
}

#[derive(Debug)]
struct PageRoutes {
    root: RootAttachment,
    frames: HashMap<FrameId, FrameRoute>,
}

/// Hints are replaced as a complete admitted tree, forgotten on close, and dropped with a run.
#[derive(Debug, Default)]
pub(crate) struct FrameRoutes {
    pages: Mutex<HashMap<TabId, PageRoutes>>,
}

impl FrameRoutes {
    pub(crate) fn replace(
        &self,
        tab: TabId,
        root: RootAttachment,
        frames: HashMap<FrameId, FrameRoute>,
    ) {
        self.pages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(tab, PageRoutes { root, frames });
    }

    /// Adds one route beside `tab`'s others under the same root attachment; under another, it
    /// starts the tab's routes afresh, as the old attachment's no longer hold.
    pub(crate) fn insert(
        &self,
        tab: TabId,
        root: RootAttachment,
        frame: FrameId,
        route: FrameRoute,
    ) {
        let mut pages = self.pages.lock().unwrap_or_else(PoisonError::into_inner);
        match pages.get_mut(&tab) {
            Some(page) if page.root == root => {
                page.frames.insert(frame, route);
            }
            _ => {
                pages.insert(
                    tab,
                    PageRoutes {
                        root,
                        frames: HashMap::from([(frame, route)]),
                    },
                );
            }
        }
    }

    pub(crate) fn forget(&self, tab: TabId) {
        self.pages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&tab);
    }

    /// A hint never admits an unknown frame. Reject it if its owning attachment changed or
    /// if the bounded journal can no longer prove its ancestor chain remained in place.
    pub(crate) fn cached(
        &self,
        tab: TabId,
        root: &RootAttachment,
        frame: &FrameId,
        events: &EventHub,
    ) -> Option<ProtocolSession> {
        let route = {
            let pages = self.pages.lock().unwrap_or_else(PoisonError::into_inner);
            let page = pages.get(&tab).filter(|page| page.root == *root)?;
            page.frames.get(frame)?.clone()
        };
        (!retired(root, frame, &route, events)).then_some(route.session)
    }
}

/// Whether a record after `route`'s stamp, or a lost stretch of the rings it reads, retires it: an
/// ancestor (the frame itself included) detached or swapped to another renderer, or a new
/// document in the main frame or an ancestor. The frame's own same-session commit keeps its
/// endpoint.
pub(crate) fn retired(
    root: &RootAttachment,
    frame: &FrameId,
    route: &FrameRoute,
    events: &EventHub,
) -> bool {
    // The root's tab (a main frame has its tab's target id) plus the no-tab ring, where a frame
    // record the watcher could not place lands: either may retire this route, and only their
    // loss, never another tab's, leaves it unproven.
    let tab = TargetId(root.frame.as_str().to_owned());
    let read = events.read_tab_after(&tab, route.since);
    read.lost > 0
        || read.records.iter().any(|record| match &record.event {
            PageEvent::FrameDetached(id) | PageEvent::FrameSwapped(id) => {
                route.ancestors.contains(id)
            }
            // A new ancestor document retires its old children, including documents parked in
            // BFCache.
            PageEvent::Navigation(navigation)
                if navigation.new_document && navigation.frame != *frame =>
            {
                navigation.frame == root.frame || route.ancestors.contains(&navigation.frame)
            }
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use browseros_core::testing::TestConnection;
    use browseros_core::{FrameId, ProtocolSession, SessionId, TabId};

    use super::{FrameRoute, FrameRoutes, RootAttachment};
    use crate::events::{EventHub, PageEvent};

    #[test]
    fn endpoint_hints_require_the_same_root_epoch_and_no_ancestor_swap() {
        let routes = FrameRoutes::default();
        let root = RootAttachment {
            session: SessionId::from("root".to_owned()),
            frame: FrameId("main".to_owned()),
            epoch: 10,
        };
        let child = FrameId("child".to_owned());
        let parent = FrameId("parent".to_owned());
        let session = ProtocolSession::for_session(
            TestConnection::new([]),
            SessionId::from("oopif".to_owned()),
        );
        let (hub, mut writer) = EventHub::new();
        routes.replace(
            TabId(7),
            root.clone(),
            HashMap::from([(
                child.clone(),
                FrameRoute {
                    session,
                    ancestors: vec![parent.clone(), child.clone()],
                    since: 0,
                },
            )]),
        );
        assert!(routes.cached(TabId(7), &root, &child, &hub).is_some());
        assert!(
            routes
                .cached(
                    TabId(7),
                    &RootAttachment {
                        epoch: 11,
                        ..root.clone()
                    },
                    &child,
                    &hub
                )
                .is_none()
        );
        assert!(
            routes
                .cached(
                    TabId(7),
                    &RootAttachment {
                        frame: FrameId("replacement".to_owned()),
                        ..root.clone()
                    },
                    &child,
                    &hub
                )
                .is_none()
        );
        writer.record(PageEvent::FrameSwapped(parent));
        assert!(routes.cached(TabId(7), &root, &child, &hub).is_none());
        // An evicted invalidation must not resurrect a route once the journal wraps.
        for _ in 0..2048 {
            writer.record(PageEvent::FrameDetached(FrameId("other".to_owned())));
        }
        assert!(routes.cached(TabId(7), &root, &child, &hub).is_none());
        routes.forget(TabId(7));
        assert!(routes.cached(TabId(7), &root, &child, &hub).is_none());
    }

    /// Each route is checked against what happened after it was made: an ancestor's new document
    /// before a later route was made retires the older route only, and the later route, added
    /// beside it, keeps working.
    #[test]
    fn a_route_added_later_keeps_its_own_stamp() {
        let routes = FrameRoutes::default();
        let root = RootAttachment {
            session: SessionId::from("root".to_owned()),
            frame: FrameId("main".to_owned()),
            epoch: 10,
        };
        let session = |id: &str| {
            ProtocolSession::for_session(TestConnection::new([]), SessionId::from(id.to_owned()))
        };
        let (hub, mut writer) = EventHub::new();
        let (old, new) = (FrameId("old".to_owned()), FrameId("new".to_owned()));
        routes.insert(
            TabId(7),
            root.clone(),
            old.clone(),
            FrameRoute {
                session: session("old-session"),
                ancestors: vec![FrameId("parent".to_owned()), old.clone()],
                since: hub.head(),
            },
        );
        writer.record(PageEvent::FrameSwapped(FrameId("parent".to_owned())));
        routes.insert(
            TabId(7),
            root.clone(),
            new.clone(),
            FrameRoute {
                session: session("new-session"),
                ancestors: vec![FrameId("parent".to_owned()), new.clone()],
                since: hub.head(),
            },
        );
        assert!(
            routes.cached(TabId(7), &root, &old, &hub).is_none(),
            "the swap came after the older route was made"
        );
        assert!(
            routes.cached(TabId(7), &root, &new, &hub).is_some(),
            "the later route was made after it, beside the older one"
        );
    }
}
