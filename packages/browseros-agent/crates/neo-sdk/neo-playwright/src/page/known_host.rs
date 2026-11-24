//! A route-less child frame's host, resolved with no renderer read: from the child's chain in the
//! target watcher's list (`TabFrames`), the browser's out-of-process iframe targets, and core's
//! frame registry. `page.frames` answers from the list and fills no route, so a child-frame op
//! (`navigation::frame_host`) often finds none; reading the whole tree instead would read every
//! cross-site frame's renderer, and one hung sibling would hold the op to its timeout.
//!
//! ```text
//!  synchronize on [tab]   the watcher applied core's attach, so the tab's frame changes
//!         │               from now on write records (`records_page_events`)
//!  frames_at(tab)         the log's head (the route's stamp) and the list, under one lock
//!         │               the chain main ─▶ … ─▶ child, every link present, or no route
//!  Target.getTargets      the browser's current split: the nearest frame on the chain, the
//!         │               child first, that is its own target hosts it; none: the root session
//!  core's registry        that frame's session (attached on demand, a command to it alone)
//!         │
//!  synchronize, cached()'s retirement check against the stamp, then the route is used
//! ```
//!
//! Each step that cannot vouch for the route gives `None`, and the op falls back to the whole
//! read on its own tab. A list that is not whole is no reason to: its frames are all real and a
//! frame never changes parent, so a complete chain is the real chain.

use std::collections::HashSet;

use browseros_core::frames::FrameTarget;
use browseros_core::{FrameId, ProtocolSession, TargetId};
use futures_util::future::BoxFuture;

use crate::events::tab_frames::TabFrameList;
use crate::page::frame_routes::FrameRoute;

/// What resolving a child's host reads, behind a seam so its order can be tested: the watcher's
/// progress, its list, the browser's iframe targets and core's frame registry.
pub(crate) trait RouteSource: Send + Sync {
    /// Waits until the watcher applied every event of `tab` received until now; whether it did
    /// in time.
    fn caught_up<'a>(&'a self, tab: &'a TargetId) -> BoxFuture<'a, bool>;

    /// The log's head and `tab`'s list, taken together.
    fn frames_at(&self, tab: &TargetId) -> (u64, Option<TabFrameList>);

    /// The target ids of the browser's out-of-process frames; `None` when the browser did not
    /// answer.
    fn iframe_targets(&self) -> BoxFuture<'_, Option<HashSet<String>>>;

    /// Core's session for out-of-process frame `frame`, attaching it when core holds none.
    fn frame_session<'a>(&'a self, frame: &'a FrameId) -> BoxFuture<'a, Option<ProtocolSession>>;
}

/// `frame`'s route on `tab`, whose root session is `root`, or `None` when it cannot be vouched
/// for. `retired` is the route cache's own check (`frame_routes::retired`), run before the route
/// is first used.
pub(crate) async fn known_host(
    source: &dyn RouteSource,
    tab: &TargetId,
    root: &ProtocolSession,
    frame: &FrameId,
    retired: impl Fn(&FrameRoute) -> bool,
) -> Option<FrameRoute> {
    // Before the stamp: a frame removed, swapped or navigated before the watcher applied core's
    // attach moves the list and writes no record, and a route stamped before it would never
    // retire.
    if !source.caught_up(tab).await {
        return None;
    }
    let (since, list) = source.frames_at(tab);
    let ancestors = chain(&list?, tab, frame)?;
    let targets = source.iframe_targets().await?;
    let session = match host_frame(&ancestors, &targets) {
        Some(host) => source.frame_session(host).await?,
        None => root.clone(),
    };
    let route = FrameRoute {
        session,
        ancestors,
        since,
    };
    // A swap, removal or new document on the chain since the stamp, already sent, retires the
    // route before its first use rather than after.
    if !source.caught_up(tab).await || retired(&route) {
        return None;
    }
    Some(route)
}

/// The frames from `tab`'s main frame (left out) down to `frame` (included), when every one of
/// them is in `list` with its parent; `None` when a link is missing. The main frame is the one
/// whose id is the tab's target id.
fn chain(list: &TabFrameList, tab: &TargetId, frame: &FrameId) -> Option<Vec<FrameId>> {
    if frame.0 == tab.0 {
        return None;
    }
    let mut path = Vec::new();
    let mut at = frame.clone();
    // Bounded by the list's size, so a parent loop cannot spin.
    for _ in 0..list.frames.len() {
        let known = list.frames.iter().find(|known| known.id == at)?;
        path.push(at.clone());
        let parent = known.parent_id.clone()?;
        if parent.0 == tab.0 {
            path.reverse();
            return Some(path);
        }
        at = parent;
    }
    None
}

