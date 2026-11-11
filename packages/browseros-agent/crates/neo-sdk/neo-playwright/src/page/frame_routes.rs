//! Run-scoped endpoint hints from admitted frame-tree discovery. Navigation can dispatch on
//! an already discovered frame without querying a renderer stalled by its pending navigation.
//! Root attachment/connection identity and journaled ancestor swaps retire stale hints.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use browseros_core::{FrameId, ProtocolSession, SessionId, TabId};

use crate::events::{EventHub, PageEvent};

/// A discovered endpoint and the ancestor identities whose removal or swap retires it.
#[derive(Clone, Debug)]
pub(crate) struct FrameRoute {
    pub(crate) session: ProtocolSession,
    pub(crate) ancestors: Vec<FrameId>,
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
    since: u64,
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
        since: u64,
        frames: HashMap<FrameId, FrameRoute>,
    ) {
        self.pages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                tab,
                PageRoutes {
                    root,
                    since,
                    frames,
                },
            );
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
        let (since, route) = {
            let pages = self.pages.lock().unwrap_or_else(PoisonError::into_inner);
            let page = pages.get(&tab).filter(|page| page.root == *root)?;
            (page.since, page.frames.get(frame)?.clone())
        };
        let read = events.read_after(since);
        if read.lost > 0
            || read.records.iter().any(|record| match &record.event {
                PageEvent::FrameDetached(id) | PageEvent::FrameSwapped(id) => {
                    route.ancestors.contains(id)
                }
                // A new ancestor document retires its old children, including documents
                // parked in BFCache. The frame's own same-session commit keeps its endpoint.
                PageEvent::Navigation(navigation)
                    if navigation.new_document && navigation.frame != *frame =>
                {
                    navigation.frame == root.frame || route.ancestors.contains(&navigation.frame)
                }
                _ => false,
            })
        {
            return None;
        }
        Some(route.session)
    }
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
            0,
            HashMap::from([(
                child.clone(),
                FrameRoute {
                    session,
                    ancestors: vec![parent.clone(), child.clone()],
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
}
