//! The look at one box (design section 5.3), shared by the host's check call and by the entry,
//! which looks at every box before its first character and again right before each box.
//!
//! ```text
//!  ref ─ a locator `aria-ref=<ref>` ─ Resolver: jump to the ref's frame document
//!      ─ resolve { want: element } ─ pin            none, detached: element_detached
//!      ─ look { pin, ask } in that world ─ facts and the first refusal
//!  tab address (main frame's `describe`) ─ page_changed, after the page side's origin check
//! ```
//!
//! The look never logs a resolution or a preview: a preview prints every attribute, `value`
//! included, so a page that mirrors what is typed into an attribute would put it in the call log.

use std::pin::pin;
use std::time::Duration;

use browseros_core::{FrameId, TabId};
use futures_util::future::{self, Either};
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::action::frame_projection::{self, Asker, Projection};
use crate::action::protocol_scroll;
use crate::action::requested_navigations::RequestedNavigations;
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::private_entry::{
    BoxAsk, BoxFacts, CoverIn, EntryCode, EntryError, LookField, LookRefusal, LookRefused, Looked,
    Sending, Unread,
};
use crate::probe::{
    ParsedSelector, Point, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget, Read, Want,
};
use crate::resolve::{self, Resolved, Resolver};
use crate::target::{AriaRefJump, LocatorTarget, Strictness};
use crate::world::{Pin, Place};
use crate::{ErrorCode, Playwright, PlaywrightError, Refusal, TimedOut};

/// A box the look found: where it lives, its pin there, its facts, and the first check it
/// fails now (`None` when it passed every one).
pub(crate) struct LookedBox {
    /// The box's frame world, its parents and their visibility.
    pub(crate) site: Resolved,
    /// The box, pinned in that world for the entry's next calls.
    pub(crate) pin: Pin,
    pub(crate) facts: BoxFacts,
    pub(crate) refusal: Option<EntryCode>,
}

/// What a box looks like now, or why it could not be looked at: its ref names no text input, or
/// the tab, the connection or the deadline failed. Only the first is about the box.
pub(crate) enum Look {
    Found(Box<LookedBox>),
    /// The ref names nothing (`element_detached`) or no text input (`not_an_input`): no facts.
    Unfound(EntryCode),
}

/// Whether the tab is at the address the host named (design section 5.3, step 3). Read once per
/// look or entry, from the main frame's own document: one look is one moment of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabAddress {
    /// The host named no address.
    Unnamed,
    /// The tab is at the host's address.
    Same,
    /// The tab is elsewhere: every box answers `page_changed`.
    Changed,
}

impl TabAddress {
    /// Reads the tab's address when the host named one, and compares it with `wanted`.
    pub(crate) async fn read(
        page: &OpPage<'_>,
        scope: &OpScope<'_>,
        wanted: Option<&str>,
    ) -> Result<Self, EntryCode> {
        let Some(wanted) = wanted else {
            return Ok(Self::Unnamed);
        };
        // A main frame whose document went away under the read has navigated: the tab is no
        // longer at the address the host approved.
        let world = match within(scope, page.world(&page.main_endpoint(), Place::Main)).await? {
            Ok(world) => world,
            Err(ProbeFailure::Gone(_)) => return Ok(Self::Changed),
            Err(failure) => return Err(failure_code(failure)),
        };
        match within(
            scope,
            resolve::ask(scope, page, &world, &ProbeRequest::Describe, None),
        )
        .await?
        {
            Ok(ProbeReply::Described { url, .. }) if url == wanted => Ok(Self::Same),
            Ok(ProbeReply::Described { .. }) | Err(ProbeFailure::Gone(_)) => Ok(Self::Changed),
            Ok(_) => Err(EntryCode::Internal),
            Err(failure) => Err(failure_code(failure)),
        }
    }
}

/// A navigation of the tab's main frame the page requested since the call began. Chromium
/// answers no call into a renderer that is starting a navigation until it commits or fails
/// (`action/requested_navigations.rs`), so a look raced with this answers at once instead of
/// waiting for its deadline: the page is changing under the host's approval.
pub(crate) struct MainNavigation {
    requested: RequestedNavigations,
    frame: FrameId,
}

impl MainNavigation {
    /// Starts watching, before the first call into the page.
    pub(crate) fn watch(pw: &Playwright, page: &OpPage<'_>) -> Self {
        let root = page.root_session();
        let sessions = root.session_id().into_iter().cloned().collect();
        Self {
            requested: RequestedNavigations::watch(pw.browser().cdp_events(), sessions),
            frame: page.main_endpoint().frame,
        }
    }

