//! Whether Neo may store a picture of a tab now: no frame of it holds a box that the mask treats
//! as private, that is not empty, and that the browser paints in clear (protected-fill ruling
//! R45; design section 10.3).
//!
//! ```text
//!  camera ─ picture_check(tab) ─ frame_tree::read_every: the main frame and every child,
//!            cross-site frames on their own sessions, a frame that cannot be read failing it
//!          ─ every frame asked at once, each within FRAME_STALL
//!          ─ each frame's isolated world (installed on first use) ─ `privateShown`: shown?
//!            does it hold a private box at all? how many inputs did one take since the first
//!            reading? does a host wait, which may carry a closed root the world does not know?
//!          ─ a frame with hosts waiting: its walk on demand ([`walked`]): the hosts handed over
//!            (`privateHosts`), each one's closed root named by DOM.describeNode and resolved
//!            into the world, and `privateShown` again with those roots (`walk`), so its answer
//!            sees the boxes inside them
//!          ─ Clear { reading } when no frame shows one, else Private or Unknown
//!  camera ─ captures ─ asks again ─ stores the picture only if both asks were Clear with the
//!            same reading
//! ```
//!
//! The reading lists every document that holds a private box, empty or not, or whose private
//! box took input, and is compared whole (ruling R54): a payment frame with an empty card box
//! that came, went or moved to another document between the two looks drops the picture, and a
//! value typed and gone again inside one document moves its count. A document with no private
//! box is not in it, so an ad frame's churn changes nothing. A frame that is gone when asked
//! (its world's install fails `Gone`, or the question does again after one fresh install) is
//! left out of the reading; a frame that stalls or fails any other way is `Unknown`.
//!
//! The limit, named: a private value that appears and disappears inside one capture window, in a
//! document that held no private box at the first look and is gone by the second, can be in a
//! stored picture.
//!
//! A box inside a closed shadow root is out of reach of every script, so the camera reaches such
//! a root from its host, on demand (ruling R65). Typing that reaches the root arrives at the
//! world's listener as a trusted input or beforeinput event whose first target is the host, which
//! takes no input itself; a box that holds the focus makes its host the document's active
//! element. The page side counts such an event as input at once and keeps the host until the
//! camera asks for it. A key is not listened for: one that types nothing fires no input event and
//! never moves a reading. No DOM capture: a host costs one description and one resolve, about 2
//! ms for a chain of two roots on a page of 75,000 elements, whose capture would take 930 ms.
//!
//! What a failed read costs depends on what pointed at the host (the design keeper's rulings). A
//! host the focus pointed at is asked once, and a read that fails or runs out of time leaves the
//! answer as main gives it, so for such a host the walk never turns a clear reading into an
//! unknown one. A host that typing reached is certain to hold a box someone typed into: while it
//! is unread the reading is unknown, and it is handed over again at each reading, five at most,
//! after which it reads as on main. A root nothing typed into and whose box never held the focus
//! stays unknown until a marked snapshot opens it to the world, a named limit.
//!
//! A tab that took the host's private fill never reaches this question: the entry's tab-life
//! set withholds it first. So what decides here is what the page itself declares private and a
//! script typed, or the owner did. A frame that cannot be read answers `Unknown`, which the camera
//! treats as private: a picture is never stored on a guess.

use std::fmt::Write as _;
use std::time::Duration;

use browseros_core::{FrameId, TabId};
use futures_util::future::join_all;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::{self, Instant};

use crate::Playwright;
use crate::object_group::ObjectGroup;
use crate::op_scope::AdmittedPage;
use crate::page::PageState;
use crate::page::frame_tree::{self, FrameDocument};
use crate::probe::{self, ProbeFailure, ProbeReply};
use crate::world::{BackendNodeId, Endpoint, Installer, IsolatedWorld, Place};

/// How long one frame may take to answer, its first install included. A frame that stalls is
/// `Unknown`, so the picture is withheld rather than waited for.
const FRAME_STALL: Duration = Duration::from_secs(2);

/// How long one frame's walk on demand may take at one reading: every round's hand-over,
/// descriptions, resolves and second question together. A chain of two roots measured about 2 ms
/// on a page of 75,000 elements; the bound is for a renderer that answers late. A walk not done by
/// then is dropped, and the frame's first answer stands.
const WALK_BUDGET: Duration = Duration::from_millis(500);

