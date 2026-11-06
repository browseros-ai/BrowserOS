//! The shadow roots of a frame document that the seal cannot find by itself, found by a pierced
//! scan of the DOM so that the seal watches them (sdk-privacy D4, closed shadow roots).
//!
//! The seal sweeps fields from `document` down through open shadow roots, and journals input
//! from listeners on the document. A closed root is invisible from outside it: its host's
//! `shadowRoot` is null, and an `input` that starts inside it reaches the document retargeted to
//! the host, or not at all when the page dispatches it (a page's own events are not composed).
//! So Neo finds those roots with `DOM.describeNode { pierce }`, which lists closed roots, and
//! hands each to `__clawSeal.watch(root)`. From then on the seal sweeps and journals that root
//! as it does the document.
//!
//! ```text
//!  SealedWorld::install, and every sweep before it reads (evaluate_guard::sweep_world)
//!    └─ SealedWorld::watch_hidden_roots(Scan::Document)    under the WatchedRoots lock
//!         ├─ HiddenRoots::scan   DOM.describeNode { the world's document, depth 30, pierce },
//!         │                      then again from each node the depth cut off, until none is
//!         ├─ each host the seal reported: __clawSeal.takeHost(), then HiddenRoots::scan of it
//!         ├─ each tree the seal watches out of the document (an element a main-world script
//!         │  was handed, then removed): __clawSeal.detachedTops(), then HiddenRoots::scan of it
//!         └─ the roots this world does not watch yet
//!              └─ SealedWorld::watch_roots   DOM.resolveNode each into the world, then one
//!                                            Runtime.callFunctionOn: __clawSeal.watch(root)
//!  the journal hears HIDDEN_ROOT_REPORT (evaluate_guard::sweep_reported)
//!    └─ SealedWorld::watch_hidden_roots(Scan::Reported)    the reported hosts only
//!         └─ then the world's sweep, whose values go to the host (values_swept)
//! ```
//!
//! **Hidden** is a closed root, or any root inside one: the seal's own walk to open roots
//! starts at the document, so it never enters a closed root. Never hidden: user-agent roots (an
//! input's internals, whose text is the input's raw value, so the walk never enters one), and
//! the documents of child frames, which have worlds and scans of their own.
//!
//! **A scan before every sweep, because nothing cheaper sees every new root** (measured on
//! BrowserOS 155):
//! - in the page, `attachShadow` on an element already in the document, or a custom element
//!   upgraded in place, makes no mutation record;
//! - CDP's `DOM.shadowRootPushed` reports a root only on a host the session's DOM agent has
//!   already sent: of 5 roots a page attached after a pierced `DOM.getDocument`, it reported 1,
//!   and `DOM.childNodeInserted` carried 1 more. That agent's bindings also belong to every
//!   other `getDocument` on the session, core's and takeover's included, and one of those with
//!   less depth would silence it;
//! - `DOMSnapshot.captureSnapshot` costs the same and lists no shadow root nodes at all.
//!
//! **And a report for the roots no scan can find.** A page can attach a closed root, have the
//! user type into it and remove its host again between two sweeps. The seal hears the typed
//! `input` reach the document retargeted to the host, holds the host and reports it through the
//! journal binding (`HIDDEN_ROOT_REPORT`). Rust takes the host, scans it, watches its root and
//! sweeps the world while the field still exists in the removed tree. A value the page sets and
//! removes with no composed event before any sweep leaves no trace, and stays a residual. The
//! takes and watches of one document run one at a time, under [`WatchedRoots`]'s lock, so a
//! sweep that waited for the lock reads every root whose host another watch had taken.
//!
//! `describeNode` binds nothing in the DOM agent, so it disturbs no one. A scan costs about
//! 3.9 µs per element of the document, almost all of it Chromium's walk (a release build of
//! this crate, `closed_roots_live`'s cost test): a read of a small one-frame page takes 0.2 ms
//! more, and one of a page of 1000 elements 4 ms more, for each frame of the read's site.
//!
//! **Depth 30 per describe.** The CDP client parses a message with `serde_json`'s limit of 128
//! nested levels and drops one it cannot parse, so a pierced describe of a deeper document would
//! never answer. One level of depth adds at most four levels of JSON (a host, its
//! `shadowRoots`, the root, the root's `children`), so 30 stays under the limit. A node the
//! depth cut off, one that has children but came without them, is described again on its own.