    /// `call`'s answer, or `None` once the main frame has a navigation pending.
    pub(crate) async fn race<F: Future>(&mut self, call: F) -> Option<F::Output> {
        let pending = pin!(self.requested.pending_in(&self.frame));
        match future::select(pin!(call), pending).await {
            Either::Left((answered, _)) => Some(answered),
            Either::Right(((), _)) => None,
        }
    }
}

/// Looks at one box. `length` is the value's length in UTF-16 code units, absent for a look
/// without a value; `address` is where the tab is against the host's address.
pub(crate) async fn look_at(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    field: &LookField,
    cut: bool,
    length: Option<u32>,
    address: TabAddress,
) -> Result<Look, EntryCode> {
    let Some(target) = ref_target(page.tab(), &field.box_ref) else {
        return Ok(Look::Unfound(EntryCode::ElementDetached));
    };
    let resolver = Resolver::new(page, scope, &target, Strictness::Strict);
    // A world that turns out gone under the call is forgotten and looked up once more, as
    // every op does; a ref's own document gone answers `element_detached` at once.
    for _ in 0..2 {
        let Some(site) = within(scope, resolver.locate())
            .await?
            .map_err(|error| playwright_code(&error))?
        else {
            return Ok(Look::Unfound(EntryCode::ElementDetached));
        };
        let resolve = ProbeRequest::Resolve {
            target: ProbeTarget::Chunk(ParsedSelector::of(target.chunks.last())),
            strictness: Strictness::Strict,
            want: Want::Element,
        };
        let pin = match within(scope, site.call(scope, page, &resolve)).await? {
            Ok(ProbeReply::Element { pin, .. }) => pin,
            Ok(ProbeReply::None { .. } | ProbeReply::Detached) => {
                return Ok(Look::Unfound(EntryCode::ElementDetached));
            }
            Ok(_) => return Err(EntryCode::Internal),
            Err(ProbeFailure::Gone(_)) => {
                if resolver.lost(&site).is_err() {
                    return Ok(Look::Unfound(EntryCode::ElementDetached));
                }
                continue;
            }
            Err(failure) => return Err(failure_code(failure)),
        };
        // Ruling R61: the box into view at once, in its own document and in every page above,
        // before any hit is asked. Main's rule sent the browser's own scroll, which is instant
        // and only when needed. The page side's own `scrollIntoViewIfNeeded` follows the
        // page's `scroll-behavior` and, under `smooth`, animates: its hit test then ran before
        // the box was in view, found nothing at the box's place, and nothing is no cover
        // (measured: a box 3000 pixels down under a sheet over the whole view was entered).
        // After this the page side's scroll finds the box in view and does nothing.
        match within(
            scope,
            protocol_scroll::scroll_if_needed(scope, page, site.world(), pin),
        )
        .await?
        {
            Ok(()) => {}
            Err(ProbeFailure::Gone(_)) => {
                if resolver.lost(&site).is_err() {
                    return Ok(Look::Unfound(EntryCode::ElementDetached));
                }
                continue;
            }
            Err(failure) => return Err(failure_code(failure)),
        }
        let ask = BoxAsk {
            expect: field.expect.clone(),
            length,
            cut,
            keys: field.keys,
            frame_visible: site.frame_visible(),
        };
        let looked = match within(
            scope,
            site.call(scope, page, &ProbeRequest::Look { pin, look: ask }),
        )
        .await?
        {
            Ok(ProbeReply::Value { value }) => decode_looked(value.into_value())?,
            // The pin's element left its document, or the document went, since the resolve.
            Ok(ProbeReply::Detached) | Err(ProbeFailure::Gone(_)) => {
                return Ok(Look::Unfound(EntryCode::ElementDetached));
            }
            Ok(_) => return Err(EntryCode::Internal),
            Err(failure) => return Err(failure_code(failure)),
        };
        let above = match pages_above(scope, page, &site, pin, &looked).await? {
            Above::Clear => None,
            Above::Refused(code) => Some(code),
            Above::Gone => return Ok(Look::Unfound(EntryCode::ElementDetached)),
        };
        return Ok(found(site, pin, looked, above, address));
    }
    Ok(Look::Unfound(EntryCode::ElementDetached))
}