/// What a walk leaves of a frame's [`FRAME_STALL`] at least, so the walk never runs into it and
/// turns the frame's answer into a stall.
const WALK_RESERVE: Duration = Duration::from_millis(200);

/// How many rounds one frame's walk takes at one reading. A root found in one round may hold the
/// focus on a nested host, which the second answer points at, and the next round reads its root;
/// while hosts wait the walk goes on. A chain deeper than this goes on at the next reading, whose
/// answer still names its host.
const WALK_ROUNDS: usize = 4;

/// How many hosts one round takes. The page hands over first the hosts a trusted input event
/// named (real closed hosts), then the one the focus pointed at.
const HOSTS_PER_ROUND: u32 = 16;

/// One frame's answer to the camera's question.
enum Asked {
    Answered(ProbeReply),
    /// The frame left, or its document went away twice: not in this reading.
    Gone,
    /// A stall or any other failure: the reading is unknown.
    Failed,
}

/// What a tab shows now, for Neo's camera.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PictureCheck {
    /// No frame holds a private box painted in clear. `reading` names each document that holds
    /// a private box or whose private box took input (its frame and loader) with that input
    /// count, so a capture taken between two checks is kept only if none of that changed.
    Clear {
        /// Those documents and their input counts, in a stable order.
        reading: String,
    },
    /// Some frame holds a private box that is not empty and is painted in clear.
    Private,
    /// The tab or one of its frames could not be read: treated as private.
    Unknown,
}

impl Playwright {
    /// Whether Neo may store a picture of `tab` now (ruling R45): every frame's isolated world
    /// is asked whether it holds a non-empty private box painted in clear. No op scope: Neo's
    /// camera asks it before and after each capture.
    pub async fn picture_check(&self, tab: TabId) -> PictureCheck {
        let browser = self.browser();
        let Some(info) = browser.pages.get_info_by_tab(tab).await else {
            return PictureCheck::Unknown;
        };
        // A tab no op touched yet gets its state here, as an op's admission would build it.
        let known = self.pages().get(tab);
        let state = if let Some(state) = known {
            state
        } else {
            let Ok(session) = browser.pages.get_session(info.page_id.clone()).await else {
                return PictureCheck::Unknown;
            };
            let admitted = AdmittedPage {
                tab,
                core_page: info.page_id.clone(),
                session,
            };
            self.pages().page(browser, &admitted).await
        };
        let Some(root) = browser
            .pages
            .get_attached_session(info.page_id.clone())
            .await
        else {
            return PictureCheck::Unknown;
        };
        // A read that fails is tried once more: a frame can be between two documents for a
        // moment. A read that stalls is Unknown at once, as a frame that stalls is: a renderer
        // that does not answer within FRAME_STALL will not answer a second read sooner.
        let read = || {
            time::timeout(
                FRAME_STALL,
                frame_tree::read_every(browser, &info.page_id, &root),
            )
        };
        let tree = match read().await {
            Ok(Ok(tree)) => tree,
            Ok(Err(_failed)) => {
                let Ok(Ok(tree)) = read().await else {
                    return PictureCheck::Unknown;
                };
                tree
            }
            Err(_stalled) => return PictureCheck::Unknown,
        };
        let main = FrameId(info.target_id.as_str().to_owned());
        let documents = tree.documents(&main);
        if !documents.iter().any(|document| document.frame == main) {
            return PictureCheck::Unknown;
        }
        // Every frame at once, each bounded on its own: one stalled frame costs FRAME_STALL, not
        // the sum, and the camera serves one job at a time.
        let mut asks = Vec::new();
        for frame in tree.ids() {
            let Some(session) = tree.endpoint(frame) else {
                return PictureCheck::Unknown;
            };
            let endpoint = Endpoint {
                session: session.clone(),
                frame: frame.clone(),
            };
            let place = if *frame == main {
                Place::Main
            } else {
                Place::Descendant
            };
            let state = &state;
            asks.push(async move {
                let deadline = Instant::now() + FRAME_STALL;
                let asked = self.ask_frame(state, &endpoint, place);
                let (asked, world) = time::timeout_at(deadline, asked)
                    .await
                    .unwrap_or((Asked::Failed, None));
                let asked = match (asked, world) {
                    (Asked::Answered(reply), Some(world)) if waiting(&reply) => {
                        let ends = (Instant::now() + WALK_BUDGET).min(deadline - WALK_RESERVE);
                        walked(&world, reply, ends).await
                    }
                    (asked, _) => asked,
                };
                (endpoint.frame, asked)
            });
        }
        let answers = join_all(asks).await;
        match listed(answers) {
            Ok(listed) => PictureCheck::Clear {
                reading: reading(&documents, &listed),
            },
            Err(verdict) => verdict,
        }
    }