use std::collections::HashSet;
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};

use browseros_cdp::CdpError;
use browseros_core::{CoreError, ProtocolSession};
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use tokio::sync::{Mutex, MutexGuard};

use crate::probe::ProbeFailure;
use crate::world::BackendNodeId;

/// How many levels of the DOM one describe takes; the module doc says why 30.
const DEPTH: u32 = 30;

/// How many describes of cut-off nodes are in flight at once.
const IN_FLIGHT: usize = 16;

/// How `DOM.describeNode` refuses a backend id whose node is gone (measured).
const NODE_GONE: &str = "No node found for given backend id";

/// Every hidden root one scan found in a frame document, by its DOM node.
#[derive(Debug, Default)]
pub(crate) struct HiddenRoots(Vec<BackendNodeId>);

/// One install's document, where its scans start, and the hidden roots its seal watches. Shared
/// by every clone of the install, so each root is resolved and watched once while it stays in
/// the document. A backend id names one node for as long as the renderer lives and is never
/// reused. A scan of the whole document forgets the roots it no longer finds, as the seal,
/// which holds its roots weakly, lets a dropped one die; one the page attaches again is found
/// and handed to the seal again, which still watches it.
#[derive(Debug)]
pub(crate) struct WatchedRoots {
    /// The world's `document`, by handle in the world. A handle, not a backend id: a backend id
    /// is a per-renderer counter that may name another document's node after the frame changes
    /// process.
    document: String,
    /// Held by a watch from its first take of a reported host to the seal's confirmation. The
    /// seal hands a host out once, so a sweep must not read while another watch holds a host
    /// whose root it has not watched yet.
    watched: Mutex<HashSet<BackendNodeId>>,
    /// A journal report of this document waits for its sweep. Later reports join it, so a page
    /// that floods the journal starts one sweep at a time, not one per report.
    report_queued: AtomicBool,
}

/// A node a describe cut off, to be described again from itself.
struct Cut {
    node: BackendNodeId,
    /// A closed root lies above it, so every root under it is hidden.
    under_closed: bool,
}

/// A cut-off node described again on its own. The describes run out of order, so each answer
/// carries the ancestry its node was cut with: whether the roots under it are hidden.
struct Redescribed {
    under_closed: bool,
    /// The node's subtree, `None` when the node left the document since it was cut.
    node: Result<Option<Value>, ProbeFailure>,
}

/// What a scan has found so far.
#[derive(Default)]
struct Walk {
    roots: Vec<BackendNodeId>,
    cut: Vec<Cut>,
}

impl HiddenRoots {
    /// Scans the subtree of the node `start` names (a handle in the world, on `session`) for its
    /// hidden roots, however deep they are: the world's document, or a host the seal reported,
    /// whose closed root is hidden like any other.
    ///
    /// # Errors
    /// `Gone` when the node's world is gone. A cut-off node that left the document since its
    /// describe is skipped, with every root under it.
    pub(crate) async fn scan(session: &ProtocolSession, start: &str) -> Result<Self, ProbeFailure> {
        let mut walk = Walk::default();
        let top = describe(session, json!({ "objectId": start }))
            .await?
            .ok_or(ProbeFailure::Gone)?;
        walk.subtree(&top, false)?;
        while !walk.cut.is_empty() {
            let described: Vec<Redescribed> = stream::iter(mem::take(&mut walk.cut))
                .map(|cut| async move {
                    Redescribed {
                        under_closed: cut.under_closed,
                        node: describe(session, json!({ "backendNodeId": cut.node.0 })).await,
                    }
                })
                .buffer_unordered(IN_FLIGHT)
                .collect()
                .await;
            for Redescribed { under_closed, node } in described {
                if let Some(node) = node? {
                    walk.subtree(&node, under_closed)?;
                }
            }
        }
        Ok(Self(walk.roots))
    }