/// What the pages above a box's frame say of its centre (ruling R60).
#[derive(Debug, PartialEq, Eq)]
enum Above {
    /// Nothing above takes the pointer there, or there was nothing to ask: the box is in the
    /// main frame, the page side already refused it, or it has no size and is reached by focus.
    Clear,
    /// `box_covered` from a page above, or `cover_unread`.
    Refused(EntryCode),
    /// A frame on the way down went away since the look.
    Gone,
}

/// Asks every page above the box's frame, from the top document down, what takes the pointer
/// at the box's centre, out-of-process frames included. The page side's look sees only the
/// documents its frame can reach (`frameElement` stops at an origin boundary), so before this a
/// banner in a cross-origin parent over a sign-in frame was not seen, and a host reported a
/// password typed under one. This reuses the click's projection (`frame_projection::project`,
/// upstream's `_checkFrameIsHitTarget`) with the look's rules (`Asker::Look`):
///
/// - the look first brought the box into view at once in every page (ruling R61); one out of
///   view in its own frame is not asked above;
/// - a parent's hit on the frame's owner element, or on what holds it, is clear; a parent that
///   still cannot show the point (it cannot scroll there, or clips the frame away) answers
///   `cover_unread`;
/// - anything else the hit lands on covers, a label or a drawing included: rulings R55 and R56
///   excuse the field's own in its own document only, since Neo cannot tell that an element of
///   one document belongs to a field of another;
/// - a frame that is only shifted is placed by its corner; one whose page changes its size or
///   angle answers `cover_unread`, as does a parent that fails under the asking. Neo never
///   guesses a point and never retries here: the sentence tells the agent what to do.
async fn pages_above(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    site: &Resolved,
    pin: Pin,
    looked: &Looked,
) -> Result<Above, EntryCode> {
    let sized = looked.facts.as_ref().is_some_and(|facts| facts.sized);
    if looked.refusal.is_some() || !sized || site.path().hops().is_empty() {
        return Ok(Above::Clear);
    }
    // Where the look's instant scroll (ruling R61) left the box: its centre in its own frame's
    // view, or none when that centre is outside it. A box out of view in its own frame is not
    // asked above, as before (a payment frame's helper box far off its view): a page above would
    // be asked at a place outside the frame, where a neighbour of the frame would read as a
    // cover. The click's own measure is not used here: it calls a box wholly above and left of
    // the view in view.
    let read = ProbeRequest::Read {
        target: ProbeTarget::Pin(pin),
        strictness: Strictness::Strict,
        frame_visible: site.frame_visible(),
        read: Read::ViewCentre,
    };
    let centre = match within(scope, site.call(scope, page, &read)).await? {
        Ok(ProbeReply::Read { value, .. }) => {
            serde_json::from_value::<Option<Point>>(value.into_value())
                .map_err(|_unreadable| EntryCode::Internal)?
        }
        Ok(ProbeReply::Detached) | Err(ProbeFailure::Gone(_)) => return Ok(Above::Gone),
        Ok(_) => return Err(EntryCode::Internal),
        Err(failure) => return Err(failure_code(failure)),
    };
    let Some(centre) = centre else {
        return Ok(Above::Clear);
    };
    let projected = within(
        scope,
        frame_projection::project(scope, page, site.path(), centre, true, Asker::Look),
    )
    .await?;
    above_of(projected)
}

/// The projection's answer as the look's. A frame gone on the way down is the box gone; the
/// entry's own deadline, a closed tab and a lost browser keep their codes; any other failure
/// under the asking leaves a page above unasked.
fn above_of(projected: Result<Projection, PlaywrightError>) -> Result<Above, EntryCode> {
    Ok(match projected {
        Ok(Projection::At(_)) => Above::Clear,
        Ok(Projection::Covered { .. }) => Above::Refused(EntryCode::BoxCovered(CoverIn::PageAbove)),
        Ok(Projection::Transformed) => Above::Refused(EntryCode::CoverUnread(Unread::SizeOrAngle)),
        Ok(Projection::Unseen) => Above::Refused(EntryCode::CoverUnread(Unread::OutOfView)),
        Ok(Projection::Gone) => Above::Gone,
        Err(error) => match playwright_code(&error) {
            EntryCode::ElementDetached => Above::Gone,
            code @ (EntryCode::RunTimeout
            | EntryCode::TargetClosed
            | EntryCode::BrowserDisconnected) => return Err(code),
            _ => Above::Refused(EntryCode::CoverUnread(Unread::NotAsked)),
        },
    })
}