    /// One frame's answer to the camera's question, its world installed on first use. A frame
    /// discovered before its attachment replayed gets its page preferences first, as every op's
    /// world does (`OpPage::world`).
    ///
    /// A world whose document went away (`Gone`) is dropped and installed afresh, and the frame
    /// asked once more, as an op does on resolution. Without it a child frame that moved to
    /// another document with no op touching it (an ad frame, a payment frame after its step)
    /// would answer `Gone` at every later reading, and its tab would never be pictured again.
    /// A frame still gone after that, or whose install fails `Gone`, has left: `Asked::Gone`.
    /// An answer comes with the world that gave it, for the frame's walk on demand ([`walked`]).
    async fn ask_frame(
        &self,
        state: &PageState,
        endpoint: &Endpoint,
        place: Place,
    ) -> (Asked, Option<IsolatedWorld>) {
        if self
            .media()
            .ensure(&endpoint.session, state.media())
            .await
            .is_err()
        {
            return (Asked::Failed, None);
        }
        for attempt in 0..2 {
            let world = match state
                .frames()
                .world(self.installer(), endpoint, place.clone())
                .await
            {
                Ok(world) => world,
                Err(ProbeFailure::Gone(_)) => return (Asked::Gone, None),
                Err(_failed) => return (Asked::Failed, None),
            };
            match probe::picture_question(&world, None).await {
                Ok(reply) => return (Asked::Answered(reply), Some(world)),
                Err(ProbeFailure::Gone(_)) if attempt == 0 => state.frames().invalidate(&world),
                Err(ProbeFailure::Gone(_)) => return (Asked::Gone, None),
                Err(_failed) => return (Asked::Failed, None),
            }
        }
        (Asked::Gone, None)
    }

    fn installer(&self) -> Installer<'_> {
        Installer {
            browser: self.browser(),
            epoch: self.epoch(),
        }
    }
}

/// Whether `reply` names hosts waiting for the camera's walk on demand (`hosts`).
fn waiting(reply: &ProbeReply) -> bool {
    flag(reply, "hosts")
}

/// Whether `reply` names a host waiting that a trusted input event named (`named`): typing
/// reached a closed root there, so the reading is not clear until that host is asked.
fn named(reply: &ProbeReply) -> bool {
    flag(reply, "named")
}

fn flag(reply: &ProbeReply, name: &str) -> bool {
    matches!(reply, ProbeReply::Value { value }
        if value.as_value().get(name).and_then(Value::as_bool) == Some(true))
}

