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

    /// A snapshot may skip its adopted-field scan only when this already discovered tree
    /// proves this tab has only the frame being read. Different CDP sessions can still host
    /// mutually reachable same-origin frames, so session identity cannot justify skipping.
    /// Endpoint dispatch can survive new children and same-session commits; this stronger
    /// negative cannot. Never refresh here: unprovable topology keeps the scan enabled.
    pub(crate) fn may_have_other_frame(
        &self,
        tab: TabId,
        root: &RootAttachment,
        frame: &FrameId,
        session: &SessionId,
        events: &EventHub,
    ) -> bool {
        let known = {
            let pages = self.pages.lock().unwrap_or_else(PoisonError::into_inner);
            pages
                .get(&tab)
                .filter(|page| page.root == *root)
                .and_then(|page| Some((page.since, page.other_frame(frame, session)?)))
        };
        let Some((since, other_frame)) = known else {
            return true;
        };
        let read = events.read_after(since);
        // Some internal transitions name only a frame, without a tab. Rejecting changes on
        // other tabs too is conservative and avoids inventing another ownership registry.
        read.lost > 0
            || read.records.iter().any(|record| match &record.event {
                PageEvent::FrameLifecycle(_)
                | PageEvent::FrameDetached(_)
                | PageEvent::FrameSwapped(_)
                | PageEvent::Closed(_)
                | PageEvent::Crashed(_) => true,
                PageEvent::Navigation(navigation) => navigation.new_document,
                _ => false,
            })
            || other_frame
    }
}