/// The page side's answer, with the tab's address checked in its place: right after the
/// box's origin (step 3), and the pages above the box's frame last.
fn found(
    site: Resolved,
    pin: Pin,
    looked: Looked,
    above: Option<EntryCode>,
    address: TabAddress,
) -> Look {
    let Looked { facts, refusal } = looked;
    let Some(facts) = facts else {
        return Look::Unfound(EntryCode::NotAnInput);
    };
    let refusal = match refusal {
        Some(LookRefused {
            code: LookRefusal::NotAnInput,
            ..
        }) => return Look::Unfound(EntryCode::NotAnInput),
        Some(LookRefused {
            code: LookRefusal::WrongOrigin,
            ..
        }) => Some(EntryCode::WrongOrigin),
        _ if address == TabAddress::Changed => Some(EntryCode::PageChanged),
        Some(refused) => Some(code_of(&refused)),
        None => above,
    };
    Look::Found(Box::new(LookedBox {
        site,
        pin,
        facts,
        refusal,
    }))
}

fn code_of(refused: &LookRefused) -> EntryCode {
    match refused.code {
        LookRefusal::NotAnInput => EntryCode::NotAnInput,
        LookRefusal::WrongOrigin => EntryCode::WrongOrigin,
        LookRefusal::WrongType => EntryCode::WrongType,
        LookRefusal::BoxChanged => match refused.fact {
            Some(fact) => EntryCode::BoxChanged(fact),
            // The page side always names the fact; a reply without one is a probe bug.
            None => EntryCode::Internal,
        },
        LookRefusal::BoxNotEditable => EntryCode::BoxNotEditable,
        LookRefusal::BoxHidden => EntryCode::BoxHidden,
        LookRefusal::BoxCovered => EntryCode::BoxCovered(CoverIn::OwnDocument),
        LookRefusal::ValueTooLong => EntryCode::ValueTooLong,
    }
}

fn decode_looked(value: Value) -> Result<Looked, EntryCode> {
    serde_json::from_value(value).map_err(|_unreadable| EntryCode::Internal)
}

/// The locator `getByRef(<ref>)` builds, for a ref the host names: a snapshot's `e5` or
/// `f2e4`, or `locator.ref()`'s `n7` or `f2n7`. The crate's one ref grammar decides
/// ([`AriaRefJump::parse`]); anything else names no element.
pub(crate) fn ref_target(tab: TabId, box_ref: &str) -> Option<LocatorTarget> {
    AriaRefJump::parse(box_ref)?;
    let target = json!({
        "page": { "$neo": "page", "tab": tab.0 },
        "chunks": [{ "parts": [{ "name": "aria-ref", "body": box_ref, "source": box_ref }] }],
        "display": format!("getByRef('{box_ref}')"),
    });
    serde_json::from_value(target).ok()
}

/// `future`, unless the look's deadline passes first (`run_timeout`).
pub(crate) async fn within<F: Future>(
    scope: &OpScope<'_>,
    future: F,
) -> Result<F::Output, EntryCode> {
    scope
        .within(future)
        .await
        .map_err(|_expired| EntryCode::RunTimeout)
}

/// A failure of the crate's own machinery, as the entry's code.
pub(crate) fn playwright_code(error: &PlaywrightError) -> EntryCode {
    // The scope's own timeout is the entry's deadline, whatever code the leaf that met it chose
    // (waiting for the tab's input answers `not_actionable`).
    if error.timed_out() == TimedOut::Yes {
        return EntryCode::RunTimeout;
    }
    match error.error_code() {
        Some(ErrorCode::ElementDetached | ErrorCode::InvalidInput) => EntryCode::ElementDetached,
        Some(ErrorCode::TargetClosed) => EntryCode::TargetClosed,
        Some(ErrorCode::BrowserDisconnected) => EntryCode::BrowserDisconnected,
        Some(ErrorCode::WaitTimeout) => EntryCode::RunTimeout,
        _ => EntryCode::Internal,
    }
}

/// The entry's error for a failure of the crate's machinery or of the door's rules: a host
/// policy's refusal (a lock, a busy page, a scope the view refused) passes through with its own
/// owner and code (design section 15, promise 2); anything else maps by [`playwright_code`].
pub(crate) fn error_of(error: &PlaywrightError, sending: Sending) -> EntryError {
    if error.error_code().is_some() {
        return EntryError::of(playwright_code(error), sending);
    }
    let refusal = Refusal {
        owner: error.owner(),
        code: error.code().to_owned(),
        message: error.message().to_owned(),
        host_code: error.host_code().map(str::to_owned),
    };
    EntryError::refused(refusal, sending)
}