    /// These roots and `other`'s.
    pub(crate) fn extend(&mut self, other: Self) {
        self.0.extend(other.0);
    }
}

impl WatchedRoots {
    /// No root watched yet in the document `document` names.
    pub(crate) fn new(document: String) -> Self {
        Self {
            document,
            watched: Mutex::default(),
            report_queued: AtomicBool::new(false),
        }
    }

    /// The document's handle, for the scan.
    pub(crate) fn document(&self) -> &str {
        &self.document
    }

    /// The roots the seal watches, for one watch at a time. Record a root in it only once the
    /// seal confirmed it: a sweep that finds a root here must have been sent after the watch.
    pub(crate) async fn lock(&self) -> MutexGuard<'_, HashSet<BackendNodeId>> {
        self.watched.lock().await
    }

    /// Queues a journal report's sweep; `false` when one is queued already and will take this
    /// report's host with its own.
    pub(crate) fn queue_report(&self) -> bool {
        !self.report_queued.swap(true, Ordering::AcqRel)
    }

    /// No report's sweep waits any more: a watch is about to take the reported hosts, or the
    /// queued sweep gave up before it could. A report from now on needs a sweep of its own.
    pub(crate) fn unqueue_report(&self) {
        self.report_queued.store(false, Ordering::Release);
    }
}

/// Forgets every root of `watched` that `found`, a scan of the whole document and of the hosts
/// the seal reported, did not find: the page removed it, so it is no longer swept by id.
pub(crate) fn keep_found(watched: &mut HashSet<BackendNodeId>, found: &HiddenRoots) {
    let found: HashSet<_> = found.0.iter().collect();
    watched.retain(|root| found.contains(root));
}

/// The roots of `found` that `watched` does not hold, each once.
pub(crate) fn unwatched(
    watched: &HashSet<BackendNodeId>,
    found: &HiddenRoots,
) -> Vec<BackendNodeId> {
    let mut fresh = HashSet::new();
    found
        .0
        .iter()
        .filter(|root| !watched.contains(root) && fresh.insert(**root))
        .copied()
        .collect()
}

impl Walk {
    /// Walks one described subtree, `top` under a closed root when `under_closed` says so. A
    /// loop rather than recursion: the page decides how deep a DOM is.
    fn subtree(&mut self, top: &Value, under_closed: bool) -> Result<(), ProbeFailure> {
        let mut pending = vec![(top, under_closed)];
        while let Some((node, under_closed)) = pending.pop() {
            let has_children = node
                .get("childNodeCount")
                .and_then(Value::as_u64)
                .is_some_and(|count| count > 0);
            if node.get("children").is_none() && has_children {
                // Described again whole, its shadow roots with it.
                self.cut.push(Cut {
                    node: backend_id(node)?,
                    under_closed,
                });
                continue;
            }
            for root in array(node, "shadowRoots") {
                let closed = match root.get("shadowRootType").and_then(Value::as_str) {
                    Some("closed") => true,
                    Some("open") => false,
                    // An input's own internals, whose text is its raw value.
                    _ => continue,
                };
                let hidden = under_closed || closed;
                if hidden {
                    self.roots.push(backend_id(root)?);
                }
                pending.push((root, hidden));
            }
            // A frame owner's `contentDocument` is another frame's document, and never walked.
            pending.extend(array(node, "children").map(|child| (child, under_closed)));
        }
        Ok(())
    }
}