/// The frame's answer once the closed roots of the hosts it has waiting are read (ruling R65):
/// round by round, the hosts handed over, each one's closed root from the browser, and the
/// question again with the roots found, while hosts wait, up to [`WALK_ROUNDS`] and by `ends`.
///
/// The page keeps the rules of who waits (`private-shown.ts`): a host the focus pointed at is
/// asked once, and a failed read leaves it as on main; one that a trusted input event named is
/// handed over again at each reading until its root is kept, five readings at most. While such a
/// host waits unread, whatever stopped the walk (a failed read, no time left, a hand-over that
/// failed, more hosts than the rounds take), the reading is unknown, never clear: typing reached a
/// closed root there and its box is unread (the design keeper's ruling B). Debug lines name the
/// step and never what a host or its description holds.
///
/// It runs outside any op, as the question itself does: the hand-over's remote array and the
/// resolved roots live in an object group of the walk's own, released when it ends on every path,
/// a timeout that drops it included. The world keeps the roots for this question alone and never
/// opens them to its engine.
async fn walked(world: &IsolatedWorld, first: ProbeReply, ends: Instant) -> Asked {
    let group = ObjectGroup::new();
    let name = group.on(world.session());
    let mut unread = named(&first);
    let mut reply = first;
    for _ in 0..WALK_ROUNDS {
        // A round started late would still hand its hosts over before the timeout is asked
        // (tokio polls the future first), spending them with nothing read: they wait instead.
        if Instant::now() >= ends {
            ended(world, "no time left");
            break;
        }
        match time::timeout_at(ends, round(world, name)).await {
            Ok(Ok(Round::Idle)) => break,
            Ok(Ok(Round::Answered(second))) => {
                unread = named(&second);
                let more = waiting(&second);
                reply = second;
                if !more {
                    break;
                }
            }
            Ok(Ok(Round::Unanswered(why)) | Err(why)) => {
                ended(world, why);
                break;
            }
            Err(_late) => {
                ended(world, "timeout");
                break;
            }
        }
    }
    if unread {
        ended(world, "typing reached a closed root that is not read yet");
        return Asked::Failed;
    }
    Asked::Answered(reply)
}

/// One debug line where the walk stops short: the step only.
fn ended(world: &IsolatedWorld, why: &str) {
    tracing::debug!(
        frame = world.frame().as_str(),
        why,
        "the camera's walk of a closed shadow root ended early"
    );
}

/// What one round of the walk came to.
enum Round {
    /// No host waited to be handed over at this reading.
    Idle,
    /// The question asked again with the roots found, perhaps none, answered whole: its flags say
    /// what still waits.
    Answered(ProbeReply),
    /// Hosts were handed over, and the question asked again failed or answered in part.
    Unanswered(&'static str),
}

/// One round of the walk: up to [`HOSTS_PER_ROUND`] of the hosts waiting, each one's closed root,
/// and the question again with the roots found. It is asked even when none came back, so the
/// page's answer says whether a host that input reached is still unread. `Err` when the
/// hand-over itself failed.
async fn round(world: &IsolatedWorld, group: &str) -> Result<Round, &'static str> {
    let hosts = probe::private_hosts(world, group, HOSTS_PER_ROUND)
        .await
        .map_err(|_failed| "the hand-over failed")?;
    if hosts.is_empty() {
        return Ok(Round::Idle);
    }
    let roots: Vec<String> = join_all(
        hosts
            .into_iter()
            .map(|host| closed_root(world, group, host)),
    )
    .await
    .into_iter()
    .flatten()
    .collect();
    Ok(match probe::picture_question(world, Some(&roots)).await {
        Ok(second) if complete(&second) => Round::Answered(second),
        Ok(_) => Round::Unanswered("an answer in part"),
        Err(_failed) => Round::Unanswered("the second question failed"),
    })
}

