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
//!            reading?
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
//! A tab that took the host's private fill never reaches this question: the entry's tab-life
//! set withholds it first. So what decides here is what the page itself declares private and a
//! script typed, or the owner did. A frame that cannot be read answers `Unknown`, which the camera
//! treats as private: a picture is never stored on a guess.

use std::fmt::Write as _;
use std::time::Duration;

use browseros_core::{FrameId, TabId};
use futures_util::future::join_all;
use serde_json::Value;
use tokio::time;

use crate::Playwright;
use crate::op_scope::AdmittedPage;
use crate::page::PageState;
use crate::page::frame_tree::{self, FrameDocument};
use crate::probe::{self, ProbeFailure, ProbeReply};
use crate::world::{Endpoint, Installer, Place};

/// How long one frame may take to answer, its first install included. A frame that stalls is
/// `Unknown`, so the picture is withheld rather than waited for.
const FRAME_STALL: Duration = Duration::from_secs(2);

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
                let answer = time::timeout(FRAME_STALL, self.ask_frame(state, &endpoint, place))
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
    async fn ask_frame(&self, state: &PageState, endpoint: &Endpoint, place: Place) -> Asked {
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
            match probe::picture_question(&world).await {
                Ok(reply) => return Asked::Answered(reply),
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