/// One pierced describe from the node `start` names; `None` when that node is gone.
async fn describe(session: &ProtocolSession, start: Value) -> Result<Option<Value>, ProbeFailure> {
    let mut params = start;
    if let Some(fields) = params.as_object_mut() {
        fields.insert("depth".to_owned(), json!(DEPTH));
        fields.insert("pierce".to_owned(), json!(true));
    }
    match session.send_value("DOM.describeNode", params).await {
        Ok(mut described) => described
            .get_mut("node")
            .map(Value::take)
            .map(Some)
            .ok_or_else(|| ProbeFailure::Broken("DOM.describeNode returned no node".to_owned())),
        Err(CoreError::Cdp(CdpError::Protocol { message, .. })) if message.contains(NODE_GONE) => {
            Ok(None)
        }
        Err(error) => Err(ProbeFailure::from_core(error)),
    }
}

/// A described node's backend id. Chromium always sends one, and a root without it could not be
/// watched, so a node without one fails the scan closed.
fn backend_id(node: &Value) -> Result<BackendNodeId, ProbeFailure> {
    node.get("backendNodeId")
        .and_then(Value::as_i64)
        .map(BackendNodeId)
        .ok_or_else(|| {
            ProbeFailure::Broken("DOM.describeNode sent a node without its id".to_owned())
        })
}