impl PageRoutes {
    /// The stored paths exclude main and include their own frame. Validate every parent
    /// prefix before using absence: an incomplete or malformed tree cannot prove isolation.
    fn other_frame(&self, frame: &FrameId, session: &SessionId) -> Option<bool> {
        let main = self.frames.get(&self.root.frame)?;
        if !main.ancestors.is_empty() || main.session.session_id() != Some(&self.root.session) {
            return None;
        }
        let current = self.frames.get(frame)?;
        if current.session.session_id() != Some(session) {
            return None;
        }
        for (id, route) in &self.frames {
            route.session.session_id()?;
            if *id == self.root.frame {
                continue;
            }
            let (last, prefix) = route.ancestors.split_last()?;
            if last != id {
                return None;
            }
            let parent = prefix.last().unwrap_or(&self.root.frame);
            if self.frames.get(parent)?.ancestors.as_slice() != prefix {
                return None;
            }
        }
        Some(self.frames.keys().any(|id| id != frame))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::error::Error;

    use browseros_core::testing::TestConnection;
    use browseros_core::{FrameId, ProtocolSession, SessionId, TabId, TargetId};

    use super::{FrameRoute, FrameRoutes, RootAttachment};
    use crate::events::{EventHub, FrameLifecycle, NavigationEvent, PageEvent, PageEventKind};

    fn attachment() -> RootAttachment {
        RootAttachment {
            session: SessionId::from("root"),
            frame: FrameId("main".to_owned()),
            epoch: 10,
        }
    }

    fn tree(entries: &[(&str, &str, &[&str])]) -> FrameRoutes {
        let routes = FrameRoutes::default();
        let frames = entries
            .iter()
            .map(|(frame, session, ancestors)| {
                (
                    FrameId((*frame).to_owned()),
                    FrameRoute {
                        session: ProtocolSession::for_session(
                            TestConnection::new([]),
                            SessionId::from(*session),
                        ),
                        ancestors: ancestors
                            .iter()
                            .map(|id| FrameId((*id).to_owned()))
                            .collect(),
                    },
                )
            })
            .collect();
        routes.replace(TabId(7), attachment(), 0, frames);
        routes
    }

    fn hint(routes: &FrameRoutes, hub: &EventHub, frame: &str, session: &str) -> bool {
        routes.may_have_other_frame(
            TabId(7),
            &attachment(),
            &FrameId(frame.to_owned()),
            &SessionId::from(session),
            hub,
        )
    }

    #[test]
    fn a_known_single_frame_skips_the_scan() {
        let (hub, _writer) = EventHub::new();
        let single = tree(&[("main", "root", &[])]);
        assert!(!hint(&single, &hub, "main", "root"));
    }

    #[test]
    fn one_cross_site_frame_keeps_the_scan_for_both_frames() {
        let (hub, _writer) = EventHub::new();
        let remote = tree(&[("main", "root", &[]), ("ad", "oopif", &["ad"])]);
        assert!(hint(&remote, &hub, "main", "root"));
        assert!(hint(&remote, &hub, "ad", "oopif"));
    }

    #[test]
    fn nested_and_shadow_frame_paths_keep_the_scan() {
        let (hub, _writer) = EventHub::new();
        let direct = tree(&[
            ("main", "root", &[]),
            ("shadow-child", "root", &["shadow-child"]),
        ]);
        assert!(hint(&direct, &hub, "main", "root"));
        assert!(hint(&direct, &hub, "shadow-child", "root"));
        let routes = tree(&[
            ("main", "root", &[]),
            ("outer", "oopif", &["outer"]),
            ("shadow-child", "oopif", &["outer", "shadow-child"]),
        ]);
        assert!(hint(&routes, &hub, "main", "root"));
        assert!(hint(&routes, &hub, "outer", "oopif"));
        assert!(
            hint(&routes, &hub, "shadow-child", "oopif"),
            "DOM placement is irrelevant to cached native ancestry"
        );
    }

    #[test]
    fn sibling_frames_below_a_cross_site_intermediary_keep_the_scan_for_every_frame() {
        let (hub, _writer) = EventHub::new();
        let routes = tree(&[
            ("main", "root", &[]),
            ("cross-site", "oopif", &["cross-site"]),
            ("left", "shared", &["cross-site", "left"]),
            ("right", "shared", &["cross-site", "right"]),
        ]);
        assert!(hint(&routes, &hub, "main", "root"));
        assert!(hint(&routes, &hub, "cross-site", "oopif"));
        assert!(hint(&routes, &hub, "left", "shared"));
        assert!(hint(&routes, &hub, "right", "shared"));
    }

    #[test]
    fn cousin_frames_across_cross_site_intermediaries_keep_the_scan_for_every_frame() {
        let (hub, _writer) = EventHub::new();
        let routes = tree(&[
            ("main", "root", &[]),
            ("left-parent", "left-oopif", &["left-parent"]),
            ("right-parent", "right-oopif", &["right-parent"]),
            ("left", "shared", &["left-parent", "left"]),
            ("right", "shared", &["right-parent", "right"]),
        ]);
        assert!(hint(&routes, &hub, "main", "root"));
        assert!(hint(&routes, &hub, "left-parent", "left-oopif"));
        assert!(hint(&routes, &hub, "right-parent", "right-oopif"));
        assert!(hint(&routes, &hub, "left", "shared"));
        assert!(hint(&routes, &hub, "right", "shared"));
    }

    #[test]
    fn unknown_frames_attachments_sessions_and_invalid_ancestry_keep_the_scan()
    -> Result<(), Box<dyn Error>> {
        let (hub, _writer) = EventHub::new();
        let routes = tree(&[("main", "root", &[])]);
        assert!(hint(&FrameRoutes::default(), &hub, "main", "root"));
        assert!(hint(&routes, &hub, "unseen-child", "root"));
        assert!(hint(&routes, &hub, "main", "replacement-session"));
        for root in [
            RootAttachment {
                epoch: 11,
                ..attachment()
            },
            RootAttachment {
                session: SessionId::from("replacement"),
                ..attachment()
            },
            RootAttachment {
                frame: FrameId("replacement".to_owned()),
                ..attachment()
            },
        ] {
            assert!(routes.may_have_other_frame(
                TabId(7),
                &root,
                &FrameId("main".to_owned()),
                &SessionId::from("root"),
                &hub,
            ));
        }
        for entries in [
            vec![("child", "root", vec!["child"])],
            vec![("main", "root", vec!["main"])],
            vec![("main", "wrong-root-session", vec![])],
            vec![("main", "root", vec![]), ("child", "other", vec![])],
            vec![
                ("main", "root", vec![]),
                ("child", "other", vec!["wrong-id"]),
            ],
            vec![
                ("main", "root", vec![]),
                ("child", "other", vec!["missing-parent", "child"]),
            ],
            vec![
                ("main", "root", vec![]),
                ("parent", "other", vec!["parent"]),
                ("child", "other", vec!["main", "parent", "child"]),
            ],
        ] {
            let borrowed: Vec<_> = entries
                .iter()
                .map(|(frame, session, path)| (*frame, *session, path.as_slice()))
                .collect();
            let malformed = tree(&borrowed);
            assert!(hint(&malformed, &hub, "main", "root"), "{entries:?}");
            // A malformed multi-frame tree also keeps the scan. Verify it remains unknown,
            // so the conservative fallback does not hide a missing admission check.
            let pages = malformed
                .pages
                .lock()
                .map_err(|_poisoned| "test cache poisoned")?;
            let page = pages.get(&TabId(7)).ok_or("test page missing")?;
            assert_eq!(
                page.other_frame(&FrameId("main".to_owned()), &SessionId::from("root")),
                None,
                "{entries:?}"
            );
        }
        let missing_session = tree(&[("main", "root", &[]), ("child", "other", &["child"])]);
        let mut pages = missing_session
            .pages
            .lock()
            .map_err(|_poisoned| "test cache poisoned")?;
        let page = pages.get_mut(&TabId(7)).ok_or("test page missing")?;
        let child = page
            .frames
            .get_mut(&FrameId("child".to_owned()))
            .ok_or("test child missing")?;
        child.session = ProtocolSession::root(TestConnection::new([]));
        assert_eq!(
            page.other_frame(&FrameId("main".to_owned()), &SessionId::from("root")),
            None,
            "a route without a session is unknown"
        );
        drop(pages);
        assert!(hint(&missing_session, &hub, "main", "root"));
        Ok(())
    }

    #[test]
    fn every_topology_or_document_transition_invalidates_a_cached_negative()
    -> Result<(), Box<dyn Error>> {
        let child = FrameId("new-child".to_owned());
        let navigation = NavigationEvent {
            frame: FrameId("main".to_owned()),
            url: "https://next.test/".to_owned(),
            new_document: true,
            loader: None,
            unreachable: None,
        };
        let attached_frame = serde_json::from_value(serde_json::json!({
            "id": "new-child", "parentId": "main",
        }))?;
        for event in [
            PageEvent::FrameLifecycle(FrameLifecycle {
                target: TargetId("main".to_owned()),
                kind: PageEventKind::FrameAttached,
                frame: attached_frame,
            }),
            PageEvent::FrameDetached(child.clone()),
            PageEvent::FrameSwapped(child),
            PageEvent::Navigation(navigation.clone()),
            PageEvent::Closed(TargetId("main".to_owned())),
            PageEvent::Crashed(TargetId("main".to_owned())),
        ] {
            let routes = tree(&[("main", "root", &[])]);
            let (hub, mut writer) = EventHub::new();
            assert!(!hint(&routes, &hub, "main", "root"));
            writer.record(event.clone());
            assert!(hint(&routes, &hub, "main", "root"), "{event:?}");
        }
        let routes = tree(&[("main", "root", &[])]);
        let (hub, mut writer) = EventHub::new();
        writer.record(PageEvent::Navigation(NavigationEvent {
            new_document: false,
            ..navigation
        }));
        assert!(
            !hint(&routes, &hub, "main", "root"),
            "history changes retain topology"
        );
        for _ in 0..2048 {
            writer.record(PageEvent::Navigation(NavigationEvent {
                frame: FrameId("main".to_owned()),
                url: String::new(),
                new_document: false,
                loader: None,
                unreachable: None,
            }));
        }
        assert!(
            hint(&routes, &hub, "main", "root"),
            "journal loss cannot prove absence"
        );
        Ok(())
    }

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