/// `DOM.describeNode`'s answer: the host and the shadow roots it carries.
#[derive(Deserialize)]
struct Described {
    node: DescribedNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedNode {
    #[serde(default)]
    shadow_roots: Vec<DescribedRoot>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedRoot {
    backend_node_id: BackendNodeId,
    #[serde(default)]
    shadow_root_type: Option<String>,
}

/// The closed shadow root a host's description lists, if any: only the type `closed`, never
/// `user-agent` (a root the browser made itself) nor `open` (which the world reads already).
fn closed_of(host: DescribedNode) -> Option<BackendNodeId> {
    host.shadow_roots
        .into_iter()
        .find(|root| root.shadow_root_type.as_deref() == Some("closed"))
        .map(|root| root.backend_node_id)
}

/// `DOM.resolveNode`'s answer.
#[derive(Deserialize)]
struct Resolved {
    object: ResolvedObject,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolvedObject {
    object_id: String,
}

/// `host`'s closed shadow root as a remote object in `world`, in `group`, or `None`: the host
/// carries none, or a call fails (a debug line says which). The type comes from the browser's own
/// description of the host only: a script, and deep serialization's `mode`, call a root the
/// browser made itself (a video's controls, a `details` element) `closed` too, where the
/// description says `user-agent`. Depth 0 lists the host's shadow roots without its children,
/// which on a container of 20,000 elements measured 4.2 MB and 244 ms at depth 1.
async fn closed_root(world: &IsolatedWorld, group: &str, host: BackendNodeId) -> Option<String> {
    // The step only: an error can carry what the browser answered, and a description the host's
    // attributes, which a component may mirror its box's value into.
    let failed = |step: &str| {
        tracing::debug!(
            frame = world.frame().as_str(),
            step,
            "the camera could not read a host's closed shadow root"
        );
    };
    let described = world
        .session()
        .send::<_, Described>(
            "DOM.describeNode",
            json!({ "backendNodeId": host, "depth": 0 }),
        )
        .await
        .map_err(|_error| failed("describe"))
        .ok()?;
    let root = closed_of(described.node)?;
    let params = json!({
        "backendNodeId": root,
        "executionContextId": world.context(),
        "objectGroup": group,
    });
    let resolved = world
        .session()
        .send::<_, Resolved>("DOM.resolveNode", params)
        .await
        .map_err(|_error| failed("resolve"))
        .ok()?;
    Some(resolved.object.object_id)
}

/// Whether `reply` is a whole answer to the camera's question (`shown`, `inputs` and `boxes`). A
/// probe's error reply arrives as `Ok` too, and taking it would turn the frame's clear first
/// answer into an unknown reading.
fn complete(reply: &ProbeReply) -> bool {
    let ProbeReply::Value { value } = reply else {
        return false;
    };
    let value = value.as_value();
    value.get("shown").is_some_and(Value::is_boolean)
        && value.get("inputs").is_some_and(Value::is_u64)
        && value.get("boxes").is_some_and(Value::is_boolean)
}

/// The documents a reading lists, with their input counts: each frame that answered it holds a
/// private box or one of its private boxes took input. A frame that has gone is not in it. `Err`
/// is the verdict when a frame shows a private value (`Private`) or did not answer (`Unknown`).
fn listed(answers: Vec<(FrameId, Asked)>) -> Result<Vec<(FrameId, u64)>, PictureCheck> {
    let mut listed = Vec::new();
    for (frame, answer) in answers {
        let value = match answer {
            Asked::Answered(ProbeReply::Value { value }) => value.into_value(),
            Asked::Gone => continue,
            Asked::Answered(_) | Asked::Failed => return Err(PictureCheck::Unknown),
        };
        match (
            value.get("shown").and_then(Value::as_bool),
            value.get("inputs").and_then(Value::as_u64),
            value.get("boxes").and_then(Value::as_bool),
        ) {
            (Some(true), _, _) => return Err(PictureCheck::Private),
            (Some(false), Some(count), Some(boxes)) => {
                if boxes || count > 0 {
                    listed.push((frame, count));
                }
            }
            _ => return Err(PictureCheck::Unknown),
        }
    }
    Ok(listed)
}

/// The reading the camera compares: each listed document as `frame=loader#inputs;`, in the
/// tree's order, so two readings of an unchanged tab are equal.
fn reading(documents: &[FrameDocument], listed: &[(FrameId, u64)]) -> String {
    let mut reading = String::new();
    for document in documents {
        let Some(count) = listed
            .iter()
            .find(|(frame, _)| *frame == document.frame)
            .map(|(_, count)| *count)
        else {
            continue;
        };
        let _ = write!(
            reading,
            "{}={}#{};",
            document.frame.as_str(),
            document.loader,
            count
        );
    }
    reading
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_core::FrameId;
    use serde_json::{Value, json};

    use super::{Asked, PictureCheck, listed};
    use crate::probe::ProbeReply;

    fn frame(id: &str) -> FrameId {
        FrameId(id.to_owned())
    }

    fn answered(value: &Value) -> Result<Asked, Box<dyn Error>> {
        let reply = serde_json::from_value(json!({ "kind": "value", "value": value }))?;
        Ok(Asked::Answered(reply))
    }

    /// The walk on demand reads a host's root type from its description only (the snapshot
    /// design's reviewer's runs): a closed root is taken, and a video's or a `details` element's
    /// root, which deep serialization also calls `closed`, is not, nor an open one.
    #[test]
    fn tests_that_only_a_root_the_browser_calls_closed_is_taken() -> Result<(), Box<dyn Error>> {
        use super::{Described, closed_of};
        use crate::world::BackendNodeId;
        let host = |roots: Value| -> Result<_, Box<dyn Error>> {
            let described: Described =
                serde_json::from_value(json!({ "node": { "nodeType": 1, "shadowRoots": roots } }))?;
            Ok(closed_of(described.node))
        };
        let root = |id: i64, kind: &str| json!({ "backendNodeId": id, "shadowRootType": kind });
        assert_eq!(host(json!([root(4, "closed")]))?, Some(BackendNodeId(4)));
        assert_eq!(host(json!([root(6, "user-agent")]))?, None, "a video");
        assert_eq!(host(json!([root(13, "open")]))?, None, "an open root");
        assert_eq!(host(json!([]))?, None, "no root");
        let bare: Described = serde_json::from_value(json!({ "node": { "nodeType": 1 } }))?;
        assert_eq!(closed_of(bare.node), None, "no shadowRoots listed");
        Ok(())
    }

    /// A walk's second reply stands only when it is a whole answer: an error reply, or a value
    /// missing a field, keeps the frame's first answer.
    #[test]
    fn tests_that_only_a_whole_second_answer_replaces_the_first() -> Result<(), Box<dyn Error>> {
        use super::complete;

        let reply = |value: &Value| -> Result<ProbeReply, Box<dyn Error>> {
            Ok(serde_json::from_value(
                json!({ "kind": "value", "value": value }),
            )?)
        };
        let whole = json!({ "shown": false, "inputs": 0, "boxes": true });
        assert!(complete(&reply(&whole)?));
        assert!(!complete(&reply(
            &json!({ "shown": false, "boxes": true })
        )?));
        let error: ProbeReply = serde_json::from_value(
            json!({ "kind": "error", "message": "the probe threw", "class": "script" }),
        )?;
        assert!(!complete(&error), "an error reply arrives as Ok");
        Ok(())
    }

    /// Ruling R54's sorting of the frames' answers: a frame that shows a private value is
    /// Private, one that did not answer is Unknown (never left out as a gone frame is), and only
    /// a document that holds a private box or whose private box took input is listed.
    #[test]
    fn each_frames_answer_lists_it_leaves_it_out_or_decides_the_reading()
    -> Result<(), Box<dyn Error>> {
        let clear =
            |boxes: bool, inputs: u64| json!({ "shown": false, "inputs": inputs, "boxes": boxes });
        let listed_frames = listed(vec![
            (frame("card"), answered(&clear(true, 0))?),
            (frame("typed"), answered(&clear(false, 2))?),
            (frame("ad"), answered(&clear(false, 0))?),
            (frame("left"), Asked::Gone),
        ]);
        assert_eq!(
            listed_frames.ok(),
            Some(vec![(frame("card"), 0), (frame("typed"), 2)]),
            "a private box or a count lists a frame; an ad and a gone frame are not in it"
        );
        let shown = json!({ "shown": true, "inputs": 0, "boxes": true });
        let cases = [
            (
                vec![(frame("x"), Asked::Failed)],
                PictureCheck::Unknown,
                "a frame that failed",
            ),
            (
                vec![(frame("x"), answered(&shown)?)],
                PictureCheck::Private,
                "a value shown",
            ),
            (
                vec![(
                    frame("x"),
                    answered(&json!({ "shown": false, "inputs": 0 }))?,
                )],
                PictureCheck::Unknown,
                "a missing field",
            ),
            (
                vec![(
                    frame("x"),
                    Asked::Answered(serde_json::from_value::<ProbeReply>(
                        json!({ "kind": "detached" }),
                    )?),
                )],
                PictureCheck::Unknown,
                "a reply that is not a value",
            ),
            (
                vec![(frame("left"), Asked::Gone), (frame("x"), Asked::Failed)],
                PictureCheck::Unknown,
                "a gone frame does not excuse one that failed",
            ),
        ];
        for (answers, verdict, case) in cases {
            assert_eq!(listed(answers).err(), Some(verdict), "{case}");
        }
        Ok(())
    }
}