/// `node[key]` as a list, empty when absent.
fn array<'a>(node: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    node.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::{Value, json};

    use std::collections::HashSet;

    use super::{HiddenRoots, Walk, keep_found, unwatched};
    use crate::world::BackendNodeId;

    type TestResult = Result<(), Box<dyn Error>>;

    /// A described node as `DOM.describeNode { pierce }` sends one: `children` only when the
    /// depth reached them.
    fn node(id: i64, name: &str, children: Option<Vec<Value>>) -> Value {
        let mut node = json!({ "backendNodeId": id, "nodeName": name });
        if let Some(children) = children {
            node["childNodeCount"] = json!(children.len());
            node["children"] = Value::Array(children);
        }
        node
    }

    fn root(id: i64, kind: &str, children: Vec<Value>) -> Value {
        let mut root = node(id, "#document-fragment", Some(children));
        root["shadowRootType"] = json!(kind);
        root
    }

    fn host(id: i64, roots: Vec<Value>, children: Vec<Value>) -> Value {
        let mut host = node(id, "DIV", Some(children));
        host["shadowRoots"] = Value::Array(roots);
        host
    }

    /// A node the depth cut off: it has children, and came without them.
    fn cut(id: i64) -> Value {
        json!({ "backendNodeId": id, "nodeName": "DIV", "childNodeCount": 2 })
    }

    fn walk(top: &Value) -> Result<Walk, Box<dyn Error>> {
        let mut walk = Walk::default();
        walk.subtree(top, false)?;
        Ok(walk)
    }

    #[test]
    fn tests_that_closed_roots_and_the_roots_inside_them_are_hidden() -> TestResult {
        let document = node(
            1,
            "#document",
            Some(vec![
                host(
                    2,
                    vec![root(
                        3,
                        "open",
                        vec![host(4, vec![root(5, "closed", vec![])], vec![])],
                    )],
                    vec![],
                ),
                host(
                    6,
                    vec![root(
                        7,
                        "closed",
                        vec![host(8, vec![root(9, "open", vec![])], vec![])],
                    )],
                    vec![],
                ),
            ]),
        );
        let mut roots = walk(&document)?.roots;
        roots.sort_by_key(|root| root.0);
        assert_eq!(
            roots,
            [BackendNodeId(5), BackendNodeId(7), BackendNodeId(9)],
            "the seal reaches open root 3 itself, but neither a closed root nor an open one inside it"
        );
        Ok(())
    }

    #[test]
    fn tests_that_user_agent_roots_and_child_frame_documents_are_never_entered() -> TestResult {
        let mut input = host(
            2,
            vec![root(
                3,
                "user-agent",
                vec![host(4, vec![root(5, "closed", vec![])], vec![])],
            )],
            vec![],
        );
        input["nodeName"] = json!("INPUT");
        let mut frame = node(6, "IFRAME", Some(vec![]));
        frame["contentDocument"] = node(
            7,
            "#document",
            Some(vec![host(8, vec![root(9, "closed", vec![])], vec![])]),
        );
        let walked = walk(&node(1, "#document", Some(vec![input, frame])))?;
        assert!(
            walked.roots.is_empty(),
            "an input's internals hold its raw value, and a child frame's document has its own scan"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_cut_off_node_is_described_again_with_its_roots() -> TestResult {
        let mut cut_host = cut(4);
        cut_host["shadowRoots"] =
            json!([{ "backendNodeId": 5, "shadowRootType": "closed", "childNodeCount": 1 }]);
        let mut bare_host = node(6, "DIV", None);
        bare_host["shadowRoots"] =
            json!([{ "backendNodeId": 7, "shadowRootType": "closed", "childNodeCount": 1 }]);
        let document = node(
            1,
            "#document",
            Some(vec![host(
                2,
                vec![root(3, "closed", vec![cut(10)])],
                vec![cut_host, bare_host],
            )]),
        );
        let walked = walk(&document)?;
        let cut: Vec<(i64, bool)> = walked
            .cut
            .iter()
            .map(|cut| (cut.node.0, cut.under_closed))
            .collect();
        assert!(
            cut.contains(&(10, true)),
            "a node cut off inside the closed root keeps that: {cut:?}"
        );
        assert!(
            cut.contains(&(4, false)),
            "a cut-off host is described again whole: {cut:?}"
        );
        assert!(
            cut.contains(&(7, true)),
            "a root whose children the depth left out: {cut:?}"
        );
        assert_eq!(
            cut.len(),
            3,
            "the cut-off host's own root comes back with it: {cut:?}"
        );
        let mut roots = walked.roots;
        roots.sort_by_key(|root| root.0);
        assert_eq!(
            roots,
            [BackendNodeId(3), BackendNodeId(7)],
            "root 5 is found when its host is described again, not twice"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_node_without_its_id_fails_the_scan_closed() {
        let mut nameless = root(3, "closed", vec![]);
        nameless["backendNodeId"] = Value::Null;
        let document = node(1, "#document", Some(vec![host(2, vec![nameless], vec![])]));
        assert!(
            walk(&document).is_err(),
            "a hidden root Neo cannot name cannot be watched, so its frame is never swept"
        );
    }

    #[test]
    fn tests_that_a_watched_root_is_not_watched_again() {
        let mut watched = HashSet::new();
        let found = HiddenRoots(vec![BackendNodeId(5), BackendNodeId(7), BackendNodeId(5)]);
        assert_eq!(
            unwatched(&watched, &found),
            [BackendNodeId(5), BackendNodeId(7)]
        );
        watched.insert(BackendNodeId(5));
        assert_eq!(
            unwatched(&watched, &found),
            [BackendNodeId(7)],
            "a backend id names one node for good, so a watched root stays watched"
        );
    }

    #[test]
    fn tests_that_a_root_the_page_removed_is_forgotten_and_watched_again_when_it_returns() {
        let mut watched = HashSet::from([BackendNodeId(5), BackendNodeId(7)]);
        keep_found(&mut watched, &HiddenRoots(vec![BackendNodeId(7)]));
        assert_eq!(
            watched,
            HashSet::from([BackendNodeId(7)]),
            "root 5 left the document, so Neo keeps no record of it"
        );
        assert_eq!(
            unwatched(
                &watched,
                &HiddenRoots(vec![BackendNodeId(5), BackendNodeId(7)])
            ),
            [BackendNodeId(5)],
            "attached again, it is handed to the seal again"
        );
    }
}