/// Whether a failure ends the whole call rather than one box's answer: the tab, the browser
/// connection or the deadline (design section 4.8; ruling R32).
pub(crate) fn ends_the_call(code: EntryCode) -> bool {
    matches!(
        code,
        EntryCode::TargetClosed | EntryCode::BrowserDisconnected | EntryCode::RunTimeout
    )
}

/// A probe call that failed below the page: a lost connection or a closed frame.
pub(crate) fn failure_code(failure: ProbeFailure) -> EntryCode {
    match failure {
        ProbeFailure::Gone(_) => EntryCode::TargetClosed,
        ProbeFailure::Cdp(error) => match ErrorCode::from_core(&error) {
            ErrorCode::TargetClosed => EntryCode::TargetClosed,
            ErrorCode::BrowserDisconnected => EntryCode::BrowserDisconnected,
            _ => EntryCode::Internal,
        },
        ProbeFailure::Broken(_) => EntryCode::Internal,
    }
}

/// The deadline as the scope's own timeout: at least a millisecond, since a scope without a
/// timeout would wait for ever.
pub(crate) fn timeout_until(deadline: Instant) -> Duration {
    deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(1))
}

#[cfg(test)]
mod tests {
    use browseros_core::TabId;
    use serde_json::json;

    use super::{Above, CoverIn, EntryCode, Unread, above_of, ref_target};
    use crate::action::frame_projection::Projection;
    use crate::probe::{PageText, Point};
    use crate::{ErrorCode, PlaywrightError};

    /// Ruling R60's table: what each answer of the pages above the box's frame refuses with.
    /// Row (a), a page above that fails under the asking, has no live way to make it fail.
    #[test]
    fn tests_that_the_pages_above_answer_covered_unread_or_the_entrys_own_code()
    -> Result<(), serde_json::Error> {
        let banner: PageText = serde_json::from_value(json!("<div class=\"banner\">"))?;
        let failed = |code| Err(PlaywrightError::new(code, "frameBox: failed".to_owned()));
        let point = Point { x: 140.0, y: 147.0 };
        assert_eq!(above_of(Ok(Projection::At(point))), Ok(Above::Clear));
        assert_eq!(
            above_of(Ok(Projection::Covered { by: banner })),
            Ok(Above::Refused(EntryCode::BoxCovered(CoverIn::PageAbove)))
        );
        assert_eq!(
            above_of(Ok(Projection::Transformed)),
            Ok(Above::Refused(EntryCode::CoverUnread(Unread::SizeOrAngle)))
        );
        assert_eq!(
            above_of(Ok(Projection::Unseen)),
            Ok(Above::Refused(EntryCode::CoverUnread(Unread::OutOfView)))
        );
        assert_eq!(above_of(Ok(Projection::Gone)), Ok(Above::Gone));
        assert_eq!(
            above_of(failed(ErrorCode::ElementDetached)),
            Ok(Above::Gone)
        );
        for unasked in [ErrorCode::Internal, ErrorCode::NotActionable] {
            let wanted = if unasked == ErrorCode::NotActionable {
                // A leaf that met its wait answers the entry's own deadline.
                Err(EntryCode::RunTimeout)
            } else {
                Ok(Above::Refused(EntryCode::CoverUnread(Unread::NotAsked)))
            };
            assert_eq!(above_of(failed(unasked)), wanted, "{unasked:?}");
        }
        assert_eq!(
            above_of(failed(ErrorCode::TargetClosed)),
            Err(EntryCode::TargetClosed)
        );
        assert_eq!(
            above_of(failed(ErrorCode::BrowserDisconnected)),
            Err(EntryCode::BrowserDisconnected)
        );
        Ok(())
    }

    #[test]
    fn tests_that_only_a_printed_or_minted_ref_names_a_box() {
        for good in ["e5", "f2e4", "n7", "f2n3", "f12n30"] {
            assert!(ref_target(TabId(7), good).is_some(), "{good}");
        }
        for bad in [
            "",
            "e",
            "e0",
            "e05",
            "f0e5",
            "f2",
            "fe4",
            "x5",
            "e5 ",
            "#4",
            "f2e4e",
            "e-1",
            "aria-ref=e5",
        ] {
            assert!(ref_target(TabId(7), bad).is_none(), "{bad}");
        }
    }
}
