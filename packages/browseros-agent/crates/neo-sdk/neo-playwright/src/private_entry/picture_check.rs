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
//!            reading? and, at a document's first reading only, may the camera walk it?
//!          ─ that one walk, when offered and the document is small enough: a DOM capture of the
//!            frame's session, shared by the frames of one check (CameraCaptures), the elements
//!            inside its closed shadow roots resolved into the world, and `privateShown` again
//!            with them (`walk`), so its answer sees the boxes inside those roots
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
//! A box inside a closed shadow root is out of reach of every script, so the camera walks a
//! document's closed roots itself, once, at the first reading in that document: the page side
//! offers the walk in that answer only (`walkable`), whether Rust takes it or not. Three rules
//! keep the walk from costing the camera a picture main stores (the design keeper's conditions):
//! it is offered once, so a walk that found nothing, failed or was skipped is not tried again at
//! each reading; a document larger than [`WALK_ELEMENTS`] is not walked, as a capture once sent
//! cannot be called back and the frame's next question would queue behind it; and anything short
//! of a full second answer within [`WALK_BUDGET`] leaves the first answer as it was, so the walk
//! never turns a clear reading into an unknown one. A closed root the page attaches after its
//! document's walk is seen once a marked snapshot opens it, a limit as on main.
//!
//! A tab that took the host's private fill never reaches this question: the entry's tab-life
//! set withholds it first. So what decides here is what the page itself declares private and a
//! script typed, or the owner did. A frame that cannot be read answers `Unknown`, which the camera
//! treats as private: a picture is never stored on a guess.

use std::fmt::Write as _;
use std::time::Duration;

use browseros_core::{FrameId, TabId};
use futures_util::future::join_all;
use serde_json::Value;
use tokio::time::{self, Instant};

use crate::Playwright;
use crate::object_group::ObjectGroup;
use crate::op_scope::AdmittedPage;
use crate::page::PageState;
use crate::page::frame_tree::{self, FrameDocument};
use crate::probe::{self, ProbeFailure, ProbeReply};
use crate::snapshot::{CameraCaptures, closed_root_seeds};
use crate::world::{Endpoint, Installer, IsolatedWorld, Place};

/// How long one frame may take to answer, its first install included. A frame that stalls is
/// `Unknown`, so the picture is withheld rather than waited for.
const FRAME_STALL: Duration = Duration::from_secs(2);

/// How long the camera's walk of one document's closed shadow roots may take, its DOM capture,
/// the seeds' resolves and its second question together. A walk not done by then is dropped and
/// the frame's first answer stands. Well inside [`FRAME_STALL`], beside a capture that measured
/// 85 ms on a 15,000-node page (`clickable_targets`' `MARK_BUDGET`).
const WALK_BUDGET: Duration = Duration::from_millis(500);

/// What a walk leaves of a frame's [`FRAME_STALL`] at least, so the walk never runs into it and
/// turns the frame's answer into a stall.
const WALK_RESERVE: Duration = Duration::from_millis(200);

/// The largest document the camera walks, in elements outside shadow roots. A capture once sent
/// cannot be called back: Chromium finishes it, and the frame's next question queues behind it in
/// the page, so a document whose capture could outlast [`WALK_BUDGET`] is not walked, and its
/// closed roots stay a limit as on main. A capture took 950 ms on a 75,000-element page.
const WALK_ELEMENTS: u64 = 20_000;

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
        // the sum, and the camera serves one job at a time. The frames of one session share one
        // DOM capture for their walks.
        let captures = CameraCaptures::new();
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
            let walk = Walk {
                captures: &captures,
                shares_parent_session: tree.shares_parent_session(frame),
            };
            asks.push(async move {
                let deadline = Instant::now() + FRAME_STALL;
                let asked = self.ask_frame(state, &endpoint, place, walk, deadline);
                let answer = time::timeout_at(deadline, asked)
                    .await
                    .unwrap_or(Asked::Failed);
                (endpoint.frame, answer)
            });
        }
        match listed(join_all(asks).await) {
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
    /// An answer that offers its document's walk is walked before it stands ([`Self::walked`]).
    async fn ask_frame(
        &self,
        state: &PageState,
        endpoint: &Endpoint,
        place: Place,
        walk: Walk<'_>,
        deadline: Instant,
    ) -> Asked {
        if self
            .media()
            .ensure(&endpoint.session, state.media())
            .await
            .is_err()
        {
            return Asked::Failed;
        }
        for attempt in 0..2 {
            let world = match state
                .frames()
                .world(self.installer(), endpoint, place.clone())
                .await
            {
                Ok(world) => world,
                Err(ProbeFailure::Gone(_)) => return Asked::Gone,
                Err(_failed) => return Asked::Failed,
            };
            match probe::picture_question(&world, None).await {
                Ok(reply) => return Asked::Answered(walked(&world, reply, walk, deadline).await),
                Err(ProbeFailure::Gone(_)) if attempt == 0 => state.frames().invalidate(&world),
                Err(ProbeFailure::Gone(_)) => return Asked::Gone,
                Err(_failed) => return Asked::Failed,
            }
        }
        Asked::Gone
    }

    fn installer(&self) -> Installer<'_> {
        Installer {
            browser: self.browser(),
            epoch: self.epoch(),
        }
    }
}

/// What one frame's ask needs for its document's walk: the check's shared captures, and where the
/// frame's document sits in its session's capture.
#[derive(Clone, Copy)]
struct Walk<'a> {
    captures: &'a CameraCaptures,
    shares_parent_session: bool,
}

/// `first`, or, when it offers its document's one walk (`walkable`) and the document holds at most
/// [`WALK_ELEMENTS`] elements, the answer to the same question asked with the elements inside the
/// document's closed shadow roots, so it sees the boxes there. Everything short of that second
/// answer within [`WALK_BUDGET`], and before `deadline` less [`WALK_RESERVE`], leaves `first`: a
/// document too large, no closed root found, a capture or resolve that fails or runs out of time,
/// a second question that fails. The walk is offered once, so none of those is tried again.
///
/// The capture, the resolves and the second question run outside any op, as the question itself
/// does; the resolved nodes live in an object group of the walk's own, released when it ends. The
/// world keeps the roots for this question alone and never opens them to its engine.
async fn walked(
    world: &IsolatedWorld,
    first: ProbeReply,
    walk: Walk<'_>,
    deadline: Instant,
) -> ProbeReply {
    let ProbeReply::Value { value } = &first else {
        return first;
    };
    let offered = value.as_value().get("walkable").and_then(Value::as_bool) == Some(true);
    let elements = value.as_value().get("elements").and_then(Value::as_u64);
    if !offered || elements.is_none_or(|elements| elements > WALK_ELEMENTS) {
        return first;
    }
    let ends = (Instant::now() + WALK_BUDGET).min(deadline - WALK_RESERVE);
    let group = ObjectGroup::new();
    let name = group.on(world.session());
    let Ok(seeds) =
        closed_root_seeds(walk.captures, world, walk.shares_parent_session, name, ends).await
    else {
        return first;
    };
    if seeds.is_empty() {
        return first;
    }
    match time::timeout_at(ends, probe::picture_question(world, Some(&seeds))).await {
        Ok(Ok(second)) => second,
        Ok(Err(_)) | Err(_) => first,
    }
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