/// The session core's registry answered for an out-of-process frame, when it is that frame's own:
/// one it held or just attached. When the attach failed (the target went between the browser's
/// list and the attach, removed or swapped back into its parent's process), core falls back to
/// sharing a parent's session, the page's here, which hosts the frame only if it moved back into
/// the main frame's process; the op then falls back to the read rather than trust it.
///
/// It reads core's `FrameTarget::cursor_uses_session_default` as "core returned the frame's own
/// session". In `resolve_child_target` (browseros-core `frames.rs`) that is true only for the
/// out-of-process cache hit, a fresh `attach_oopif_on_demand` and the cache re-check after it,
/// and false for the shared-parent fallback. A change to when core sets it changes this check.
pub(crate) fn own_session(target: FrameTarget) -> Option<ProtocolSession> {
    target.cursor_uses_session_default.then_some(target.session)
}

/// The nearest frame of `chain`, the child first, that the browser lists as an out-of-process
/// target: its session hosts the child. `None`: the whole chain shares the main frame's process.
fn host_frame<'a>(chain: &'a [FrameId], targets: &HashSet<String>) -> Option<&'a FrameId> {
    chain.iter().rev().find(|frame| targets.contains(&frame.0))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::{Mutex, PoisonError};

    use browseros_core::frames::FrameTarget;
    use browseros_core::testing::TestConnection;
    use browseros_core::{FrameId, ProtocolSession, SessionId, TargetId};
    use futures_util::future::BoxFuture;

    use super::{RouteSource, chain, host_frame, known_host, own_session};
    use crate::events::tab_frames::{ListedFrame, TabFrameList};

    fn id(name: &str) -> FrameId {
        FrameId(name.to_owned())
    }

    fn session(name: &str) -> ProtocolSession {
        ProtocolSession::for_session(TestConnection::new([]), SessionId::from(name.to_owned()))
    }

    /// Main `a`, a same-site child `c1` in its process, `c1`'s cross-site child `g` (its own
    /// target) and `g`'s same-process child `g2`, and `c1`'s same-site child `s`.
    fn list(whole: bool, without: Option<&str>) -> TabFrameList {
        let frame = |id: &str, parent: Option<&str>| ListedFrame {
            id: FrameId(id.to_owned()),
            parent_id: parent.map(|parent| FrameId(parent.to_owned())),
            name: String::new(),
            url: format!("https://{id}.test/"),
            loader_id: None,
        };
        let frames = [
            frame("a", None),
            frame("c1", Some("a")),
            frame("g", Some("c1")),
            frame("g2", Some("g")),
            frame("s", Some("c1")),
        ]
        .into_iter()
        .filter(|frame| Some(frame.id.0.as_str()) != without)
        .collect();
        TabFrameList {
            frames,
            version: 1,
            whole,
            events: 0,
        }
    }

    /// The host follows the browser's current split down the chain: a cross-site child of a
    /// same-site child is its own host, its same-process child is hosted by it, and a same-site
    /// grandchild is hosted by the main frame's session. A missing link builds no chain, and a
    /// list that is not whole builds one all the same.
    #[test]
    fn tests_that_a_childs_host_is_its_nearest_out_of_process_frame() {
        let tab = TargetId("a".to_owned());
        let targets: HashSet<String> = HashSet::from(["g".to_owned()]);
        let whole = list(true, None);
        let host = |child: &str| {
            let chain = chain(&whole, &tab, &id(child)).unwrap_or_default();
            host_frame(&chain, &targets).cloned()
        };
        assert_eq!(
            chain(&whole, &tab, &id("g2")),
            Some(vec![id("c1"), id("g"), id("g2")])
        );
        assert_eq!(host("g"), Some(id("g")), "its own target");
        assert_eq!(host("g2"), Some(id("g")), "its out-of-process parent");
        assert_eq!(host("s"), None, "the main frame's process");
        assert_eq!(
            chain(&list(true, Some("c1")), &tab, &id("g2")),
            None,
            "a missing link"
        );
        assert_eq!(
            chain(&whole, &tab, &id("a")),
            None,
            "the main frame is no child"
        );
        assert!(
            chain(&list(false, None), &tab, &id("g2")).is_some(),
            "a list that is not whole still holds the real chain"
        );
    }

    /// Core's answer counts as an out-of-process frame's host only when it is that frame's own
    /// session; a failed attach's fallback to a parent's session builds no route.
    #[test]
    fn tests_that_only_a_frames_own_session_hosts_it() {
        let own = FrameTarget {
            session: session("g"),
            ax_params: serde_json::json!({}),
            cursor_uses_session_default: true,
        };
        let shared = FrameTarget {
            session: session("page"),
            ax_params: serde_json::json!({ "frameId": "g" }),
            cursor_uses_session_default: false,
        };
        assert_eq!(
            own_session(own).and_then(|session| session.session_id().cloned()),
            Some(SessionId::from("g".to_owned()))
        );
        assert!(
            own_session(shared).is_none(),
            "the page's session, after a failed attach"
        );
    }

    /// A tab whose frame changes the watcher applies only once it caught up: the list held
    /// `gone` until then, and its removal wrote no record (core's attach not applied yet, so
    /// `records_page_events` was off).
    struct Source {
        list: Mutex<TabFrameList>,
        caught_up: Mutex<bool>,
    }

    impl RouteSource for Source {
        fn caught_up<'a>(&'a self, _tab: &'a TargetId) -> BoxFuture<'a, bool> {
            Box::pin(async move {
                let mut caught_up = self
                    .caught_up
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if !*caught_up {
                    *caught_up = true;
                    let mut list = self.list.lock().unwrap_or_else(PoisonError::into_inner);
                    list.frames.retain(|frame| frame.id.0 != "g2");
                }
                true
            })
        }
        fn frames_at(&self, _tab: &TargetId) -> (u64, Option<TabFrameList>) {
            let list = self
                .list
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            (5, Some(list))
        }
        fn iframe_targets(&self) -> BoxFuture<'_, Option<HashSet<String>>> {
            Box::pin(async { Some(HashSet::from(["g".to_owned()])) })
        }
        fn frame_session<'a>(
            &'a self,
            frame: &'a FrameId,
        ) -> BoxFuture<'a, Option<ProtocolSession>> {
            Box::pin(async move { Some(session(&frame.0)) })
        }
    }

    /// A frame removed between core's attach and the watcher applying it writes no record, so a
    /// route stamped before the watcher caught up would never retire. The resolution catches up
    /// first: the list then lacks the frame, and no route is built.
    #[tokio::test]
    async fn tests_that_a_frame_gone_before_the_watcher_caught_up_builds_no_route() {
        let source = Source {
            list: Mutex::new(list(true, None)),
            caught_up: Mutex::new(false),
        };
        let tab = TargetId("a".to_owned());
        let route = known_host(&source, &tab, &session("root"), &id("g2"), |_| false).await;
        assert!(route.is_none(), "g2 left before the stamp: {route:?}");
    }

    /// A route is checked against the retirement records before its first use, and one the
    /// check retires is not used; one it keeps carries the stamp the list was taken with.
    #[tokio::test]
    async fn tests_that_a_route_is_checked_before_its_first_use() {
        let source = Source {
            list: Mutex::new(list(true, None)),
            caught_up: Mutex::new(true),
        };
        let tab = TargetId("a".to_owned());
        let root = session("root");
        assert!(
            known_host(&source, &tab, &root, &id("g2"), |_| true)
                .await
                .is_none(),
            "retired on the chain since the stamp"
        );
        let short = Source {
            list: Mutex::new(list(false, None)),
            caught_up: Mutex::new(true),
        };
        assert!(
            known_host(&short, &tab, &root, &id("g2"), |_| false)
                .await
                .is_some(),
            "a list that is not whole still holds g2's real chain"
        );
        let route = known_host(&source, &tab, &root, &id("g2"), |_| false).await;
        let route = route.as_ref();
        assert_eq!(route.map(|route| route.since), Some(5));
        assert_eq!(
            route.and_then(|route| route.session.session_id().cloned()),
            Some(SessionId::from("g".to_owned()))
        );
    }
}
